#![no_std]
#![no_main]

use libc::{
    arg, close, kill, kill_pgid, open, read, readdir, rename, setpgid, sleep_ms, spawn, stat,
    unlink, write, StatBuf, O_APPEND, O_CREATE, O_RDONLY, O_TRUNC, O_WRONLY,
};

struct KernelUnit {
    name: &'static str,
    sub: &'static str,
    desc: &'static str,
}

const KERNEL_UNITS: &[KernelUnit] = &[
    KernelUnit { name: "console", sub: "running", desc: "UART0 interactive serial console" },
    KernelUnit { name: "vfs", sub: "mounted", desc: "Virtual File System & NOR flash storage" },
    KernelUnit { name: "net", sub: "running", desc: "Wi-Fi 802.11 STA & smoltcp IPv4 stack" },
    KernelUnit { name: "ssh", sub: "listening", desc: "SSH-2.0 server (:22)" },
    KernelUnit { name: "ota", sub: "ready", desc: "Dual-slot A/B firmware supervisor" },
    KernelUnit { name: "watchdog", sub: "running", desc: "Hardware watchdog & health monitor" },
];

#[inline(never)]
fn puts(s: &str) {
    if !s.is_empty() {
        let _ = write(1, s.as_bytes());
    }
}

#[inline(never)]
fn putln(s: &str) {
    puts(s);
    let _ = write(1, b"\n");
}

#[inline(never)]
fn print_col(s: &str, width: usize) {
    puts(s);
    let pad = width.saturating_sub(s.len());
    for _ in 0..pad {
        let _ = write(1, b" ");
    }
}

#[inline(never)]
fn ascii_str(b: &[u8]) -> &str {
    for &c in b {
        if c >= 0x80 {
            return "";
        }
    }
    unsafe { core::str::from_utf8_unchecked(b) }
}

fn strip_svc(s: &str) -> &str {
    let t = s.trim();
    t.strip_suffix(".service").unwrap_or(t)
}

fn build_path<'a>(pfx: &[u8], mid: &str, sfx: &[u8], buf: &'a mut [u8; 64]) -> &'a str {
    let mb = mid.as_bytes();
    if pfx.len() + mb.len() + sfx.len() >= buf.len() {
        return "";
    }
    let mut k = 0;
    buf[..pfx.len()].copy_from_slice(pfx);
    k += pfx.len();
    buf[k..k + mb.len()].copy_from_slice(mb);
    k += mb.len();
    buf[k..k + sfx.len()].copy_from_slice(sfx);
    k += sfx.len();
    ascii_str(&buf[..k])
}

fn supervisor_online() -> bool {
    let mut st = StatBuf::zeroed();
    stat("/run/systemd/units.state", &mut st) == 0
}

fn send_ctl(cmd: &str, unit: &str) -> bool {
    if !supervisor_online() {
        return false;
    }
    let fd = open("/run/init.ctl", O_WRONLY | O_CREATE | O_APPEND);
    if fd < 0 {
        return false;
    }
    let _ = write(fd as i32, cmd.as_bytes());
    if !unit.is_empty() {
        let _ = write(fd as i32, b" ");
        let _ = write(fd as i32, unit.as_bytes());
    }
    let _ = write(fd as i32, b"\n");
    close(fd as i32);

    let mut st = StatBuf::zeroed();
    for _ in 0..8 {
        sleep_ms(50);
        if stat("/run/init.ctl", &mut st) < 0 {
            return true;
        }
    }
    true
}

fn read_file<'a>(path: &str, buf: &'a mut [u8]) -> &'a str {
    let fd = open(path, O_RDONLY);
    if fd < 0 {
        return "";
    }
    let n = read(fd as i32, buf);
    close(fd as i32);
    if n > 0 {
        ascii_str(&buf[..n as usize])
    } else {
        ""
    }
}

