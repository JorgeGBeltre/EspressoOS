//! Comandos internos de la shell y su despachador.
// COMPILE-STATUS: borrador (sin compilar)
//!
//! Coreutils mínimos implementados sobre la API del contrato:
//! `echo, help, clear, uptime, free, ps, reboot, ls, cat, mkdir, touch, rm,
//! write`. Los comandos de sistema de archivos usan [`crate::vfs`]; `free`
//! usa [`crate::mm`]; `uptime` usa [`crate::arch::xtensa::timer`]; `ps` usa
//! [`crate::scheduler`].
//!
//! ## Redirección de salida
//! La firma canónica `dispatch(name, args) -> i32` no transporta un destino de
//! salida, así que el enrutado de la salida vive en un sink global de este
//! módulo ([`begin_redirect`]/[`end_redirect`]). La shell lo configura según la
//! redirección del `Command` ANTES de despachar y lo restaura después. Toda la
//! salida "normal" de los comandos pasa por [`emit`], que respeta el sink; los
//! diagnósticos/errores van SIEMPRE a la consola (estilo stderr).
#![allow(dead_code)]

use crate::arch::xtensa::sync::Mutex;
use crate::arch::xtensa::timer;
use crate::drivers::uart;
use crate::mm;
use crate::prelude::*;
use crate::scheduler;
use crate::vfs::{self, DirEntry, InodeKind, OpenFlags};
use alloc::format;

use super::parser::Redirect;

// ===========================================================================
// Enrutado de salida (sink) — respeta redirecciones `>` / `>>`.
// ===========================================================================

/// Destino actual de la salida de los comandos.
#[derive(Clone, Copy)]
enum Sink {
    /// Consola (USB-Serial-JTAG) vía `drivers::uart`.
    Console,
    /// Canal SSH: la salida va al puente de `shell::remote` (sesión remota).
    Ssh,
    /// Archivo abierto en el VFS (descriptor).
    File(vfs::Fd),
}

/// Sink global. Estado compartido protegido con el `Mutex` del contrato
/// (nunca `static mut`). La shell es cooperativa, pero el `Mutex` deja el
/// diseño listo para preempción/SMP.
static OUTPUT: Mutex<Sink> = Mutex::new(Sink::Console);

/// Sink BASE al que se vuelve cuando NO hay redirección `>`/`>>` activa. Lo fija
/// la REPL: `Console` para la shell local, `Ssh` para una sesión SSH. Así una
/// redirección a archivo, al terminar, restaura el destino correcto (consola o
/// canal) en vez de forzar siempre consola.
///
/// LIMITACIÓN (MVP): el sink es GLOBAL, así que mientras una sesión SSH lo pone en
/// `Ssh`, la salida de comandos de la shell LOCAL también iría al canal. Se acepta
/// porque el MVP sirve UNA shell a la vez. Bajo el planificador cooperativo un
/// `dispatch` corre hasta el final sin `yield`, así que el intercalado por swap del
/// sink no rompe una ejecución en curso.
static BASE: Mutex<Sink> = Mutex::new(Sink::Console);

/// Enruta la salida de los comandos al canal SSH (lo llama `remote::run_with_io`
/// al iniciar una sesión remota).
pub fn set_base_ssh() {
    *BASE.lock() = Sink::Ssh;
    *OUTPUT.lock() = Sink::Ssh;
}

/// Restaura la salida de los comandos a la consola (fin de sesión remota).
pub fn set_base_console() {
    *BASE.lock() = Sink::Console;
    *OUTPUT.lock() = Sink::Console;
}

// ===========================================================================
// Directorio de trabajo (CWD) y resolución de rutas relativas.
// El VFS solo acepta rutas ABSOLUTAS; la shell mantiene un CWD y convierte las
// rutas relativas a absolutas antes de llamar al VFS. (MVP: un CWD global.)
// ===========================================================================

/// Directorio de trabajo actual. Vacío = raíz (`/`).
static CWD: Mutex<String> = Mutex::new(String::new());

/// CWD actual como ruta absoluta (siempre empieza por `/`).
pub fn cwd_get() -> String {
    let c = CWD.lock();
    if c.is_empty() {
        String::from("/")
    } else {
        c.clone()
    }
}

