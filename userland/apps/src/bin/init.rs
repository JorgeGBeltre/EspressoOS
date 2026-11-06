#![no_std]
#![no_main]

use libc::{
    close, dup2, kill, kill_pgid, mkdir, open, println, read, readdir, setpgid, sleep_ms, spawn,
    unlink, uptime_ms, waitpid, write, yield_now, O_APPEND, O_CREATE, O_RDONLY, O_TRUNC, O_WRONLY,
    WNOHANG,
};

const MAX_UNITS: usize = 12;
const MAX_DEPS: usize = 3;

const TYPE_SIMPLE: u8 = 0;
const TYPE_ONESHOT: u8 = 1;

const RESTART_NO: u8 = 0;
const RESTART_ALWAYS: u8 = 1;
const RESTART_ON_FAILURE: u8 = 2;

const STATE_INACTIVE: u8 = 0;
const STATE_ACTIVE: u8 = 1;
const STATE_EXITED: u8 = 2;
const STATE_FAILED: u8 = 3;
const STATE_BACKOFF: u8 = 4;

#[derive(Clone, Copy)]
struct Unit {
    used: bool,
    name: [u8; 20],
    nlen: u8,
    desc: [u8; 48],
    dlen: u8,
    exec: [u8; 64],
    elen: u8,
    deps: [[u8; 20]; MAX_DEPS],
    dep_len: [u8; MAX_DEPS],
    dep_count: u8,
    utype: u8,
    restart: u8,
    restart_ms: u32,
    enabled: bool,
    state: u8,
    pid: i32,
    exit_code: i32,
    restarts: u16,
    next_start_ms: usize,
}

impl Unit {
    const fn empty() -> Self {
        Self {
            used: false,
            name: [0; 20],
            nlen: 0,
            desc: [0; 48],
            dlen: 0,
            exec: [0; 64],
            elen: 0,
            deps: [[0; 20]; MAX_DEPS],
            dep_len: [0; MAX_DEPS],
            dep_count: 0,
            utype: TYPE_SIMPLE,
            restart: RESTART_NO,
            restart_ms: 2000,
            enabled: false,
            state: STATE_INACTIVE,
            pid: -1,
            exit_code: 0,
            restarts: 0,
            next_start_ms: 0,
        }
    }

    fn name_str(&self) -> &str {
        core::str::from_utf8(&self.name[..self.nlen as usize]).unwrap_or("")
    }

    fn desc_str(&self) -> &str {
        core::str::from_utf8(&self.desc[..self.dlen as usize]).unwrap_or("")
    }
}

static mut UNITS: [Unit; MAX_UNITS] = [Unit::empty(); MAX_UNITS];
static mut DIR_BUF: [u8; 1024] = [0; 1024];

fn klog(msg: &str) {
    let fd = open("/dev/log", O_WRONLY);
    if fd >= 0 {
        let _ = write(fd as i32, b"[init] ");
        let _ = write(fd as i32, msg.as_bytes());
        let _ = write(fd as i32, b"\n");
        close(fd as i32);
    }
}

fn strip_service_suffix(s: &str) -> &str {
    let t = s.trim();
    if let Some(stripped) = t.strip_suffix(".service") {
        stripped
    } else {
        t
    }
}

fn parse_u32(s: &str) -> Option<u32> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    let mut v = 0u32;
    for &b in t.as_bytes() {
        if b >= b'0' && b <= b'9' {
            v = v.saturating_mul(10).saturating_add((b - b'0') as u32);
        } else {
            break;
        }
    }
    Some(v)
}

fn fmt_u32<'a>(mut v: u32, buf: &'a mut [u8; 16]) -> &'a [u8] {
    if v == 0 {
        buf[0] = b'0';
        return &buf[..1];
    }
    let mut tmp = [0u8; 16];
    let mut k = 0;
    while v > 0 {
        tmp[k] = b'0' + (v % 10) as u8;
        v /= 10;
        k += 1;
    }
    for i in 0..k {
        buf[i] = tmp[k - 1 - i];
    }
    &buf[..k]
}

