//! Despachador de syscalls: traduce (número, args) a llamadas de subsistema.
// COMPILE-STATUS: borrador (sin compilar)
//!
//! Recibe el número de syscall y hasta 6 argumentos crudos (`usize`) ya
//! extraídos de los registros por el vector de excepción (véase `super`), y
//! delega en `vfs`, `scheduler`, `mm` o `arch::xtensa::timer` según el contrato.
//!
//! ## Reglas duras
//! - **NUNCA panica.** Todo acceso a `args` es por índice comprobado; toda
//!   reconstrucción de punteros valida `NULL`; los errores externos se traducen
//!   a `KError` y de ahí a errno negativo (`KError::as_errno`).
//! - Retorno `isize`: `>= 0` éxito (bytes, fd, tid, posición…); `< 0` error.
//! - Sin MMU: kernel y "usuario" comparten espacio de direcciones, así que un
//!   puntero del ABI se puede desreferenciar directamente. Aun así se valida que
//!   no sea nulo para atrapar llamadas mal formadas.
#![allow(dead_code)]

use crate::prelude::*;
use crate::vfs::{Fd, InodeKind, OpenFlags, SeekFrom};
use super::table::Syscall;

/// Punto de entrada del despachador de syscalls. [CANÓNICO]
///
/// `num` es el número de syscall; `args` los argumentos crudos (0..=6). Nunca
/// panica: un número desconocido devuelve `-ENOTSUP`.
pub fn dispatch(num: usize, args: &[usize]) -> isize {
    let sc = match Syscall::from_usize(num) {
        Some(s) => s,
        None => return KError::NotSupported.as_errno(),
    };

    match sc {
        Syscall::Read => sys_read(args),
        Syscall::Write => sys_write(args),
        Syscall::Open => sys_open(args),
        Syscall::Close => sys_close(args),
        Syscall::Ioctl => sys_ioctl(args),
        Syscall::Exit => sys_exit(args),
        Syscall::Spawn => sys_spawn(args),
        Syscall::Wait => sys_wait(args),
        Syscall::Seek => sys_seek(args),
        Syscall::Mkdir => sys_mkdir(args),
        Syscall::Unlink => sys_unlink(args),
        Syscall::Readdir => sys_readdir(args),
        Syscall::UptimeMs => sys_uptime_ms(args),
        Syscall::Sbrk => sys_sbrk(args),
        Syscall::Yield => sys_yield(args),
    }
}

// ===========================================================================
// Helpers de ABI (extracción segura de argumentos y reconstrucción de buffers).
// ===========================================================================

/// Lee el argumento `i`; si falta, devuelve 0 (evita indexado que desborde).
#[inline]
fn arg(args: &[usize], i: usize) -> usize {
    match args.get(i) {
        Some(&v) => v,
        None => 0,
    }
}

/// Reconstruye un slice inmutable desde `(ptr, len)` crudos del ABI.
///
/// # Safety
/// El llamador (el vector de excepción) garantiza que, si `len > 0`, la región
/// `[ptr, ptr+len)` pertenece al llamante y es válida durante la syscall. Sin
/// MMU no hay forma de comprobarlo aquí; solo se atrapa el puntero nulo.
unsafe fn user_slice<'a>(ptr: usize, len: usize) -> KResult<&'a [u8]> {
    if len == 0 {
        return Ok(&[]);
    }
    if ptr == 0 {
        return Err(KError::Fault);
    }
    // SAFETY: contrato del ABI; espacio de direcciones compartido (sin MMU).
    Ok(core::slice::from_raw_parts(ptr as *const u8, len))
}

/// Igual que `user_slice`, pero mutable (para buffers de lectura/`readdir`).
///
/// # Safety
/// Ver `user_slice`. Además, la región no debe solaparse con datos vivos del
/// kernel mientras dure la syscall.
unsafe fn user_slice_mut<'a>(ptr: usize, len: usize) -> KResult<&'a mut [u8]> {
    if len == 0 {
        return Ok(&mut []);
    }
    if ptr == 0 {
        return Err(KError::Fault);
    }
    // SAFETY: contrato del ABI; espacio de direcciones compartido (sin MMU).
    Ok(core::slice::from_raw_parts_mut(ptr as *mut u8, len))
}

/// Reconstruye una `&str` UTF-8 desde `(ptr, len)`.
///
/// # Safety
/// Ver `user_slice`.
unsafe fn user_str<'a>(ptr: usize, len: usize) -> KResult<&'a str> {
    let bytes = user_slice(ptr, len)?;
    core::str::from_utf8(bytes).map_err(|_| KError::InvalidArgument)
}

/// Traduce `KResult<usize>` (bytes) al valor de retorno del ABI.
#[inline]
fn ret_usize(r: KResult<usize>) -> isize {
    match r {
        // Saturamos por seguridad: un `usize` no cabe garantizado en `isize`.
        Ok(n) => core::cmp::min(n, isize::MAX as usize) as isize,
        Err(e) => e.as_errno(),
    }
}