/// Normaliza una ruta ABSOLUTA colapsando `.`, `..` y `//`.
fn norm_abs(path: &str) -> String {
    let mut comps: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                comps.pop();
            }
            p => comps.push(p),
        }
    }
    let mut out = String::from("/");
    out.push_str(&comps.join("/"));
    out
}

/// Convierte `path` (relativo o absoluto) a una ruta ABSOLUTA normalizada,
/// resolviéndolo contra el CWD.
fn resolve(path: &str) -> String {
    if path.starts_with('/') {
        norm_abs(path)
    } else {
        let mut base = cwd_get();
        if !base.ends_with('/') {
            base.push('/');
        }
        base.push_str(path);
        norm_abs(&base)
    }
}

/// Escribe bytes crudos en el sink actual (consola, canal SSH o archivo).
fn emit(bytes: &[u8]) {
    let mut sink = OUTPUT.lock();
    match &mut *sink {
        Sink::Console => {
            let _ = uart::write(bytes);
        }
        Sink::Ssh => {
            let _ = crate::shell::remote::command_output_to_ssh(bytes);
        }
        Sink::File(fd) => {
            // La salida a archivo ignora fallos parciales aquí; los comandos
            // que necesitan detectar errores de escritura (p. ej. `write`)
            // usan `vfs::write` directamente.
            let _ = vfs::write(*fd, bytes);
        }
    }
}

/// Escribe una cadena en el sink actual (sin salto de línea).
fn emit_str(s: &str) {
    emit(s.as_bytes());
}

/// Escribe una cadena y termina la línea.
///
/// En consola usa `\r\n` (terminales serie); en archivo usa `\n`.
fn emit_line(s: &str) {
    let mut sink = OUTPUT.lock();
    match &mut *sink {
        Sink::Console => {
            let _ = uart::write(s.as_bytes());
            let _ = uart::write(b"\r\n");
        }
        Sink::Ssh => {
            let _ = crate::shell::remote::command_output_to_ssh(s.as_bytes());
            let _ = crate::shell::remote::command_output_to_ssh(b"\r\n");
        }
        Sink::File(fd) => {
            let _ = vfs::write(*fd, s.as_bytes());
            let _ = vfs::write(*fd, b"\n");
        }
    }
}

/// Emite una línea de diagnóstico/error SIEMPRE a la consola, ignorando la
/// redirección (comportamiento estilo stderr).
fn eprint_line(s: &str) {
    // Estilo stderr: ignora la redirección `>` a archivo, pero va al sink BASE
    // (consola O canal SSH) para que los errores se VEAN en la sesión activa,
    // también la remota.
    match *BASE.lock() {
        Sink::Ssh => {
            let _ = crate::shell::remote::command_output_to_ssh(s.as_bytes());
            let _ = crate::shell::remote::command_output_to_ssh(b"\r\n");
        }
        _ => {
            let _ = uart::write(s.as_bytes());
            let _ = uart::write(b"\r\n");
        }
    }
}

/// Configura el sink de salida según la redirección del comando.
///
/// Abre (creando si hace falta) el archivo destino. Debe emparejarse SIEMPRE
/// con [`end_redirect`]. Devuelve el error del VFS si la apertura falla, en
/// cuyo caso el sink queda en consola.
pub fn begin_redirect(redirect: &Redirect) -> KResult<()> {
    match redirect {
        Redirect::None => {
            // Sin redirección: el destino es el sink BASE (consola o canal SSH).
            let base = *BASE.lock();
            *OUTPUT.lock() = base;
            Ok(())
        }
        Redirect::Truncate(path) => {
            let flags = OpenFlags(OpenFlags::WRONLY.0 | OpenFlags::CREATE.0 | OpenFlags::TRUNC.0);
            let fd = vfs::open(&resolve(path), flags)?;
            *OUTPUT.lock() = Sink::File(fd);
            Ok(())
        }
        Redirect::Append(path) => {
            let flags = OpenFlags(OpenFlags::WRONLY.0 | OpenFlags::CREATE.0 | OpenFlags::APPEND.0);
            let fd = vfs::open(&resolve(path), flags)?;
            *OUTPUT.lock() = Sink::File(fd);
            Ok(())
        }
    }
}