fn write_pid_file(name: &str, pid: i32) {
    let mut pbuf = [0u8; 48];
    let prefix = b"/run/";
    let suffix = b".pid";
    let nb = name.as_bytes();
    if prefix.len() + nb.len() + suffix.len() >= pbuf.len() {
        return;
    }
    let mut k = 0;
    pbuf[..prefix.len()].copy_from_slice(prefix);
    k += prefix.len();
    pbuf[k..k + nb.len()].copy_from_slice(nb);
    k += nb.len();
    pbuf[k..k + suffix.len()].copy_from_slice(suffix);
    k += suffix.len();
    if let Ok(path) = core::str::from_utf8(&pbuf[..k]) {
        let fd = open(path, O_WRONLY | O_CREATE | O_TRUNC);
        if fd >= 0 {
            let mut ibuf = [0u8; 16];
            let b = fmt_u32(pid as u32, &mut ibuf);
            let _ = write(fd as i32, b);
            let _ = write(fd as i32, b"\n");
            close(fd as i32);
        }
    }
}

fn remove_pid_file(name: &str) {
    let mut pbuf = [0u8; 48];
    let prefix = b"/run/";
    let suffix = b".pid";
    let nb = name.as_bytes();
    if prefix.len() + nb.len() + suffix.len() >= pbuf.len() {
        return;
    }
    let mut k = 0;
    pbuf[..prefix.len()].copy_from_slice(prefix);
    k += prefix.len();
    pbuf[k..k + nb.len()].copy_from_slice(nb);
    k += nb.len();
    pbuf[k..k + suffix.len()].copy_from_slice(suffix);
    k += suffix.len();
    if let Ok(path) = core::str::from_utf8(&pbuf[..k]) {
        let _ = unlink(path);
    }
}

fn add_dep(u: &mut Unit, raw_list: &str) {
    for tok in raw_list.split_whitespace() {
        let dep = strip_service_suffix(tok);
        if dep.is_empty() || dep == u.name_str() {
            continue;
        }
        let mut dup = false;
        for d in 0..u.dep_count as usize {
            let existing = core::str::from_utf8(&u.deps[d][..u.dep_len[d] as usize]).unwrap_or("");
            if existing == dep {
                dup = true;
                break;
            }
        }
        if !dup && (u.dep_count as usize) < MAX_DEPS {
            let idx = u.dep_count as usize;
            let c = dep.len().min(20);
            u.deps[idx][..c].copy_from_slice(&dep.as_bytes()[..c]);
            u.dep_len[idx] = c as u8;
            u.dep_count += 1;
        }
    }
}

fn find_unit_idx(units: &[Unit; MAX_UNITS], name: &str) -> Option<usize> {
    let clean = strip_service_suffix(name);
    for (i, u) in units.iter().enumerate() {
        if u.used && u.name_str() == clean {
            return Some(i);
        }
    }
    None
}

fn load_units() {
    let units = unsafe { &mut *core::ptr::addr_of_mut!(UNITS) };
    let dir_buf = unsafe { &mut *core::ptr::addr_of_mut!(DIR_BUF) };

    // Read enabled list from /etc/services.conf
    let mut conf_buf = [0u8; 512];
    let mut conf_len = 0usize;
    let cfd = open("/etc/services.conf", O_RDONLY);
    if cfd >= 0 {
        let n = read(cfd as i32, &mut conf_buf);
        close(cfd as i32);
        if n > 0 {
            conf_len = n as usize;
        }
    }
    let conf_text = core::str::from_utf8(&conf_buf[..conf_len]).unwrap_or("");

    let n = readdir("/etc/services.d", dir_buf);
    if n <= 0 {
        return;
    }
    let limit = n as usize;
    let mut pos = 0usize;
    while pos + 11 <= limit {
        let nlen = u16::from_le_bytes([dir_buf[pos + 9], dir_buf[pos + 10]]) as usize;
        pos += 11;
        if pos + nlen > limit {
            break;
        }
        if let Ok(fname) = core::str::from_utf8(&dir_buf[pos..pos + nlen]) {
            if let Some(uname) = fname.strip_suffix(".service") {
                let mut path_buf = [0u8; 64];
                let pfx = b"/etc/services.d/";
                let fb = fname.as_bytes();
                if pfx.len() + fb.len() < path_buf.len() {
                    path_buf[..pfx.len()].copy_from_slice(pfx);
                    path_buf[pfx.len()..pfx.len() + fb.len()].copy_from_slice(fb);
                    if let Ok(full) = core::str::from_utf8(&path_buf[..pfx.len() + fb.len()]) {
                        parse_unit_file(units, uname, full, conf_text);
                    }
                }
            }
        }
        pos += nlen;
    }
}

