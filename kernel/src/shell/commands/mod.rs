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

#[derive(Clone, Copy)]
enum Sink {

    Console,

    Ssh,

    File(vfs::Fd),
}

static OUTPUT: Mutex<Sink> = Mutex::new(Sink::Console);

static BASE: Mutex<Sink> = Mutex::new(Sink::Console);

pub fn set_base_ssh() {
    *BASE.lock() = Sink::Ssh;
    *OUTPUT.lock() = Sink::Ssh;
}

pub fn set_base_console() {
    *BASE.lock() = Sink::Console;
    *OUTPUT.lock() = Sink::Console;
}

static CWD: Mutex<String> = Mutex::new(String::new());

pub fn cwd_get() -> String {
    let c = CWD.lock();
    if c.is_empty() {
        String::from("/")
    } else {
        c.clone()
    }
}

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

            let _ = vfs::write(*fd, bytes);
        }
    }
}

fn emit_str(s: &str) {
    emit(s.as_bytes());
}

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

fn eprint_line(s: &str) {

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

pub fn begin_redirect(redirect: &Redirect) -> KResult<()> {
    match redirect {
        Redirect::None => {

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

pub fn end_redirect() {
    let mut sink = OUTPUT.lock();
    if let Sink::File(fd) = *sink {
        let _ = vfs::close(fd);
    }

    *sink = *BASE.lock();
}

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
        "i2c" => cmd_i2c(args),
        "spi" => cmd_spi(args),
        "ota" => cmd_ota(args),
        "syscalltest" => cmd_syscalltest(),
        "smp" => cmd_smp(),
        "pms" => cmd_pms(args),
        "" => 0,
        other => {
            eprint_line(&format!("shell: comando no encontrado: {}", other));
            127
        }
    }
}

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
    emit_line("  i2c scan|read|write   bus I2C (/dev/i2c0)");
    emit_line("  spi transfer B0...    bus SPI (/dev/spi0)");
    emit_line("  ota status|set        actualización A/B (otadata)");
    emit_line("  syscalltest           ejercita la ABI de syscalls");
    emit_line("  smp                   estado del multinúcleo (SMP)");
    emit_line("  pms [world1]          protección de memoria (PMS)");
    emit_line("");
    emit_line("Redirección: '> archivo' (trunca) y '>> archivo' (añade).");
    0
}

fn cmd_clear() -> i32 {

    emit_str("\x1b[2J\x1b[H");
    0
}

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

fn cmd_free() -> i32 {
    let s = mm::stats();
    emit_line("            total        usado        libre");
    emit_line(&format!(
        "heap  {:>11}  {:>11}  {:>11}",
        s.total, s.used, s.free
    ));
    0
}

fn cmd_ps() -> i32 {
    let tid = scheduler::current();
    emit_line("  TID  ESTADO    NOMBRE");
    emit_line(&format!("{:>5}  Running   (actual)", tid));
    0
}

fn cmd_reboot() -> i32 {
    eprint_line("Reiniciando el sistema...");

    esp_hal::reset::software_reset();

    #[allow(unreachable_code)]
    loop {
        core::hint::spin_loop();
    }
}

fn cmd_ls(args: &[&str]) -> i32 {

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

fn cmd_cd(args: &[&str]) -> i32 {
    let target = args.first().copied().unwrap_or("/");
    let abs = resolve(target);

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

fn cmd_pwd() -> i32 {
    emit_line(&cwd_get());
    0
}

fn format_entry(e: &DirEntry) -> String {
    let tag = match e.kind {
        InodeKind::Dir => "/",
        InodeKind::Device => "@",
        InodeKind::Symlink => "~",
        InodeKind::File => "",
    };
    format!("{}{}", e.name, tag)
}

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

fn cat_one(path: &str) -> KResult<()> {
    let fd = vfs::open(path, OpenFlags::RDONLY)?;
    let mut buf = [0u8; 128];
    loop {
        match vfs::read(fd, &mut buf) {
            Ok(0) => break,
            Ok(n) => {

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

// ============================================================================
// Comandos de bus (Fase 3): i2c / spi.
// ============================================================================

fn parse_u8_hex(s: &str) -> Option<u8> {
    let t = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    u8::from_str_radix(t, 16).ok()
}

fn zeroed(len: usize) -> Vec<u8> {
    let mut v = Vec::new();
    v.resize(len, 0u8);
    v
}

fn hex_bytes(data: &[u8]) -> String {
    let mut s = String::new();
    for (i, b) in data.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        s.push_str(&format!("{:02x}", b));
    }
    s
}

fn cmd_i2c(args: &[&str]) -> i32 {
    use crate::drivers::i2c;
    match args.first().copied() {
        Some("scan") => {
            if !i2c::is_ready() {
                eprint_line("i2c: bus no inicializado");
                return 1;
            }
            emit_line("Escaneando bus I2C (0x08..0x77)...");
            let mut found = 0u32;
            for addr in i2c::SCAN_FIRST..=i2c::SCAN_LAST {
                if i2c::probe(addr) {
                    emit_line(&format!("  dispositivo en 0x{:02x}", addr));
                    found += 1;
                }
            }
            emit_line(&format!("{} dispositivo(s) encontrado(s)", found));
            0
        }
        Some("read") => {
            let addr = args.get(1).and_then(|s| parse_u8_hex(s));
            let len = args.get(2).and_then(|s| s.parse::<usize>().ok());
            match (addr, len) {
                (Some(addr), Some(len)) if (1..=64).contains(&len) => {
                    let mut buf = zeroed(len);
                    match i2c::read(addr, &mut buf) {
                        Ok(()) => {
                            emit_line(&hex_bytes(&buf));
                            0
                        }
                        Err(e) => {
                            eprint_line(&format!("i2c read: {}", err_str(e)));
                            1
                        }
                    }
                }
                _ => {
                    eprint_line("uso: i2c read ADDR_HEX LEN(1..64)");
                    2
                }
            }
        }
        Some("write") => {
            let addr = args.get(1).and_then(|s| parse_u8_hex(s));
            match addr {
                Some(addr) if args.len() >= 3 => {
                    let mut data = Vec::new();
                    for s in &args[2..] {
                        match parse_u8_hex(s) {
                            Some(b) => data.push(b),
                            None => {
                                eprint_line(&format!("i2c write: byte inválido: {}", s));
                                return 2;
                            }
                        }
                    }
                    match i2c::write(addr, &data) {
                        Ok(()) => 0,
                        Err(e) => {
                            eprint_line(&format!("i2c write: {}", err_str(e)));
                            1
                        }
                    }
                }
                _ => {
                    eprint_line("uso: i2c write ADDR_HEX B0 [B1 ...]");
                    2
                }
            }
        }
        _ => {
            eprint_line("uso: i2c scan | i2c read ADDR LEN | i2c write ADDR B0 ...");
            2
        }
    }
}

fn cmd_spi(args: &[&str]) -> i32 {
    use crate::drivers::spi;
    match args.first().copied() {
        Some("transfer") => {
            if args.len() < 2 {
                eprint_line("uso: spi transfer B0 [B1 ...]");
                return 2;
            }
            let mut tx = Vec::new();
            for s in &args[1..] {
                match parse_u8_hex(s) {
                    Some(b) => tx.push(b),
                    None => {
                        eprint_line(&format!("spi transfer: byte inválido: {}", s));
                        return 2;
                    }
                }
            }
            let mut rx = zeroed(tx.len());
            match spi::transfer(&tx, &mut rx) {
                Ok(()) => {
                    emit_line(&hex_bytes(&rx));
                    0
                }
                Err(e) => {
                    eprint_line(&format!("spi transfer: {}", err_str(e)));
                    1
                }
            }
        }
        _ => {
            eprint_line("uso: spi transfer B0 [B1 ...]");
            2
        }
    }
}

// ============================================================================
// Comando OTA A/B (Fase 5): status / set.
// ============================================================================

fn slot_name(s: crate::ota::Slot) -> &'static str {
    match s {
        crate::ota::Slot::Factory => "factory",
        crate::ota::Slot::Ota0 => "ota0",
    }
}

fn parse_slot(s: &str) -> Option<crate::ota::Slot> {
    match s {
        "factory" => Some(crate::ota::Slot::Factory),
        "ota0" | "ota_0" => Some(crate::ota::Slot::Ota0),
        _ => None,
    }
}

fn ota_state_str(raw: u32) -> &'static str {
    use crate::ota::OtaImgState::*;
    match crate::ota::OtaImgState::from_raw(raw) {
        New => "new",
        PendingVerify => "pending-verify",
        Valid => "valid",
        Invalid => "invalid",
        Aborted => "aborted",
        Undefined => "undef",
    }
}

fn cmd_ota(args: &[&str]) -> i32 {
    use crate::ota;
    match args.first().copied() {
        Some("status") => {
            emit_line(&format!("slot activo: {}", slot_name(ota::active_slot())));
            match ota::otadata_entries() {
                Ok(entries) => {
                    for (i, e) in entries.iter().enumerate() {
                        let seq = if e.ota_seq == 0xFFFF_FFFF {
                            String::from("(vacío)")
                        } else {
                            format!("{}", e.ota_seq)
                        };
                        emit_line(&format!(
                            "  otadata[{}]: seq={} state={} ({})",
                            i,
                            seq,
                            ota_state_str(e.ota_state),
                            if e.is_valid() { "válido" } else { "inválido/vacío" }
                        ));
                    }
                    0
                }
                Err(e) => {
                    eprint_line(&format!("ota status: {}", err_str(e)));
                    1
                }
            }
        }
        Some("set") => match args.get(1).copied().and_then(parse_slot) {
            Some(slot) => match ota::set_boot_slot(slot) {
                Ok(()) => {
                    emit_line(&format!("marcado para arrancar: {}", slot_name(slot)));
                    emit_line("nota: el switch real en boot requiere un bootloader que");
                    emit_line("honre otadata; ver la sección OTA del README.");
                    0
                }
                Err(e) => {
                    eprint_line(&format!("ota set: {}", err_str(e)));
                    1
                }
            },
            None => {
                eprint_line("uso: ota set factory|ota0");
                2
            }
        },
        _ => {
            eprint_line("uso: ota status | ota set factory|ota0");
            2
        }
    }
}

// ============================================================================
// Ejercicio de la ABI de syscalls (Fase 6).
// ============================================================================

fn cmd_syscalltest() -> i32 {
    use crate::syscall::{invoke, Syscall};

    let up = invoke(Syscall::UptimeMs.number(), [0; 6]);
    emit_line(&format!("SYS_UptimeMs -> {} ms", up));

    let free = invoke(Syscall::Sbrk.number(), [0; 6]);
    emit_line(&format!("SYS_Sbrk(libre) -> {} bytes", free));

    // Open/Write/Close sobre /dev/console vía la ABI.
    let path = "/dev/console";
    let flags = OpenFlags::WRONLY.0 as usize;
    let fd = invoke(
        Syscall::Open.number(),
        [path.as_ptr() as usize, path.len(), flags, 0, 0, 0],
    );
    if fd >= 0 {
        let msg = b"  <- linea escrita por SYS_Write a /dev/console\r\n";
        let n = invoke(
            Syscall::Write.number(),
            [fd as usize, msg.as_ptr() as usize, msg.len(), 0, 0, 0],
        );
        let _ = invoke(Syscall::Close.number(), [fd as usize, 0, 0, 0, 0, 0]);
        emit_line(&format!(
            "SYS_Open/Write/Close /dev/console -> fd={} escritos={}",
            fd, n
        ));
    } else {
        emit_line(&format!("SYS_Open /dev/console -> error {}", fd));
    }

    let y = invoke(Syscall::Yield.number(), [0; 6]);
    emit_line(&format!("SYS_Yield -> {}", y));

    if cfg!(feature = "syscall-trap") {
        emit_line("(vía instrucción `syscall` real / trap de CPU)");
    } else {
        emit_line("(vía puerta software; --features syscall-trap para el trap real)");
    }
    0
}

// ============================================================================
// Estado SMP (Fase 9).
// ============================================================================

fn cmd_smp() -> i32 {
    use crate::scheduler::core_sync;
    emit_line(&format!("núcleo que atiende la shell: core {}", core_sync::current_core_id()));
    if cfg!(feature = "smp") {
        if core_sync::is_running() {
            emit_line(&format!(
                "SMP: APP_CPU (core 1) activo — ticks core1 = {}",
                core_sync::core1_ticks()
            ));
        } else {
            emit_line("SMP: compilado pero el APP_CPU no arrancó");
        }
    } else {
        emit_line("SMP: no compilado (usa: cargo build --release --features smp)");
    }
    0
}

// ============================================================================
// Protección de memoria PMS (Fase 8).
// ============================================================================

fn cmd_pms(args: &[&str]) -> i32 {
    if !cfg!(feature = "pms") {
        eprint_line("pms: no compilado (usa: cargo build --release --features pms)");
        return 1;
    }
    match args.first().copied() {
        Some("world1") => {
            emit_line("PMS: aplicando restricción de World-1 (experimental)...");
            match crate::mm::mpu::protect_world1() {
                Some(s) => {
                    emit_line(&s);
                    0
                }
                None => {
                    eprint_line("pms: no disponible");
                    1
                }
            }
        }
        _ => match crate::mm::mpu::report() {
            Some(s) => {
                emit_line(&s);
                0
            }
            None => {
                eprint_line("pms: no disponible");
                1
            }
        },
    }
}

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