/// Cierra el archivo de redirección (si lo hubiera) y vuelve a consola.
pub fn end_redirect() {
    let mut sink = OUTPUT.lock();
    if let Sink::File(fd) = *sink {
        let _ = vfs::close(fd);
    }
    // Volver al sink BASE (consola o canal SSH), no forzar consola.
    *sink = *BASE.lock();
}

// ===========================================================================
// Despachador.
// ===========================================================================

/// Despacha un comando interno. Devuelve el código de salida. [CANÓNICO]
///
/// `name` y `args` son vistas prestadas que arma `shell::run` desde el
/// `Command` parseado (`args` NO incluye `name`). Nunca panica.
pub fn dispatch(name: &str, args: &[&str]) -> i32 {
    match name {
        "echo" => cmd_echo(args),
        "help" => cmd_help(args),
        "clear" => cmd_clear(),
        "uptime" => cmd_uptime(),
        "free" => cmd_free(),
        "ps" => cmd_ps(),
        "reboot" => cmd_reboot(),
        "ls" => cmd_ls(args),
        "cd" => cmd_cd(args),
        "pwd" => cmd_pwd(),
        "cat" => cmd_cat(args),
        "mkdir" => cmd_mkdir(args),
        "touch" => cmd_touch(args),
        "rm" => cmd_rm(args),
        "write" => cmd_write(args),
        "" => 0,
        other => {
            eprint_line(&format!("shell: comando no encontrado: {}", other));
            127
        }
    }
}

// ===========================================================================
// Comandos que no tocan el sistema de archivos.
// ===========================================================================

/// `echo [-n] TEXTO...` — imprime los argumentos unidos por espacios.
/// Con `-n` no añade salto de línea final.
fn cmd_echo(args: &[&str]) -> i32 {
    let mut newline = true;
    let mut start = 0usize;
    if let Some(&first) = args.first() {
        if first == "-n" {
            newline = false;
            start = 1;
        }
    }
    let mut out = String::new();
    // `start` es 0 o 1 y nunca supera `args.len()`: el slice es seguro.
    for (i, a) in args[start..].iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.push_str(a);
    }
    if newline {
        emit_line(&out);
    } else {
        emit_str(&out);
    }
    0
}

/// `help` — lista los comandos disponibles.
fn cmd_help(_args: &[&str]) -> i32 {
    emit_line("Comandos disponibles:");
    emit_line("  echo [-n] TEXTO...    imprime texto");
    emit_line("  help                  muestra esta ayuda");
    emit_line("  clear                 limpia la pantalla");
    emit_line("  uptime                tiempo desde el arranque");
    emit_line("  free                  uso del heap del kernel");
    emit_line("  ps                    tareas (tarea actual)");
    emit_line("  reboot                reinicia el sistema");
    emit_line("  ls [RUTA]             lista un directorio");
    emit_line("  cd [RUTA]             cambia de directorio (por defecto /)");
    emit_line("  pwd                   muestra el directorio actual");
    emit_line("  cat ARCHIVO...        muestra el contenido de archivos");
    emit_line("  mkdir DIR...          crea directorios");
    emit_line("  touch ARCHIVO...      crea archivos vacíos");
    emit_line("  rm ARCHIVO...         borra archivos");
    emit_line("  write ARCHIVO TEXTO   escribe TEXTO en ARCHIVO (trunca)");
    emit_line("");
    emit_line("Redirección: '> archivo' (trunca) y '>> archivo' (añade).");
    0
}

/// `clear` — borra la pantalla del terminal (secuencia ANSI).
fn cmd_clear() -> i32 {
    // ESC[2J borra la pantalla; ESC[H sitúa el cursor arriba-izquierda.
    emit_str("\x1b[2J\x1b[H");
    0
}

/// `uptime` — tiempo transcurrido desde el arranque.
fn cmd_uptime() -> i32 {
    let ms = timer::uptime_ms();
    let total_s = ms / 1000;
    let days = total_s / 86_400;
    let hours = (total_s % 86_400) / 3_600;
    let mins = (total_s % 3_600) / 60;
    let secs = total_s % 60;
    emit_line(&format!(
        "activo {}d {:02}:{:02}:{:02} ({} ms)",
        days, hours, mins, secs, ms
    ));
    0
}