fn fallback_direct_action(action: &str, unit: &str) -> i32 {
    let mut pbuf = [0u8; 64];
    let upath = build_path(b"/etc/services.d/", unit, b".service", &mut pbuf);
    let mut fbuf = [0u8; 512];
    let text = read_file(upath, &mut fbuf);
    if text.is_empty() {
        puts("systemctl: unit not found: ");
        putln(unit);
        return 5;
    }

    let mut pid_buf = [0u8; 64];
    let pid_path = build_path(b"/run/", unit, b".pid", &mut pid_buf);

    if action == "stop" || action == "restart" {
        let mut rbuf = [0u8; 16];
        let ptxt = read_file(pid_path, &mut rbuf);
        let mut pid_val = 0u32;
        for &b in ptxt.trim().as_bytes() {
            if b >= b'0' && b <= b'9' {
                pid_val = pid_val * 10 + (b - b'0') as u32;
            }
        }
        if pid_val > 0 {
            let _ = kill_pgid(pid_val, 15);
            let _ = kill(pid_val, 15);
        }
        let _ = unlink(pid_path);
        if action == "stop" {
            return 0;
        }
    }

    let mut exec_line = "";
    for line in text.lines() {
        if let Some(rest) = line.trim().strip_prefix("ExecStart=") {
            exec_line = rest.trim();
            break;
        }
    }
    if exec_line.is_empty() {
        return 1;
    }

    let mut cbuf = [0u8; 65];
    let clen = exec_line.len().min(64);
    cbuf[..clen].copy_from_slice(&exec_line.as_bytes()[..clen]);
    cbuf[clen] = 0;

    let mut argv = [core::ptr::null(); 9];
    let mut argc = 0usize;
    let mut r = 0usize;
    while r < clen && argc < 8 {
        while r < clen && cbuf[r] == b' ' {
            r += 1;
        }
        if r >= clen {
            break;
        }
        let s = r;
        while r < clen && cbuf[r] != b' ' {
            r += 1;
        }
        cbuf[r] = 0;
        argv[argc] = unsafe { cbuf.as_ptr().add(s) };
        argc += 1;
        r += 1;
    }
    argv[argc] = core::ptr::null();
    if argc == 0 {
        return 1;
    }
    let mut c0_end = 0;
    while c0_end < clen && cbuf[c0_end] != 0 {
        c0_end += 1;
    }
    let cmd0 = ascii_str(&cbuf[..c0_end]);
    let pid = spawn(cmd0, argv.as_ptr());
    if pid < 0 {
        puts("systemctl: failed to spawn ");
        putln(cmd0);
        return 1;
    }
    let _ = setpgid(pid as u32, pid as u32);
    let pfd = open(pid_path, O_WRONLY | O_CREATE | O_TRUNC);
    if pfd >= 0 {
        let mut nbuf = [0u8; 16];
        let mut v = pid as u32;
        let mut tmp = [0u8; 16];
        let mut k = 0;
        while v > 0 {
            tmp[k] = b'0' + (v % 10) as u8;
            v /= 10;
            k += 1;
        }
        for idx in 0..k {
            nbuf[idx] = tmp[k - 1 - idx];
        }
        nbuf[k] = b'\n';
        let _ = write(pfd as i32, &nbuf[..k + 1]);
        close(pfd as i32);
    }
    0
}

fn list_units() -> i32 {
    putln("UNIT               LOAD    ACTIVE      SUB           PID   DESCRIPTION");
    putln("-----------------------------------------------------------------------------");
    for ku in KERNEL_UNITS {
        print_col(ku.name, 19);
        puts("kernel  active      ");
        print_col(ku.sub, 14);
        puts("-     ");
        putln(ku.desc);
    }

    let mut sbuf = [0u8; 1024];
    let stext = read_file("/run/systemd/units.state", &mut sbuf);
    if !stext.is_empty() {
        for line in stext.lines() {
            let mut p = line.split('|');
            let name = p.next().unwrap_or("");
            let _typ = p.next().unwrap_or("simple");
            let _en = p.next().unwrap_or("disabled");
            let active = p.next().unwrap_or("inactive");
            let sub = p.next().unwrap_or("dead");
            let pid = p.next().unwrap_or("0");
            let _restarts = p.next().unwrap_or("0");
            let desc = p.next().unwrap_or("");
            let pid_disp = if pid == "0" { "-" } else { pid };
            let mut lbuf = [0u8; 64];
            let ulabel = build_path(b"", name, b".service", &mut lbuf);
            print_col(ulabel, 19);
            puts("loaded  ");
            print_col(active, 12);
            print_col(sub, 14);
            print_col(pid_disp, 6);
            putln(desc);
        }
    } else {
        let mut dbuf = [0u8; 1024];
        let n = readdir("/etc/services.d", &mut dbuf);
        if n > 0 {
            let limit = n as usize;
            let mut pos = 0usize;
            while pos + 11 <= limit {
                let nl = u16::from_le_bytes([dbuf[pos + 9], dbuf[pos + 10]]) as usize;
                pos += 11;
                if pos + nl > limit {
                    break;
                }
                let fname = ascii_str(&dbuf[pos..pos + nl]);
                if fname.ends_with(".service") {
                    print_col(fname, 19);
                    putln("loaded  inactive    dead          -     unit file");
                }
                pos += nl;
            }
        }
    }
    0
}