fn parse_unit_file(units: &mut [Unit; MAX_UNITS], uname: &str, full_path: &str, conf_text: &str) {
    let fd = open(full_path, O_RDONLY);
    if fd < 0 {
        return;
    }
    let mut fbuf = [0u8; 512];
    let n = read(fd as i32, &mut fbuf);
    close(fd as i32);
    if n <= 0 {
        return;
    }
    let text = match core::str::from_utf8(&fbuf[..n as usize]) {
        Ok(s) => s,
        Err(_) => return,
    };

    let slot_idx = match find_unit_idx(units, uname) {
        Some(idx) => idx,
        None => {
            let mut free = None;
            for (i, u) in units.iter().enumerate() {
                if !u.used {
                    free = Some(i);
                    break;
                }
            }
            match free {
                Some(i) => i,
                None => return,
            }
        }
    };

    let prev_pid = units[slot_idx].pid;
    let prev_state = units[slot_idx].state;
    let prev_restarts = units[slot_idx].restarts;

    let mut u = Unit::empty();
    u.used = true;
    let un_c = uname.len().min(20);
    u.name[..un_c].copy_from_slice(&uname.as_bytes()[..un_c]);
    u.nlen = un_c as u8;

    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') || t.starts_with('[') {
            continue;
        }
        if let Some(eq) = t.find('=') {
            let key = t[..eq].trim();
            let val = t[eq + 1..].trim();
            if key == "Description" {
                let c = val.len().min(48);
                u.desc[..c].copy_from_slice(&val.as_bytes()[..c]);
                u.dlen = c as u8;
            } else if key == "ExecStart" {
                let c = val.len().min(64);
                u.exec[..c].copy_from_slice(&val.as_bytes()[..c]);
                u.elen = c as u8;
            } else if key == "Type" {
                u.utype = if val == "oneshot" { TYPE_ONESHOT } else { TYPE_SIMPLE };
            } else if key == "Restart" {
                u.restart = match val {
                    "always" => RESTART_ALWAYS,
                    "on-failure" => RESTART_ON_FAILURE,
                    _ => RESTART_NO,
                };
            } else if key == "RestartSec" {
                if let Some(sec) = parse_u32(val) {
                    u.restart_ms = sec.saturating_mul(1000).max(200);
                }
            } else if key == "After" || key == "Requires" {
                add_dep(&mut u, val);
            }
        }
    }

    for cline in conf_text.lines() {
        let ct = strip_service_suffix(cline.trim());
        if !ct.is_empty() && !ct.starts_with('#') && ct == uname {
            u.enabled = true;
            break;
        }
    }

    if prev_pid > 0 {
        u.pid = prev_pid;
        u.state = prev_state;
        u.restarts = prev_restarts;
    }
    units[slot_idx] = u;
}

/// Kahn's topological sort over `UNITS`. Returns ordered unit indices; breaks cycles deterministically.
fn topo_order(units: &[Unit; MAX_UNITS], out: &mut [usize; MAX_UNITS]) -> usize {
    let mut in_deg = [0u8; MAX_UNITS];
    let mut visited = [false; MAX_UNITS];
    let mut total = 0usize;

    for i in 0..MAX_UNITS {
        if !units[i].used {
            visited[i] = true;
            continue;
        }
        total += 1;
        let mut deg = 0u8;
        for d in 0..units[i].dep_count as usize {
            let dname = core::str::from_utf8(&units[i].deps[d][..units[i].dep_len[d] as usize]).unwrap_or("");
            if find_unit_idx(units, dname).is_some() {
                deg += 1;
            }
        }
        in_deg[i] = deg;
    }

    let mut count = 0usize;
    while count < total {
        let mut pick = None;
        for i in 0..MAX_UNITS {
            if !visited[i] && in_deg[i] == 0 {
                pick = Some(i);
                break;
            }
        }
        let u_idx = match pick {
            Some(idx) => idx,
            None => {
                klog("dependency cycle detected in /etc/services.d; breaking cycle");
                let mut fallback = 0;
                for i in 0..MAX_UNITS {
                    if !visited[i] {
                        fallback = i;
                        break;
                    }
                }
                fallback
            }
        };

        visited[u_idx] = true;
        out[count] = u_idx;
        count += 1;

        let uname = units[u_idx].name_str();
        for j in 0..MAX_UNITS {
            if !visited[j] {
                for d in 0..units[j].dep_count as usize {
                    let dn = core::str::from_utf8(&units[j].deps[d][..units[j].dep_len[d] as usize]).unwrap_or("");
                    if dn == uname && in_deg[j] > 0 {
                        in_deg[j] -= 1;
                    }
                }
            }
        }
    }
    count
}