/// Traduce `KResult<()>` (éxito sin valor) al retorno del ABI.
#[inline]
fn ret_unit(r: KResult<()>) -> isize {
    match r {
        Ok(()) => 0,
        Err(e) => e.as_errno(),
    }
}

/// Codifica el tipo de inodo para la serialización de `readdir`.
#[inline]
const fn kind_to_u8(kind: InodeKind) -> u8 {
    match kind {
        InodeKind::File => 1,
        InodeKind::Dir => 2,
        InodeKind::Device => 3,
        InodeKind::Symlink => 4,
    }
}

// ===========================================================================
// Implementaciones por syscall.
// ===========================================================================

/// `read(fd, buf_ptr, len) -> bytes` — delega en `vfs::read`.
fn sys_read(args: &[usize]) -> isize {
    let fd = arg(args, 0) as Fd;
    let buf = match unsafe { user_slice_mut(arg(args, 1), arg(args, 2)) } {
        Ok(s) => s,
        Err(e) => return e.as_errno(),
    };
    ret_usize(crate::vfs::read(fd, buf))
}

/// `write(fd, buf_ptr, len) -> bytes` — delega en `vfs::write`.
fn sys_write(args: &[usize]) -> isize {
    let fd = arg(args, 0) as Fd;
    let buf = match unsafe { user_slice(arg(args, 1), arg(args, 2)) } {
        Ok(s) => s,
        Err(e) => return e.as_errno(),
    };
    ret_usize(crate::vfs::write(fd, buf))
}

/// `open(path_ptr, path_len, flags) -> fd` — delega en `vfs::open`.
fn sys_open(args: &[usize]) -> isize {
    let path = match unsafe { user_str(arg(args, 0), arg(args, 1)) } {
        Ok(s) => s,
        Err(e) => return e.as_errno(),
    };
    let flags = OpenFlags(arg(args, 2) as u32);
    match crate::vfs::open(path, flags) {
        Ok(fd) => fd as isize,
        Err(e) => e.as_errno(),
    }
}

/// `close(fd) -> 0` — delega en `vfs::close`.
fn sys_close(args: &[usize]) -> isize {
    let fd = arg(args, 0) as Fd;
    ret_unit(crate::vfs::close(fd))
}

/// `ioctl(fd, cmd, arg)` — el VFS de alto nivel aún no expone ioctl.
///
/// Los `Device` de `/dev` sí soportan `ioctl`, pero enrutarlo requiere la tabla
/// de descriptores (Fase posterior). De momento se rechaza con `-ENOTSUP`.
fn sys_ioctl(_args: &[usize]) -> isize {
    KError::NotSupported.as_errno()
}

/// `exit(code)` — termina la tarea actual; no retorna.
fn sys_exit(args: &[usize]) -> isize {
    let code = arg(args, 0) as i32;
    // `scheduler::exit` diverge (`!`), que coacciona al `isize` de retorno.
    crate::scheduler::exit(code)
}

/// `spawn(name_ptr, name_len, entry, arg, stack, prio) -> tid`.
///
/// `entry` es la dirección cruda de una `fn(usize)`. Sin MMU comparte espacio
/// con el kernel, así que se reinterpreta como puntero a función. Es la parte
/// más delicada del ABI: un valor inválido es comportamiento indefinido al
/// saltar; por eso se rechaza al menos el puntero nulo.
fn sys_spawn(args: &[usize]) -> isize {
    let name = match unsafe { user_str(arg(args, 0), arg(args, 1)) } {
        Ok(s) => s,
        Err(e) => return e.as_errno(),
    };
    let entry_raw = arg(args, 2);
    if entry_raw == 0 {
        return KError::Fault.as_errno();
    }
    // SAFETY: el ABI garantiza que `entry_raw` apunta a una `fn(usize)` válida.
    // `usize` y `fn(usize)` son ambos del tamaño de un puntero en Xtensa LX7.
    let entry: fn(usize) = unsafe { core::mem::transmute::<usize, fn(usize)>(entry_raw) };
    let entry_arg = arg(args, 3);
    let mut stack_size = arg(args, 4);
    if stack_size == 0 {
        stack_size = layout::DEFAULT_STACK_SIZE;
    }
    let priority = arg(args, 5) as u8;

    match crate::scheduler::spawn(name, entry, entry_arg, stack_size, priority) {
        Ok(tid) => tid as isize,
        Err(e) => e.as_errno(),
    }
}

/// `wait(tid)` — el scheduler aún no expone espera de hijos (reservado).
fn sys_wait(_args: &[usize]) -> isize {
    KError::NotSupported.as_errno()
}

