//! Tabla de números de syscall del kernel.
// COMPILE-STATUS: borrador (sin compilar)
//!
//! Numeración estable inspirada en POSIX. Los números NUNCA se reordenan ni se
//! reutilizan: solo se AÑADEN variantes al final. El ABI de usuario depende de
//! estos valores (§8 del contrato), por lo que cambiarlos rompería binarios ya
//! compilados contra esta interfaz.
#![allow(dead_code)]

/// Números de syscall. Estables; solo se AÑADEN al final. [CANÓNICO]
///
/// El discriminante `#[repr(usize)]` es exactamente el número que viaja por el
/// ABI (registro `a2` en la convención Xtensa descrita en `super`).
#[repr(usize)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Syscall {
    /// Lee de un descriptor: `read(fd, buf_ptr, len) -> bytes`.
    Read = 0,
    /// Escribe en un descriptor: `write(fd, buf_ptr, len) -> bytes`.
    Write = 1,
    /// Abre una ruta: `open(path_ptr, path_len, flags) -> fd`.
    Open = 2,
    /// Cierra un descriptor: `close(fd) -> 0`.
    Close = 3,
    /// Control de dispositivo: `ioctl(fd, cmd, arg)`.
    Ioctl = 4,
    /// Termina la tarea actual: `exit(code)` (no retorna).
    Exit = 5,
    /// Crea una tarea: `spawn(name_ptr, name_len, entry, arg, stack, prio) -> tid`.
    Spawn = 6,
    /// Espera a una tarea hija: `wait(tid)` (reservado).
    Wait = 7,
    // --- Reservados para fases siguientes (mantener el número) ---
    /// Reposiciona el cursor: `seek(fd, offset, whence) -> nueva_pos`.
    Seek = 8,
    /// Crea un directorio: `mkdir(path_ptr, path_len) -> 0`.
    Mkdir = 9,
    /// Elimina una entrada: `unlink(path_ptr, path_len) -> 0`.
    Unlink = 10,
    /// Lista un directorio: `readdir(path_ptr, path_len, buf_ptr, buf_len) -> bytes`.
    Readdir = 11,
    /// Milisegundos desde el arranque: `uptime_ms() -> ms`.
    UptimeMs = 12,
    /// Consulta/ajuste de heap: `sbrk(incr) -> bytes_libres`.
    Sbrk = 13,
    /// Cede la CPU voluntariamente: `yield() -> 0`.
    Yield = 14,
}

impl Syscall {
    /// Convierte el número crudo del ABI a variante. [CANÓNICO]
    ///
    /// Devuelve `None` si el número no corresponde a ninguna syscall conocida;
    /// el despachador traduce ese caso a `-ENOTSUP` (nunca panica).
    pub fn from_usize(n: usize) -> Option<Syscall> {
        let sc = match n {
            0 => Syscall::Read,
            1 => Syscall::Write,
            2 => Syscall::Open,
            3 => Syscall::Close,
            4 => Syscall::Ioctl,
            5 => Syscall::Exit,
            6 => Syscall::Spawn,
            7 => Syscall::Wait,
            8 => Syscall::Seek,
            9 => Syscall::Mkdir,
            10 => Syscall::Unlink,
            11 => Syscall::Readdir,
            12 => Syscall::UptimeMs,
            13 => Syscall::Sbrk,
            14 => Syscall::Yield,
            _ => return None,
        };
        Some(sc)
    }

    /// Número crudo del ABI (equivalente al discriminante `#[repr(usize)]`).
    pub const fn number(self) -> usize {
        self as usize
    }
}