/// `free` — instantánea de uso del heap del kernel.
fn cmd_free() -> i32 {
    let s = mm::stats();
    emit_line("            total        usado        libre");
    emit_line(&format!(
        "heap  {:>11}  {:>11}  {:>11}",
        s.total, s.used, s.free
    ));
    0
}

/// `ps` — tareas del planificador.
///
/// NOTA: el contrato del scheduler sólo expone `current()`; no hay API pública
/// para enumerar todas las tareas, así que mostramos la tarea en ejecución.
/// Cuando el scheduler ofrezca un iterador de TCBs, ampliar aquí.
fn cmd_ps() -> i32 {
    let tid = scheduler::current();
    emit_line("  TID  ESTADO    NOMBRE");
    emit_line(&format!("{:>5}  Running   (actual)", tid));
    0
}

/// `reboot` — reinicio por software del SoC (mejor esfuerzo).
fn cmd_reboot() -> i32 {
    eprint_line("Reiniciando el sistema...");
    // MEJOR ESFUERZO / RIESGO: el contrato del kernel no expone una primitiva
    // de reinicio, así que recurrimos a la del HAL. El nombre exacto puede
    // variar entre versiones de esp-hal; verificar contra la 0.23 instalada.
    // No debería retornar si tiene éxito.
    esp_hal::reset::software_reset();
    // Si por lo que sea la llamada retornara, bloqueamos para no continuar en
    // un estado inconsistente. (Código inalcanzable si `software_reset` es `!`.)
    #[allow(unreachable_code)]
    loop {
        core::hint::spin_loop();
    }
}

// ===========================================================================
// Comandos de sistema de archivos (VFS).
// ===========================================================================

/// `ls [RUTA]` — lista un directorio (por defecto `/`).
fn cmd_ls(args: &[&str]) -> i32 {
    // Sin argumento -> el directorio actual (CWD). Se resuelve a ruta absoluta.
    let raw = args.first().copied().unwrap_or(".");
    let path = resolve(raw);
    match vfs::readdir(&path) {
        Ok(entries) => {
            for e in &entries {
                emit_line(&format_entry(e));
            }
            0
        }
        Err(e) => {
            eprint_line(&format!("ls: {}: {}", raw, err_str(e)));
            1
        }
    }
}

/// `cd [RUTA]` — cambia el directorio de trabajo. Sin argumento, va a `/`.
fn cmd_cd(args: &[&str]) -> i32 {
    let target = args.first().copied().unwrap_or("/");
    let abs = resolve(target);
    // Debe existir y ser un directorio: `readdir` falla si no lo es o no existe.
    match vfs::readdir(&abs) {
        Ok(_) => {
            *CWD.lock() = abs;
            0
        }
        Err(e) => {
            eprint_line(&format!("cd: {}: {}", target, err_str(e)));
            1
        }
    }
}

/// `pwd` — imprime el directorio de trabajo actual.
fn cmd_pwd() -> i32 {
    emit_line(&cwd_get());
    0
}

/// Formatea una entrada de directorio con un sufijo según su tipo.
fn format_entry(e: &DirEntry) -> String {
    let tag = match e.kind {
        InodeKind::Dir => "/",
        InodeKind::Device => "@",
        InodeKind::Symlink => "~",
        InodeKind::File => "",
    };
    format!("{}{}", e.name, tag)
}

/// `cat ARCHIVO...` — vuelca el contenido de cada archivo.
fn cmd_cat(args: &[&str]) -> i32 {
    if args.is_empty() {
        eprint_line("uso: cat ARCHIVO...");
        return 2;
    }
    let mut status = 0;
    for path in args {
        if let Err(e) = cat_one(&resolve(path)) {
            eprint_line(&format!("cat: {}: {}", path, err_str(e)));
            status = 1;
        }
    }
    status
}

/// Vuelca un único archivo al sink actual, por bloques.
fn cat_one(path: &str) -> KResult<()> {
    let fd = vfs::open(path, OpenFlags::RDONLY)?;
    let mut buf = [0u8; 128];
    loop {
        match vfs::read(fd, &mut buf) {
            Ok(0) => break,
            Ok(n) => {
                // `get(..n)` evita cualquier indexado que pudiera desbordar.
                let chunk = buf.get(..n).unwrap_or(&[]);
                emit(chunk);
            }
            Err(e) => {
                let _ = vfs::close(fd);
                return Err(e);
            }
        }
    }
    let _ = vfs::close(fd);
    Ok(())
}

