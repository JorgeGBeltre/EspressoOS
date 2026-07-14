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
        "power" => cmd_power(args),
        "sha256" => cmd_sha256(args),
        "ble" => cmd_ble(args),
        "wifi" => cmd_wifi(args),
        "ip" => cmd_ip(args),
        "nmcli" => cmd_nmcli(args),
        "sudo" => cmd_sudo(args),
        "" => 0,
        other => {
            eprint_line(&format!("shell: command not found: {}", other));
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

fn cmd_wifi(args: &[&str]) -> i32 {
    use crate::drivers::wifi;
    match args {
        [] | ["status"] => {
            emit_line(&format!("state:  {:?}", wifi::status()));
            emit_line(&format!(
                "ssid:   {}",
                wifi::current_ssid().unwrap_or_else(|| String::from("(none)"))
            ));
            match wifi::current_ip() {
                Some(ip) => emit_line(&format!("ip:     {}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3])),
                None => emit_line("ip:     (none)"),
            }
            0
        }
        ["scan"] | ["list"] => {
            emit_line("Scanning for networks (briefly drops the Wi-Fi link)...");
            wifi::request_scan();
            let start = timer::uptime_ms();
            while wifi::scan_state() == wifi::SCAN_RUNNING {
                if timer::uptime_ms().saturating_sub(start) > 20_000 {
                    eprint_line(&format!(
                        "wifi: scan timed out [diag: {}]",
                        wifi::scan_diag()
                    ));
                    return 1;
                }
                scheduler::yield_now();
            }
            if wifi::scan_state() == wifi::SCAN_ERROR {
                eprint_line(&format!("wifi: scan failed [diag: {}]", wifi::scan_diag()));
                return 1;
            }
            let mut aps = wifi::scan_results();
            aps.sort_by(|a, b| b.rssi.cmp(&a.rssi));
            emit_line(&format!(
                "{:<32} {:>5}  {:>3}  {}",
                "SSID", "RSSI", "CH", "SEC"
            ));
            for ap in aps.iter() {
                let name = if ap.ssid.is_empty() {
                    "(hidden)"
                } else {
                    ap.ssid.as_str()
                };
                emit_line(&format!(
                    "{:<32} {:>5}  {:>3}  {}",
                    name,
                    ap.rssi,
                    ap.channel,
                    if ap.secured { "WPA" } else { "open" }
                ));
            }
            if aps.is_empty() {
                emit_line("(no networks found)");
            }
            0
        }
        ["connect", ssid] => {
            emit_line(&format!("Connecting to '{}' (open network)...", ssid));
            wifi::request_connect(String::from(*ssid), String::new());
            emit_line("(use 'wifi status' to check the result)");
            0
        }
        ["connect", ssid, password] => {
            emit_line(&format!("Connecting to '{}'...", ssid));
            wifi::request_connect(String::from(*ssid), String::from(*password));
            emit_line("(use 'wifi status' to check the result)");
            0
        }
        ["disconnect"] => {
            wifi::request_disconnect();
            emit_line("Disconnecting...");
            0
        }
        _ => {
            emit_line("usage: wifi status | scan | connect \"SSID\" [PASSWORD] | disconnect");
            1
        }
    }
}

fn cmd_ip(_args: &[&str]) -> i32 {
    use crate::drivers::wifi;

    match wifi::current_ip() {
        Some(ip) => emit_line(&format!(
            "wlan0: {}.{}.{}.{}  ssid \"{}\"  state {:?}",
            ip[0],
            ip[1],
            ip[2],
            ip[3],
            wifi::current_ssid().unwrap_or_else(|| String::from("(none)")),
            wifi::status()
        )),
        None => emit_line(&format!("wlan0: <no address>  state {:?}", wifi::status())),
    }
    0
}

fn cmd_sudo(args: &[&str]) -> i32 {
    if args.is_empty() {
        eprint_line("sudo: usage: sudo COMMAND [ARGS...]");
        return 1;
    }
    dispatch(args[0], &args[1..])
}

fn cmd_nmcli(args: &[&str]) -> i32 {
    match args {
        ["device", "status"] | ["dev", "status"] | ["general", "status"] | ["g", "status"] => {
            cmd_wifi(&["status"])
        }
        ["radio", ..] => {
            emit_line("wifi radio is always on");
            0
        }
        ["device", "wifi", "list"]
        | ["dev", "wifi", "list"]
        | ["device", "wifi"]
        | ["dev", "wifi"] => cmd_wifi(&["scan"]),
        ["device", "wifi", "connect", rest @ ..] | ["dev", "wifi", "connect", rest @ ..] => {
            let ssid = match rest.first() {
                Some(s) => *s,
                None => {
                    eprint_line("nmcli: missing SSID");
                    return 1;
                }
            };
            let pass = match rest {
                [_, "password", p, ..] => *p,
                _ => "",
            };
            if pass.is_empty() {
                cmd_wifi(&["connect", ssid])
            } else {
                cmd_wifi(&["connect", ssid, pass])
            }
        }
        _ => {
            emit_line("nmcli (EspressoOS shim). Supported:");
            emit_line("  nmcli device status");
            emit_line("  nmcli device wifi list");
            emit_line("  nmcli device wifi connect \"SSID\" password \"PASS\"");
            emit_line("(native equivalent: the 'wifi' command)");
            0
        }
    }
}

fn cmd_help(_args: &[&str]) -> i32 {
    emit_line("Available commands:");
    emit_line("  echo [-n] TEXT...     print text");
    emit_line("  help                  show this help");
    emit_line("  clear                 clear the screen");
    emit_line("  uptime                time since boot");
    emit_line("  free                  kernel heap usage");
    emit_line("  ps                    tasks (current task)");
    emit_line("  reboot                reboot the system");
    emit_line("  ls [PATH]             list a directory");
    emit_line("  cd [PATH]             change directory (default /)");
    emit_line("  pwd                   show the current directory");
    emit_line("  cat FILE...           show the contents of files");
    emit_line("  mkdir DIR...          create directories");
    emit_line("  touch FILE...         create empty files");
    emit_line("  rm FILE...            remove files");
    emit_line("  write FILE TEXT       write TEXT to FILE (truncate)");
    emit_line("  i2c scan|read|write   I2C bus (/dev/i2c0)");
    emit_line("  spi transfer B0...    SPI bus (/dev/spi0)");
    emit_line("  ota status|set|rx|apply  A/B update (otadata + OTA:3300)");
    emit_line("  syscalltest           exercise the syscall ABI");
    emit_line("  smp                   multicore status (SMP)");
    emit_line("  pms [world1]          memory protection (PMS)");
    emit_line("  power sleep|deep-sleep [seconds]  power management");
    emit_line("  sha256 [text]         hardware SHA-256 hashing");
    emit_line("  ble status|advertise  Bluetooth LE management and advertising");
    emit_line("  wifi status|scan|connect \"SSID\" [PASS]|disconnect   Wi-Fi management");
    emit_line("  ip                    show the wlan0 address");
    emit_line("");
    emit_line("Redirection: '> file' (truncate) and '>> file' (append).");
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
        "up {}d {:02}:{:02}:{:02} ({} ms)",
        days, hours, mins, secs, ms
    ));
    0
}

fn cmd_free() -> i32 {
    let s = mm::stats();
    emit_line("            total         used         free");
    emit_line(&format!(
        "heap  {:>11}  {:>11}  {:>11}",
        s.total, s.used, s.free
    ));
    0
}

fn cmd_ps() -> i32 {
    let tid = scheduler::current();
    emit_line("  TID  STATE     NAME");
    emit_line(&format!("{:>5}  Running   (current)", tid));
    0
}

fn cmd_reboot() -> i32 {
    eprint_line("Rebooting the system...");

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
        eprint_line("usage: cat FILE...");
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
        eprint_line("usage: mkdir DIRECTORY...");
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
        eprint_line("usage: touch FILE...");
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
        eprint_line("usage: rm FILE...");
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
            eprint_line("usage: write FILE TEXT...");
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
                eprint_line("i2c: bus not initialized");
                return 1;
            }
            emit_line("Scanning I2C bus (0x08..0x77)...");
            let mut found = 0u32;
            for addr in i2c::SCAN_FIRST..=i2c::SCAN_LAST {
                if i2c::probe(addr) {
                    emit_line(&format!("  device at 0x{:02x}", addr));
                    found += 1;
                }
            }
            emit_line(&format!("{} device(s) found", found));
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
                    eprint_line("usage: i2c read ADDR_HEX LEN(1..64)");
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
                                eprint_line(&format!("i2c write: invalid byte: {}", s));
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
                    eprint_line("usage: i2c write ADDR_HEX B0 [B1 ...]");
                    2
                }
            }
        }
        _ => {
            eprint_line("usage: i2c scan | i2c read ADDR LEN | i2c write ADDR B0 ...");
            2
        }
    }
}