fn state_name(st: u8) -> &'static str {
    match st {
        STATE_ACTIVE => "running",
        STATE_EXITED => "exited",
        STATE_FAILED => "failed",
        STATE_BACKOFF => "auto-restart",
        _ => "dead",
    }
}

fn write_state_snapshot(units: &[Unit; MAX_UNITS]) {
    let fd = open("/run/systemd/units.state.tmp", O_WRONLY | O_CREATE | O_TRUNC);
    if fd < 0 {
        return;
    }
    let fd = fd as i32;
    for u in units.iter() {
        if !u.used {
            continue;
        }
        let _ = write(fd, u.name_str().as_bytes());
        let _ = write(fd, b"|");
        let _ = write(fd, if u.utype == TYPE_ONESHOT { b"oneshot" } else { b"simple" });
        let _ = write(fd, b"|");
        let _ = write(fd, if u.enabled { b"enabled" } else { b"disabled" });
        let _ = write(fd, b"|");
        let active_col: &[u8] = match u.state {
            STATE_ACTIVE | STATE_EXITED => b"active",
            STATE_FAILED => b"failed",
            STATE_BACKOFF => b"activating",
            _ => b"inactive",
        };
        let _ = write(fd, active_col);
        let _ = write(fd, b"|");
        let _ = write(fd, state_name(u.state).as_bytes());
        let _ = write(fd, b"|");
        let mut nbuf = [0u8; 16];
        let pid_val = if u.pid > 0 { u.pid as u32 } else { 0 };
        let _ = write(fd, fmt_u32(pid_val, &mut nbuf));
        let _ = write(fd, b"|");
        let _ = write(fd, fmt_u32(u.restarts as u32, &mut nbuf));
        let _ = write(fd, b"|");
        let _ = write(fd, u.desc_str().as_bytes());
        let _ = write(fd, b"\n");
    }
    close(fd);
    let _ = libc::rename("/run/systemd/units.state.tmp", "/run/systemd/units.state");
}