/// `mkdir DIR...` — crea uno o más directorios.
fn cmd_mkdir(args: &[&str]) -> i32 {
    if args.is_empty() {
        eprint_line("uso: mkdir DIRECTORIO...");
        return 2;
    }
    let mut status = 0;
    for path in args {
        if let Err(e) = vfs::mkdir(&resolve(path)) {
            eprint_line(&format!("mkdir: {}: {}", path, err_str(e)));
            status = 1;
        }
    }
    status
}

/// `touch ARCHIVO...` — crea archivos vacíos si no existen.
///
/// No actualiza marca de tiempo (el VFS del contrato no la expone); si el
/// archivo ya existe, es un no-op.
fn cmd_touch(args: &[&str]) -> i32 {
    if args.is_empty() {
        eprint_line("uso: touch ARCHIVO...");
        return 2;
    }
    let mut status = 0;
    let flags = OpenFlags(OpenFlags::WRONLY.0 | OpenFlags::CREATE.0);
    for path in args {
        match vfs::open(&resolve(path), flags) {
            Ok(fd) => {
                let _ = vfs::close(fd);
            }
            Err(e) => {
                eprint_line(&format!("touch: {}: {}", path, err_str(e)));
                status = 1;
            }
        }
    }
    status
}

/// `rm ARCHIVO...` — elimina archivos.
fn cmd_rm(args: &[&str]) -> i32 {
    if args.is_empty() {
        eprint_line("uso: rm ARCHIVO...");
        return 2;
    }
    let mut status = 0;
    for path in args {
        if let Err(e) = vfs::unlink(&resolve(path)) {
            eprint_line(&format!("rm: {}: {}", path, err_str(e)));
            status = 1;
        }
    }
    status
}

/// `write ARCHIVO TEXTO...` — escribe TEXTO (unido por espacios) en ARCHIVO,
/// truncándolo. Útil para probar el FS sin depender del operador `>`.
fn cmd_write(args: &[&str]) -> i32 {
    let path = match args.first() {
        Some(p) => *p,
        None => {
            eprint_line("uso: write ARCHIVO TEXTO...");
            return 2;
        }
    };
    let mut text = String::new();
    for (i, a) in args.iter().skip(1).enumerate() {
        if i > 0 {
            text.push(' ');
        }
        text.push_str(a);
    }
    text.push('\n');

    let flags = OpenFlags(OpenFlags::WRONLY.0 | OpenFlags::CREATE.0 | OpenFlags::TRUNC.0);
    let fd = match vfs::open(&resolve(path), flags) {
        Ok(fd) => fd,
        Err(e) => {
            eprint_line(&format!("write: {}: {}", path, err_str(e)));
            return 1;
        }
    };
    let res = vfs::write(fd, text.as_bytes());
    let _ = vfs::close(fd);
    match res {
        Ok(_) => 0,
        Err(e) => {
            eprint_line(&format!("write: {}: {}", path, err_str(e)));
            1
        }
    }
}

// ===========================================================================
// Utilidades.
// ===========================================================================

/// Traduce un [`KError`] a un texto breve en español para los mensajes.
fn err_str(e: KError) -> &'static str {
    match e {
        KError::NoMem => "sin memoria",
        KError::NotFound => "no encontrado",
        KError::AlreadyExists => "ya existe",
        KError::NotADirectory => "no es un directorio",
        KError::IsADirectory => "es un directorio",
        KError::InvalidArgument => "argumento inválido",
        KError::PermissionDenied => "permiso denegado",
        KError::NotSupported => "no soportado",
        KError::WouldBlock => "bloquearía",
        KError::Busy => "ocupado",
        KError::IoError => "error de E/S",
        KError::BadFd => "descriptor inválido",
        KError::NameTooLong => "nombre demasiado largo",
        KError::NoSpace => "sin espacio",
        KError::Corrupt => "datos corruptos",
        KError::Timeout => "tiempo de espera agotado",
        KError::Fault => "dirección inválida",
        KError::TableFull => "tabla llena",
    }
}