fn cmd_spi(args: &[&str]) -> i32 {
    use crate::drivers::spi;
    match args.first().copied() {
        Some("transfer") => {
            if args.len() < 2 {
                eprint_line("usage: spi transfer B0 [B1 ...]");
                return 2;
            }
            let mut tx = Vec::new();
            for s in &args[1..] {
                match parse_u8_hex(s) {
                    Some(b) => tx.push(b),
                    None => {
                        eprint_line(&format!("spi transfer: invalid byte: {}", s));
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
            eprint_line("usage: spi transfer B0 [B1 ...]");
            2
        }
    }
}

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
            emit_line(&format!("active slot: {}", slot_name(ota::active_slot())));
            match ota::otadata_entries() {
                Ok(entries) => {
                    for (i, e) in entries.iter().enumerate() {
                        let seq = if e.ota_seq == 0xFFFF_FFFF {
                            String::from("(empty)")
                        } else {
                            format!("{}", e.ota_seq)
                        };
                        emit_line(&format!(
                            "  otadata[{}]: seq={} state={} ({})",
                            i,
                            seq,
                            ota_state_str(e.ota_state),
                            if e.is_valid() {
                                "valid"
                            } else {
                                "invalid/empty"
                            }
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
                    emit_line(&format!("marked for boot: {}", slot_name(slot)));
                    emit_line("note: the actual switch at boot requires a bootloader that");
                    emit_line("honors otadata; see the OTA section of the README.");
                    0
                }
                Err(e) => {
                    eprint_line(&format!("ota set: {}", err_str(e)));
                    1
                }
            },
            None => {
                eprint_line("usage: ota set factory|ota0");
                2
            }
        },
        Some("rx") => {
            let n = ota::rx_len();
            if n == 0 {
                emit_line("no image in buffer (send it: nc <ip> 3300 < firmware.bin)");
            } else {
                emit_line(&format!("image in buffer: {} bytes (use 'ota apply')", n));
            }
            0
        }
        Some("apply") => {
            emit_line("Flashing received image to the inactive slot...");
            emit_line("(writes several MB; with WiFi active it may cut the radio)");
            match ota::apply_buffered() {
                Ok(slot) => {
                    emit_line(&format!(
                        "OK: image written to {} and marked for boot",
                        slot_name(slot)
                    ));
                    emit_line("(effective only with a bootloader that honors otadata)");
                    0
                }
                Err(e) => {
                    eprint_line(&format!("ota apply: {}", err_str(e)));
                    1
                }
            }
        }
        _ => {
            eprint_line("usage: ota status | set factory|ota0 | rx | apply");
            2
        }
    }
}

fn cmd_syscalltest() -> i32 {
    use crate::syscall::{invoke, Syscall};

    let up = invoke(Syscall::UptimeMs.number(), [0; 6]);
    emit_line(&format!("SYS_UptimeMs -> {} ms", up));

    let free = invoke(Syscall::Sbrk.number(), [0; 6]);
    emit_line(&format!("SYS_Sbrk(free) -> {} bytes", free));

    let path = "/dev/console";
    let flags = OpenFlags::WRONLY.0 as usize;
    let fd = invoke(
        Syscall::Open.number(),
        [path.as_ptr() as usize, path.len(), flags, 0, 0, 0],
    );
    if fd >= 0 {
        let msg = b"  <- line written by SYS_Write to /dev/console\r\n";
        let n = invoke(
            Syscall::Write.number(),
            [fd as usize, msg.as_ptr() as usize, msg.len(), 0, 0, 0],
        );
        let _ = invoke(Syscall::Close.number(), [fd as usize, 0, 0, 0, 0, 0]);
        emit_line(&format!(
            "SYS_Open/Write/Close /dev/console -> fd={} written={}",
            fd, n
        ));
    } else {
        emit_line(&format!("SYS_Open /dev/console -> error {}", fd));
    }

    let y = invoke(Syscall::Yield.number(), [0; 6]);
    emit_line(&format!("SYS_Yield -> {}", y));

    if cfg!(feature = "syscall-trap") {
        emit_line("(via real `syscall` instruction / CPU trap)");
    } else {
        emit_line("(via software gate; --features syscall-trap for the real trap)");
    }
    0
}

fn cmd_smp() -> i32 {
    use crate::scheduler::core_sync;
    emit_line(&format!(
        "core serving the shell: core {}",
        core_sync::current_core_id()
    ));
    if cfg!(feature = "smp") {
        if core_sync::is_running() {
            emit_line(&format!(
                "SMP: APP_CPU (core 1) active — ticks core1 = {}",
                core_sync::core1_ticks()
            ));
        } else {
            emit_line("SMP: compiled but the APP_CPU didn't start");
        }
    } else {
        emit_line("SMP: not compiled (use: cargo build --release --features smp)");
    }
    0
}

fn cmd_pms(args: &[&str]) -> i32 {
    if !cfg!(feature = "pms") {
        eprint_line("pms: not compiled (use: cargo build --release --features pms)");
        return 1;
    }
    match args.first().copied() {
        Some("world1") => {
            emit_line("PMS: applying World-1 restriction (experimental)...");
            match crate::mm::mpu::protect_world1_wx() {
                Some(s) => {
                    emit_line(&s);
                    0
                }
                None => {
                    eprint_line("pms: not available");
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
                eprint_line("pms: not available");
                1
            }
        },
    }
}

fn cmd_power(args: &[&str]) -> i32 {
    let mode = args.first().copied();
    let secs = args.get(1).and_then(|s| s.parse::<u64>().ok()).unwrap_or(5);

    match mode {
        Some("sleep") => {
            crate::drivers::power::enter_light_sleep(secs);
            0
        }
        Some("deep-sleep") => {
            crate::drivers::power::enter_deep_sleep(secs);
        }
        _ => {
            eprint_line("usage: power sleep [seconds] | power deep-sleep [seconds]");
            1
        }
    }
}

fn cmd_sha256(args: &[&str]) -> i32 {
    let input = args.first().copied().unwrap_or("hello");
    let hash = crate::drivers::crypto::sha256(input.as_bytes());
    let mut hex = String::new();
    for &b in &hash {
        hex.push_str(&format!("{:02x}", b));
    }
    emit_line(&format!("SHA256(\"{}\") = {}", input, hex));
    0
}

fn cmd_ble(args: &[&str]) -> i32 {
    let sub = args.first().copied();
    match sub {
        Some("status") => {
            if crate::drivers::ble::is_advertising() {
                emit_line("BLE: Actively advertising as 'EspressoOS'");
            } else {
                emit_line("BLE: Inactive (not advertising)");
            }
            0
        }
        Some("advertise") => {
            crate::drivers::ble::start_advertising();
            0
        }
        _ => {
            eprint_line("usage: ble status | ble advertise");
            1
        }
    }
}

fn err_str(e: KError) -> &'static str {
    match e {
        KError::NoMem => "out of memory",
        KError::NotFound => "not found",
        KError::AlreadyExists => "already exists",
        KError::NotADirectory => "not a directory",
        KError::IsADirectory => "is a directory",
        KError::InvalidArgument => "invalid argument",
        KError::PermissionDenied => "permission denied",
        KError::NotSupported => "not supported",
        KError::WouldBlock => "would block",
        KError::Busy => "busy",
        KError::IoError => "I/O error",
        KError::BadFd => "invalid descriptor",
        KError::NameTooLong => "name too long",
        KError::NoSpace => "out of space",
        KError::Corrupt => "corrupt data",
        KError::Timeout => "timed out",
        KError::Fault => "invalid address",
        KError::TableFull => "table full",
    }
}