fn start_unit(units: &mut [Unit; MAX_UNITS], idx: usize) {
    if !units[idx].used || units[idx].elen == 0 {
        return;
    }
    if units[idx].pid > 0 && units[idx].state == STATE_ACTIVE {
        return;
    }

    let mut cmd_copy = [0u8; 65];
    let elen = units[idx].elen as usize;
    cmd_copy[..elen].copy_from_slice(&units[idx].exec[..elen]);
    cmd_copy[elen] = 0;

    let mut argv = [core::ptr::null(); 9];
    let mut argc = 0usize;
    let mut r = 0usize;
    while r < elen && argc < 8 {
        while r < elen && cmd_copy[r] == b' ' {
            r += 1;
        }
        if r >= elen {
            break;
        }
        let tok_start = r;
        while r < elen && cmd_copy[r] != b' ' {
            r += 1;
        }
        cmd_copy[r] = 0;
        argv[argc] = unsafe { cmd_copy.as_ptr().add(tok_start) };
        argc += 1;
        r += 1;
    }
    argv[argc] = core::ptr::null();
    if argc == 0 {
        return;
    }

    // Extract cmd0 length
    let mut c0_end = 0usize;
    while c0_end < elen && cmd_copy[c0_end] != 0 {
        c0_end += 1;
    }
    let cmd0 = core::str::from_utf8(&cmd_copy[..c0_end]).unwrap_or("");
    let mut path_buf = [0u8; 64];
    let bin_path = if cmd0.starts_with('/') {
        cmd0
    } else {
        let p = b"/bin/";
        let cb = cmd0.as_bytes();
        let tot = (p.len() + cb.len()).min(path_buf.len());
        path_buf[..p.len()].copy_from_slice(p);
        path_buf[p.len()..tot].copy_from_slice(&cb[..tot - p.len()]);
        core::str::from_utf8(&path_buf[..tot]).unwrap_or(cmd0)
    };

    // Redirect daemon stdio: stdin -> /dev/null, stdout/stderr -> /var/log/<unit>.log
    let mut lpath_buf = [0u8; 48];
    let lpfx = b"/var/log/";
    let lsfx = b".log";
    let un = units[idx].name_str().as_bytes();
    let mut lk = 0;
    lpath_buf[..lpfx.len()].copy_from_slice(lpfx);
    lk += lpfx.len();
    let uc = un.len().min(28);
    lpath_buf[lk..lk + uc].copy_from_slice(&un[..uc]);
    lk += uc;
    lpath_buf[lk..lk + lsfx.len()].copy_from_slice(lsfx);
    lk += lsfx.len();
    let log_path = core::str::from_utf8(&lpath_buf[..lk]).unwrap_or("/var/log/daemon.log");

    let null_fd = open("/dev/null", O_RDONLY);
    let log_fd = open(log_path, O_WRONLY | O_CREATE | O_APPEND);

    let saved_in = dup2(0, 11) as i32;
    let saved_out = dup2(1, 12) as i32;
    let saved_err = dup2(2, 13) as i32;

    if null_fd >= 0 {
        let _ = dup2(null_fd as i32, 0);
        close(null_fd as i32);
    }
    if log_fd >= 0 {
        let _ = dup2(log_fd as i32, 1);
        let _ = dup2(log_fd as i32, 2);
        close(log_fd as i32);
    }

    let pid = spawn(bin_path, argv.as_ptr());

    if saved_in >= 0 {
        let _ = dup2(saved_in, 0);
        close(saved_in);
    }
    if saved_out >= 0 {
        let _ = dup2(saved_out, 1);
        close(saved_out);
    }
    if saved_err >= 0 {
        let _ = dup2(saved_err, 2);
        close(saved_err);
    }

    if pid < 0 {
        units[idx].state = STATE_FAILED;
        units[idx].pid = -1;
        return;
    }

    if units[idx].utype == TYPE_ONESHOT {
        let mut st = 0;
        let _ = waitpid(pid as i32, &mut st, 0);
        units[idx].pid = -1;
        units[idx].exit_code = st;
        units[idx].state = if st == 0 { STATE_EXITED } else { STATE_FAILED };
    } else {
        let upid = pid as u32;
        let _ = setpgid(upid, upid);
        units[idx].pid = pid as i32;
        units[idx].state = STATE_ACTIVE;
        write_pid_file(units[idx].name_str(), pid as i32);
    }
}

fn stop_unit(units: &mut [Unit; MAX_UNITS], idx: usize) {
    if !units[idx].used {
        return;
    }
    if units[idx].pid > 0 {
        let upid = units[idx].pid as u32;
        let _ = kill_pgid(upid, 15);
        let _ = kill(upid, 15);
        sleep_ms(50);
        let mut st = 0;
        if waitpid(units[idx].pid, &mut st, WNOHANG) <= 0 {
            let _ = kill_pgid(upid, 9);
            let _ = kill(upid, 9);
            let _ = waitpid(units[idx].pid, &mut st, WNOHANG);
        }
    }
    remove_pid_file(units[idx].name_str());
    units[idx].pid = -1;
    units[idx].state = STATE_INACTIVE;
    units[idx].restarts = 0;
    units[idx].next_start_ms = 0;
}