fn list_unit_files() -> i32 {
    putln("UNIT FILE                STATE");
    putln("------------------------------------");
    let mut cbuf = [0u8; 512];
    let conf = read_file("/etc/services.conf", &mut cbuf);
    let mut dbuf = [0u8; 1024];
    let n = readdir("/etc/services.d", &mut dbuf);
    if n <= 0 {
        return 0;
    }
    let limit = n as usize;
    let mut pos = 0usize;
    while pos + 11 <= limit {
        let nl = u16::from_le_bytes([dbuf[pos + 9], dbuf[pos + 10]]) as usize;
        pos += 11;
        if pos + nl > limit {
            break;
        }
        let fname = ascii_str(&dbuf[pos..pos + nl]);
        if let Some(uname) = fname.strip_suffix(".service") {
            let mut enabled = false;
            for l in conf.lines() {
                if !l.trim().starts_with('#') && strip_svc(l) == uname {
                    enabled = true;
                    break;
                }
            }
            print_col(fname, 25);
            putln(if enabled { "enabled" } else { "disabled" });
        }
        pos += nl;
    }
    0
}

fn show_unit_log(unit: &str) {
    let mut lbuf = [0u8; 64];
    let path = build_path(b"/var/log/", unit, b".log", &mut lbuf);
    let mut buf = [0u8; 512];
    let text = read_file(path, &mut buf);
    if !text.is_empty() {
        puts("     Logs (");
        puts(path);
        putln("):");
        let mut lines = [""; 16];
        let mut count = 0usize;
        for l in text.lines() {
            lines[count % 16] = l;
            count += 1;
        }
        let start = count.saturating_sub(6);
        for idx in start..count {
            puts("       ");
            putln(lines[idx % 16]);
        }
    }
}

fn show_status(raw_unit: &str) -> i32 {
    let unit = strip_svc(raw_unit);
    for ku in KERNEL_UNITS {
        if ku.name == unit {
            puts("* ");
            puts(ku.name);
            puts(".kernel - ");
            putln(ku.desc);
            putln("     Loaded: kernel built-in");
            puts("     Active: active (");
            puts(ku.sub);
            putln(")");
            return 0;
        }
    }

    let mut sbuf = [0u8; 1024];
    let stext = read_file("/run/systemd/units.state", &mut sbuf);
    for line in stext.lines() {
        let mut p = line.split('|');
        let name = p.next().unwrap_or("");
        if name == unit {
            let typ = p.next().unwrap_or("simple");
            let en = p.next().unwrap_or("disabled");
            let active = p.next().unwrap_or("inactive");
            let sub = p.next().unwrap_or("dead");
            let pid = p.next().unwrap_or("0");
            let restarts = p.next().unwrap_or("0");
            let desc = p.next().unwrap_or("");
            puts("* ");
            puts(name);
            puts(".service - ");
            putln(desc);
            puts("     Loaded: loaded (/etc/services.d/");
            puts(name);
            puts(".service; ");
            puts(en);
            puts("; type=");
            puts(typ);
            putln(")");
            puts("     Active: ");
            puts(active);
            puts(" (");
            puts(sub);
            puts(")  Restarts: ");
            putln(restarts);
            if pid != "0" {
                puts("   Main PID: ");
                putln(pid);
            }
            show_unit_log(name);
            return if active == "active" { 0 } else { 3 };
        }
    }

    let mut pbuf = [0u8; 64];
    let upath = build_path(b"/etc/services.d/", unit, b".service", &mut pbuf);
    let mut fbuf = [0u8; 512];
    if !read_file(upath, &mut fbuf).is_empty() {
        puts("* ");
        puts(unit);
        putln(".service");
        puts("     Loaded: loaded (");
        puts(upath);
        putln(")");
        putln("     Active: inactive (dead)");
        return 3;
    }

    puts("Unit not found: ");
    putln(unit);
    4
}