/// `seek(fd, offset, whence) -> nueva_pos` — delega en `vfs::seek`.
///
/// `whence`: 0 = `Start`, 1 = `Current`, 2 = `End` (estilo POSIX `SEEK_*`).
fn sys_seek(args: &[usize]) -> isize {
    let fd = arg(args, 0) as Fd;
    let off = arg(args, 1);
    let whence = arg(args, 2);
    // `off` viaja como `usize` crudo. Para `SEEK_CUR`/`SEEK_END` el
    // desplazamiento es CON SIGNO (permite retroceder), así que se reinterpreta
    // primero como `isize` (mismo ancho que el puntero) y luego se extiende con
    // signo a `i64`. Un `off as i64` directo extendería con CEROS en el target
    // de 32 bits, haciendo imposibles los desplazamientos negativos.
    let pos = match whence {
        0 => SeekFrom::Start(off as u64),
        1 => SeekFrom::Current(off as isize as i64),
        2 => SeekFrom::End(off as isize as i64),
        _ => return KError::InvalidArgument.as_errno(),
    };
    match crate::vfs::seek(fd, pos) {
        // La posición cabe en `isize` en la práctica; saturamos por seguridad.
        Ok(n) => core::cmp::min(n, isize::MAX as u64) as isize,
        Err(e) => e.as_errno(),
    }
}

/// `mkdir(path_ptr, path_len) -> 0` — delega en `vfs::mkdir`.
fn sys_mkdir(args: &[usize]) -> isize {
    let path = match unsafe { user_str(arg(args, 0), arg(args, 1)) } {
        Ok(s) => s,
        Err(e) => return e.as_errno(),
    };
    ret_unit(crate::vfs::mkdir(path))
}

/// `unlink(path_ptr, path_len) -> 0` — delega en `vfs::unlink`.
fn sys_unlink(args: &[usize]) -> isize {
    let path = match unsafe { user_str(arg(args, 0), arg(args, 1)) } {
        Ok(s) => s,
        Err(e) => return e.as_errno(),
    };
    ret_unit(crate::vfs::unlink(path))
}

/// `readdir(path_ptr, path_len, buf_ptr, buf_len) -> bytes_escritos`.
///
/// Serializa las entradas en el buffer del llamante mientras quepan, con el
/// formato por registro (little-endian):
/// `[ino:u64][kind:u8][name_len:u16][name:bytes]`.
/// Devuelve el número de bytes escritos; si el buffer se llena, corta en el
/// último registro completo (el llamante puede reintentar con más espacio).
fn sys_readdir(args: &[usize]) -> isize {
    let path = match unsafe { user_str(arg(args, 0), arg(args, 1)) } {
        Ok(s) => s,
        Err(e) => return e.as_errno(),
    };
    let out = match unsafe { user_slice_mut(arg(args, 2), arg(args, 3)) } {
        Ok(s) => s,
        Err(e) => return e.as_errno(),
    };

    let entries = match crate::vfs::readdir(path) {
        Ok(v) => v,
        Err(e) => return e.as_errno(),
    };

    let mut pos: usize = 0;
    for e in entries.iter() {
        let name = e.name.as_bytes();
        let name_len = name.len();
        // Cabecera fija (8 + 1 + 2) + nombre. `checked_add` evita overflow.
        let rec = match name_len.checked_add(8 + 1 + 2) {
            Some(r) => r,
            None => break,
        };
        match pos.checked_add(rec) {
            Some(end) if end <= out.len() => {}
            // No cabe otro registro completo: paramos sin desbordar.
            _ => break,
        }

        // ino (u64 LE).
        out[pos..pos + 8].copy_from_slice(&e.ino.to_le_bytes());
        pos += 8;
        // kind (u8).
        out[pos] = kind_to_u8(e.kind);
        pos += 1;
        // name_len (u16 LE), saturado por si el nombre excede 65535 bytes.
        let nl = core::cmp::min(name_len, u16::MAX as usize) as u16;
        out[pos..pos + 2].copy_from_slice(&nl.to_le_bytes());
        pos += 2;
        // name (bytes).
        out[pos..pos + name_len].copy_from_slice(name);
        pos += name_len;
    }

    pos as isize
}

/// `uptime_ms() -> ms` — delega en `arch::xtensa::timer::uptime_ms`.
fn sys_uptime_ms(_args: &[usize]) -> isize {
    let ms = crate::arch::xtensa::timer::uptime_ms();
    // Saturamos: `u64` de ms no cabe garantizado en `isize` (32-bit en Xtensa).
    core::cmp::min(ms, isize::MAX as u64) as isize
}

/// `sbrk(incr) -> bytes_libres` — variante "consulta de heap" (§ tabla).
///
/// No hay heap de usuario separado en esta fase; se interpreta como una
/// consulta y devuelve los bytes libres actuales del allocator del kernel.
fn sys_sbrk(_args: &[usize]) -> isize {
    let free = crate::mm::stats().free;
    core::cmp::min(free, isize::MAX as usize) as isize
}

/// `yield() -> 0` — cede la CPU a la siguiente tarea lista.
fn sys_yield(_args: &[usize]) -> isize {
    crate::scheduler::yield_now();
    0
}