fn process_ctl(units: &mut [Unit; MAX_UNITS]) -> bool {
    let fd = open("/run/init.ctl", O_RDONLY);
    if fd < 0 {
        return false;
    }
    let mut cbuf = [0u8; 256];
    let n = read(fd as i32, &mut cbuf);
    close(fd as i32);
    let _ = unlink("/run/init.ctl");
    if n <= 0 {
        return true;
    }
    if let Ok(text) = core::str::from_utf8(&cbuf[..n as usize]) {
        for line in text.lines() {
            let mut parts = line.trim().split_whitespace();
            let action = parts.next().unwrap_or("");
            let target = parts.next().unwrap_or("");
            match action {
                "reload" => {
                    load_units();
                }
                "start" => {
                    if let Some(idx) = find_unit_idx(units, target) {
                        units[idx].restarts = 0;
                        start_unit(units, idx);
                    }
                }
                "stop" => {
                    if let Some(idx) = find_unit_idx(units, target) {
                        stop_unit(units, idx);
                    }
                }
                "restart" => {
                    if let Some(idx) = find_unit_idx(units, target) {
                        stop_unit(units, idx);
                        start_unit(units, idx);
                    }
                }
                _ => {}
            }
        }
    }
    true
}

#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    let _ = mkdir("/run/systemd");
    let _ = mkdir("/var/log");
    println!("[init] Micro-Systemd supervisor up");
    klog("Micro-Systemd supervisor starting /etc/rc");

    let rc_argv = [b"sh\0".as_ptr(), b"/etc/rc\0".as_ptr(), core::ptr::null()];
    let rc_pid = spawn("/bin/sh", rc_argv.as_ptr());
    if rc_pid >= 0 {
        let mut rc_st = 0;
        let _ = waitpid(rc_pid as i32, &mut rc_st, 0);
    } else {
        println!("[init] warning: failed to spawn /etc/rc ({})", rc_pid);
    }

    load_units();
    let units = unsafe { &mut *core::ptr::addr_of_mut!(UNITS) };
    let mut order = [0usize; MAX_UNITS];
    let n_ord = topo_order(units, &mut order);
    for k in 0..n_ord {
        let idx = order[k];
        if units[idx].enabled {
            start_unit(units, idx);
        }
    }
    write_state_snapshot(units);

    let sh_argv = [b"sh\0".as_ptr(), core::ptr::null()];
    let mut shell_pid: i32 = -1;
    let mut sh_failures = 0u32;

    loop {
        if shell_pid < 0 {
            let p = spawn("/bin/sh", sh_argv.as_ptr());
            if p >= 0 {
                shell_pid = p as i32;
                sh_failures = 0;
            } else {
                sh_failures += 1;
                println!("[init] ERROR spawning /bin/sh: {}", p);
                if sh_failures >= 3 {
                    println!("[init] /bin/sh will not start; exiting so supervisor falls back");
                    return 1;
                }
                for _ in 0..50_000 {
                    yield_now();
                }
            }
        }

        let mut dirty = false;
        loop {
            let mut st = 0;
            let reaped = waitpid(-1, &mut st, WNOHANG);
            if reaped <= 0 {
                break;
            }
            let rpid = reaped as i32;
            if rpid == shell_pid {
                shell_pid = -1;
                continue;
            }
            for u in units.iter_mut() {
                if u.used && u.pid == rpid {
                    remove_pid_file(u.name_str());
                    u.pid = -1;
                    u.exit_code = st;
                    let want_restart = u.restart == RESTART_ALWAYS
                        || (u.restart == RESTART_ON_FAILURE && st != 0);
                    if want_restart {
                        let shift = (u.restarts as u32).min(4);
                        let delay = u.restart_ms.saturating_mul(1u32 << shift).min(30_000) as usize;
                        u.restarts = u.restarts.saturating_add(1);
                        u.state = STATE_BACKOFF;
                        u.next_start_ms = uptime_ms().saturating_add(delay);
                    } else {
                        u.state = if st == 0 { STATE_INACTIVE } else { STATE_FAILED };
                    }
                    dirty = true;
                    break;
                }
            }
        }

        let now = uptime_ms();
        for idx in 0..MAX_UNITS {
            if units[idx].used && units[idx].state == STATE_BACKOFF && now >= units[idx].next_start_ms {
                start_unit(units, idx);
                dirty = true;
            }
        }

        if process_ctl(units) {
            dirty = true;
        }

        if dirty {
            write_state_snapshot(units);
        }

        if shell_pid >= 0 {
            sleep_ms(40);
        }
    }
}