fn set_enabled(raw_unit: &str, enable: bool) -> i32 {
    let unit = strip_svc(raw_unit);
    let mut cbuf = [0u8; 512];
    let conf = read_file("/etc/services.conf", &mut cbuf);
    let fd = open("/etc/services.conf.tmp", O_WRONLY | O_CREATE | O_TRUNC);
    if fd < 0 {
        putln("systemctl: cannot modify /etc/services.conf");
        return 1;
    }
    let mut found = false;
    for line in conf.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if !t.starts_with('#') && strip_svc(t) == unit {
            found = true;
            if !enable {
                continue;
            }
        }
        let _ = write(fd as i32, t.as_bytes());
        let _ = write(fd as i32, b"\n");
    }
    if enable && !found {
        let _ = write(fd as i32, unit.as_bytes());
        let _ = write(fd as i32, b"\n");
    }
    close(fd as i32);
    let _ = rename("/etc/services.conf.tmp", "/etc/services.conf");
    let _ = send_ctl("reload", "");
    puts("systemctl: ");
    puts(unit);
    putln(if enable { ".service enabled" } else { ".service disabled" });
    0
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc <= 1 {
        return list_units();
    }

    let cmd = unsafe { arg(argv, 1) };
    if cmd == "-h" || cmd == "--help" {
        putln("Usage: systemctl [list-units | list-unit-files | status <u> | start <u> | stop <u> | restart <u> | enable <u> | disable <u> | is-active <u> | is-enabled <u> | daemon-reload | journal [-u <u>]]");
        return 0;
    }

    match cmd {
        "list-units" | "list" => list_units(),
        "list-unit-files" => list_unit_files(),
        "daemon-reload" | "reload" => {
            let _ = send_ctl("reload", "");
            0
        }
        "status" => {
            if argc <= 2 { list_units() } else { show_status(unsafe { arg(argv, 2) }) }
        }
        "start" | "stop" | "restart" => {
            if argc < 3 {
                puts("usage: systemctl ");
                puts(cmd);
                putln(" <unit>");
                return 1;
            }
            let unit = strip_svc(unsafe { arg(argv, 2) });
            if send_ctl(cmd, unit) { 0 } else { fallback_direct_action(cmd, unit) }
        }
        "enable" | "disable" => {
            if argc < 3 {
                putln("usage: systemctl enable|disable <unit>");
                return 1;
            }
            set_enabled(unsafe { arg(argv, 2) }, cmd == "enable")
        }
        "is-active" => {
            if argc < 3 {
                return 3;
            }
            let unit = strip_svc(unsafe { arg(argv, 2) });
            let mut sbuf = [0u8; 1024];
            for line in read_file("/run/systemd/units.state", &mut sbuf).lines() {
                let mut p = line.split('|');
                if p.next() == Some(unit) {
                    let _ = p.next();
                    let _ = p.next();
                    let active = p.next().unwrap_or("inactive");
                    putln(active);
                    return if active == "active" { 0 } else { 3 };
                }
            }
            putln("inactive");
            3
        }
        "is-enabled" => {
            if argc < 3 {
                return 1;
            }
            let unit = strip_svc(unsafe { arg(argv, 2) });
            let mut cbuf = [0u8; 512];
            for l in read_file("/etc/services.conf", &mut cbuf).lines() {
                if !l.trim().starts_with('#') && strip_svc(l) == unit {
                    putln("enabled");
                    return 0;
                }
            }
            putln("disabled");
            1
        }
        "journal" => {
            if argc >= 4 && unsafe { arg(argv, 2) } == "-u" {
                show_unit_log(strip_svc(unsafe { arg(argv, 3) }));
            } else {
                let mut kbuf = [0u8; 1024];
                puts(read_file("/proc/dmesg", &mut kbuf));
            }
            0
        }
        _ => {
            puts("systemctl: unknown command: ");
            putln(cmd);
            1
        }
    }
}
