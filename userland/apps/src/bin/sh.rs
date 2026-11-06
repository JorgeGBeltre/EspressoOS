#![no_std]
#![no_main]

use libc::{
    arg, chdir, close, dup2, getcwd, getpid, open, pipe, read, readdir, setpgid, spawn, tcsetpgrp,
    uptime_ms, waitpid, write, yield_now, O_APPEND, O_CREATE, O_RDONLY, O_TRUNC, O_WRONLY,
    PATH_MAX, WNOHANG,
};

const MAX_ARGV: usize = 20;
const LINE: usize = 256;
const MAX_STAGES: usize = 8;
const SCRIPT_MAX: usize = 2048;
const HISTORY_MAX: usize = 12;
const ENV_MAX: usize = 16;
const KEY_MAX: usize = 20;
const VAL_MAX: usize = 48;

static mut USER_BUF: [u8; 32] = [0; 32];
static mut USER_LEN: usize = 0;
static mut LAST_STATUS: i32 = 0;

static mut SCRIPT_BUF: [u8; SCRIPT_MAX] = [0; SCRIPT_MAX];
static mut DIR_BUF: [u8; 3584] = [0; 3584];
static mut EXP_BUF: [u8; LINE + 64] = [0; LINE + 64];

static mut HISTORY_BUF: [[u8; LINE]; HISTORY_MAX] = [[0; LINE]; HISTORY_MAX];
static mut HISTORY_LENS: [usize; HISTORY_MAX] = [0; HISTORY_MAX];
static mut HISTORY_COUNT: usize = 0;

static mut ENV_KEYS: [[u8; KEY_MAX]; ENV_MAX] = [[0; KEY_MAX]; ENV_MAX];
static mut ENV_KLEN: [usize; ENV_MAX] = [0; ENV_MAX];
static mut ENV_VALS: [[u8; VAL_MAX]; ENV_MAX] = [[0; VAL_MAX]; ENV_MAX];
static mut ENV_VLEN: [usize; ENV_MAX] = [0; ENV_MAX];

#[inline(never)]
fn puts(s: &[u8]) {
    if !s.is_empty() {
        let _ = write(1, s);
    }
}

#[inline(never)]
fn putln(s: &[u8]) {
    puts(s);
    let _ = write(1, b"\n");
}

#[inline(never)]
fn fmt_usize(mut n: usize, buf: &mut [u8; 16]) -> &[u8] {
    if n == 0 {
        buf[0] = b'0';
        return &buf[..1];
    }
    let mut tmp = [0u8; 16];
    let mut k = 0;
    while n > 0 {
        tmp[k] = b'0' + (n % 10) as u8;
        n /= 10;
        k += 1;
    }
    for i in 0..k {
        buf[i] = tmp[k - 1 - i];
    }
    &buf[..k]
}

#[inline(never)]
fn fmt_i32(v: i32, buf: &mut [u8; 16]) -> &[u8] {
    if v < 0 {
        let mut ubuf = [0u8; 16];
        let ub = fmt_usize((-v) as usize, &mut ubuf);
        buf[0] = b'-';
        buf[1..1 + ub.len()].copy_from_slice(ub);
        &buf[..1 + ub.len()]
    } else {
        fmt_usize(v as usize, buf)
    }
}

fn put_i32(v: i32) {
    let mut b = [0u8; 16];
    puts(fmt_i32(v, &mut b));
}

#[inline(never)]
fn ascii_str(b: &[u8]) -> Option<&str> {
    for &c in b {
        if c >= 0x80 {
            return None;
        }
    }
    Some(unsafe { core::str::from_utf8_unchecked(b) })
}

#[inline(never)]
fn reap_background_jobs() {
    let mut status = 0;
    while waitpid(-1, &mut status, WNOHANG) > 0 {}
}

fn set_status(st: i32) {
    unsafe { LAST_STATUS = st; }
}

fn get_status() -> i32 {
    unsafe { LAST_STATUS }
}

fn trim(buf: &[u8], mut s: usize, mut e: usize) -> (usize, usize) {
    if e > buf.len() {
        e = buf.len();
    }
    while s < e && (buf[s] == b' ' || buf[s] == b'\t') {
        s += 1;
    }
    while e > s && (buf[e - 1] == b' ' || buf[e - 1] == b'\t') {
        e -= 1;
    }
    (s, e)
}

fn is_var_char(b: u8) -> bool {
    (b >= b'a' && b <= b'z') || (b >= b'A' && b <= b'Z') || (b >= b'0' && b <= b'9') || b == b'_'
}

fn env_set(key: &[u8], val: &[u8]) {
    if key.is_empty() || key.len() > KEY_MAX {
        return;
    }
    let vlen = val.len().min(VAL_MAX);
    unsafe {
        let mut free_slot = None;
        for i in 0..ENV_MAX {
            if ENV_KLEN[i] == key.len() && &ENV_KEYS[i][..key.len()] == key {
                ENV_VALS[i][..vlen].copy_from_slice(&val[..vlen]);
                ENV_VLEN[i] = vlen;
                return;
            }
            if ENV_KLEN[i] == 0 && free_slot.is_none() {
                free_slot = Some(i);
            }
        }
        if let Some(i) = free_slot {
            ENV_KEYS[i][..key.len()].copy_from_slice(key);
            ENV_KLEN[i] = key.len();
            ENV_VALS[i][..vlen].copy_from_slice(&val[..vlen]);
            ENV_VLEN[i] = vlen;
        }
    }
}

fn env_unset(key: &[u8]) {
    unsafe {
        for i in 0..ENV_MAX {
            if ENV_KLEN[i] == key.len() && &ENV_KEYS[i][..key.len()] == key {
                ENV_KLEN[i] = 0;
                ENV_VLEN[i] = 0;
            }
        }
    }
}

fn env_get<'a>(key: &[u8], out: &'a mut [u8; PATH_MAX]) -> Option<&'a [u8]> {
    if key == b"PWD" {
        let n = getcwd(out);
        if n >= 0 {
            return Some(&out[..n as usize]);
        }
        return None;
    }
    unsafe {
        for i in 0..ENV_MAX {
            if ENV_KLEN[i] == key.len() && &ENV_KEYS[i][..key.len()] == key {
                let vl = ENV_VLEN[i];
                out[..vl].copy_from_slice(&ENV_VALS[i][..vl]);
                return Some(&out[..vl]);
            }
        }
    }
    None
}

fn sync_env_file() {
    let fd = open("/run/sh.env", O_WRONLY | O_CREATE | O_TRUNC);
    if fd < 0 {
        return;
    }
    let fd = fd as i32;
    unsafe {
        for i in 0..ENV_MAX {
            let kl = ENV_KLEN[i];
            if kl > 0 {
                let _ = write(fd, &ENV_KEYS[i][..kl]);
                let _ = write(fd, b"=");
                let _ = write(fd, &ENV_VALS[i][..ENV_VLEN[i]]);
                let _ = write(fd, b"\n");
            }
        }
    }
    close(fd);
}

fn parse_env_blob(blob: &[u8], require_export_prefix: bool) {
    let mut start = 0usize;
    let n = blob.len();
    while start < n {
        let mut end = start;
        while end < n && blob[end] != b'\n' {
            end += 1;
        }
        let mut e = end;
        if e > start && blob[e - 1] == b'\r' {
            e -= 1;
        }
        let (ts, te) = trim(blob, start, e);
        if ts < te && blob[ts] != b'#' {
            let line = &blob[ts..te];
            let assign = if require_export_prefix {
                if line.starts_with(b"export ") {
                    &line[7..]
                } else {
                    &[][..]
                }
            } else {
                line
            };
            if let Some(eq) = assign.iter().position(|&b| b == b'=') {
                let k = &assign[..eq];
                let mut v = &assign[eq + 1..];
                if v.len() >= 2 && (v[0] == b'"' || v[0] == b'\'') && v[v.len() - 1] == v[0] {
                    v = &v[1..v.len() - 1];
                }
                env_set(k, v);
            }
        }
        start = end + 1;
    }
}

fn init_env() {
    env_set(b"OS", b"EspressoOS");
    env_set(b"ARCH", b"xtensa-lx7");
    env_set(b"SHELL", b"/bin/sh");
    env_set(b"PATH", b"/bin");
    env_set(b"HOME", b"/root");
    env_set(b"USER", b"root");
    env_set(b"TERM", b"xterm-256color");

    unsafe {
        if USER_LEN > 0 {
            let u = &(&*core::ptr::addr_of!(USER_BUF))[..USER_LEN];
            env_set(b"USER", u);
        }
    }

    let mut ebuf = [0u8; 512];
    let fd = open("/run/sh.env", O_RDONLY);
    if fd >= 0 {
        let n = read(fd as i32, &mut ebuf);
        close(fd as i32);
        if n > 0 {
            parse_env_blob(&ebuf[..n as usize], false);
        }
    } else {
        let pfd = open("/etc/profile", O_RDONLY);
        if pfd >= 0 {
            let n = read(pfd as i32, &mut ebuf);
            close(pfd as i32);
            if n > 0 {
                parse_env_blob(&ebuf[..n as usize], true);
            }
        }
    }
    sync_env_file();
}

fn push_history_mem(entry: &[u8]) {
    if entry.is_empty() || entry.len() > LINE {
        return;
    }
    unsafe {
        if HISTORY_COUNT > 0 {
            let prev = (HISTORY_COUNT - 1) % HISTORY_MAX;
            if HISTORY_LENS[prev] == entry.len() && &HISTORY_BUF[prev][..entry.len()] == entry {
                return;
            }
        }
        let slot = HISTORY_COUNT % HISTORY_MAX;
        HISTORY_BUF[slot][..entry.len()].copy_from_slice(entry);
        HISTORY_LENS[slot] = entry.len();
        HISTORY_COUNT += 1;
    }
}

fn load_history() {
    let mut fd = open("/root/.sh_history", O_RDONLY);
    if fd < 0 {
        fd = open("/tmp/.sh_history", O_RDONLY);
    }
    if fd < 0 {
        return;
    }
    let mut hbuf = [0u8; 768];
    let n = read(fd as i32, &mut hbuf);
    close(fd as i32);
    if n <= 0 {
        return;
    }
    let limit = n as usize;
    let mut s = 0usize;
    while s < limit {
        let mut e = s;
        while e < limit && hbuf[e] != b'\n' {
            e += 1;
        }
        let mut le = e;
        if le > s && hbuf[le - 1] == b'\r' {
            le -= 1;
        }
        if le > s {
            push_history_mem(&hbuf[s..le]);
        }
        s = e + 1;
    }
}

fn record_history(entry: &[u8]) {
    let (s, e) = trim(entry, 0, entry.len());
    if s >= e {
        return;
    }
    let slice = &entry[s..e];
    push_history_mem(slice);
    let mut fd = open("/root/.sh_history", O_WRONLY | O_CREATE | O_APPEND);
    if fd < 0 {
        fd = open("/tmp/.sh_history", O_WRONLY | O_CREATE | O_APPEND);
    }
    if fd >= 0 {
        let _ = write(fd as i32, slice);
        let _ = write(fd as i32, b"\n");
        close(fd as i32);
    }
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut script_path: Option<&str> = None;
    let mut cmd_string: Option<&str> = None;
    let mut i = 1;
    while i < argc {
        let a = unsafe { arg(argv, i) };
        if a == "--user" && i + 1 < argc {
            i += 1;
            let u = unsafe { arg(argv, i) };
            unsafe {
                let bytes = u.as_bytes();
                let ubuf = &mut *core::ptr::addr_of_mut!(USER_BUF);
                let len = bytes.len().min(ubuf.len());
                ubuf[..len].copy_from_slice(&bytes[..len]);
                USER_LEN = len;
            }
        } else if a == "-c" && i + 1 < argc {
            i += 1;
            cmd_string = Some(unsafe { arg(argv, i) });
        } else if script_path.is_none() {
            script_path = Some(a);
        }
        i += 1;
    }

    init_env();

    if let Some(cmdline) = cmd_string {
        let mut buf = [0u8; LINE + 1];
        let bytes = cmdline.as_bytes();
        if bytes.len() > LINE {
            putln(b"sh: -c command too long");
            return 1;
        }
        buf[..bytes.len()].copy_from_slice(bytes);
        let (s, e) = trim(&buf, 0, bytes.len());
        let _ = run_line(&mut buf, s, e);
        return get_status();
    }

    if let Some(path) = script_path {
        return run_script(path);
    }

    load_history();
    putln(b"--- EspressoOS Shell (Userland) ---");
    let mut buf = [0u8; LINE + 1];
    loop {
        reap_background_jobs();
        print_prompt();
        let len = read_line(&mut buf[..LINE]);
        if len == usize::MAX {
            break;
        }
        let (s, e) = trim(&buf, 0, len);
        if s < e {
            record_history(&buf[s..e]);
        }
        if !run_line(&mut buf, s, e) {
            putln(b"Exiting the shell...");
            break;
        }
    }
    get_status()
}

fn run_script(path: &str) -> i32 {
    let fd = open(path, O_RDONLY);
    if fd < 0 {
        puts(b"sh: cannot open ");
        putln(path.as_bytes());
        return 1;
    }
    let file = unsafe { &mut *core::ptr::addr_of_mut!(SCRIPT_BUF) };
    let mut total = 0usize;
    loop {
        if total == file.len() {
            let mut probe = [0u8; 1];
            if read(fd as i32, &mut probe) > 0 {
                close(fd as i32);
                puts(b"sh: script too large: ");
                putln(path.as_bytes());
                return 1;
            }
            break;
        }
        let n = read(fd as i32, &mut file[total..]);
        if n < 0 {
            close(fd as i32);
            puts(b"sh: read error on ");
            putln(path.as_bytes());
            return 1;
        }
        if n == 0 {
            break;
        }
        total += n as usize;
    }
    close(fd as i32);

    let end = total;
    let mut buf = [0u8; LINE + 1];
    let mut start = 0usize;
    let mut i = 0usize;
    while i <= end {
        if i == end || file[i] == b'\n' {
            let mut line: &[u8] = &file[start..i];
            if line.last() == Some(&b'\r') {
                line = &line[..line.len() - 1];
            }
            if line.len() > LINE {
                puts(b"sh: line too long, skipped in ");
                putln(path.as_bytes());
            } else {
                let m = line.len();
                buf[..m].copy_from_slice(line);
                let (s, e) = trim(&buf, 0, m);
                if s < e && buf[s] != b'#' && !run_line(&mut buf, s, e) {
                    return get_status();
                }
            }
            start = i + 1;
        }
        i += 1;
    }
    get_status()
}

fn expand_segment(src: &[u8], out: &mut [u8; LINE + 1]) -> usize {
    let exp = unsafe { &mut *core::ptr::addr_of_mut!(EXP_BUF) };
    let mut r = 0usize;
    let mut w = 0usize;
    let mut quote: u8 = 0;

    while r < src.len() && w < LINE {
        let c = src[r];
        if quote == b'\'' {
            if c == b'\'' {
                quote = 0;
            }
            exp[w] = c;
            w += 1;
            r += 1;
            continue;
        }
        if c == b'\'' || c == b'"' {
            quote = if quote == c { 0 } else { c };
            exp[w] = c;
            w += 1;
            r += 1;
            continue;
        }
        if c == b'$' && r + 1 < src.len() {
            let next = src[r + 1];
            if next == b'?' {
                let mut nbuf = [0u8; 16];
                let bytes = fmt_i32(get_status(), &mut nbuf);
                let cp = bytes.len().min(LINE - w);
                exp[w..w + cp].copy_from_slice(&bytes[..cp]);
                w += cp;
                r += 2;
                continue;
            } else if next == b'$' {
                let mut nbuf = [0u8; 16];
                let bytes = fmt_usize(getpid() as usize, &mut nbuf);
                let cp = bytes.len().min(LINE - w);
                exp[w..w + cp].copy_from_slice(&bytes[..cp]);
                w += cp;
                r += 2;
                continue;
            } else if next == b'{' {
                if let Some(close_rel) = src[r + 2..].iter().position(|&b| b == b'}') {
                    let key = &src[r + 2..r + 2 + close_rel];
                    let mut vbuf = [0u8; PATH_MAX];
                    if let Some(val) = env_get(key, &mut vbuf) {
                        let cp = val.len().min(LINE - w);
                        exp[w..w + cp].copy_from_slice(&val[..cp]);
                        w += cp;
                    }
                    r += 3 + close_rel;
                    continue;
                }
            } else if is_var_char(next) {
                let mut kend = r + 1;
                while kend < src.len() && is_var_char(src[kend]) {
                    kend += 1;
                }
                let key = &src[r + 1..kend];
                let mut vbuf = [0u8; PATH_MAX];
                if let Some(val) = env_get(key, &mut vbuf) {
                    let cp = val.len().min(LINE - w);
                    exp[w..w + cp].copy_from_slice(&val[..cp]);
                    w += cp;
                }
                r = kend;
                continue;
            }
        }
        exp[w] = c;
        w += 1;
        r += 1;
    }

    out[..w].copy_from_slice(&exp[..w]);
    out[w] = 0;
    w
}

fn is_assignment_only(seg: &[u8]) -> Option<(&[u8], &[u8])> {
    let eq = seg.iter().position(|&b| b == b'=')?;
    if eq == 0 {
        return None;
    }
    let key = &seg[..eq];
    for (idx, &b) in key.iter().enumerate() {
        if (idx == 0 && b >= b'0' && b <= b'9') || !is_var_char(b) {
            return None;
        }
    }
    let rest = &seg[eq + 1..];
    let quoted = rest.len() >= 2 && (rest[0] == b'"' || rest[0] == b'\'') && rest[rest.len() - 1] == rest[0];
    if rest.contains(&b' ') && !quoted {
        return None;
    }
    Some((key, if quoted { &rest[1..rest.len() - 1] } else { rest }))
}

fn builtin_export(rest: &[u8]) {
    let (s, e) = trim(rest, 0, rest.len());
    if s >= e {
        unsafe {
            for i in 0..ENV_MAX {
                let kl = ENV_KLEN[i];
                if kl > 0 {
                    puts(b"export ");
                    puts(&ENV_KEYS[i][..kl]);
                    puts(b"=\"");
                    puts(&ENV_VALS[i][..ENV_VLEN[i]]);
                    putln(b"\"");
                }
            }
        }
        set_status(0);
        return;
    }
    let sub = &rest[s..e];
    if let Some(eq) = sub.iter().position(|&b| b == b'=') {
        let key = &sub[..eq];
        let mut val = &sub[eq + 1..];
        if val.len() >= 2 && (val[0] == b'"' || val[0] == b'\'') && val[val.len() - 1] == val[0] {
            val = &val[1..val.len() - 1];
        }
        env_set(key, val);
    }
    sync_env_file();
    set_status(0);
}

fn builtin_history() {
    unsafe {
        let start = HISTORY_COUNT.saturating_sub(HISTORY_MAX);
        for idx in start..HISTORY_COUNT {
            let slot = idx % HISTORY_MAX;
            let mut nbuf = [0u8; 16];
            puts(b"  ");
            puts(fmt_usize(idx + 1, &mut nbuf));
            puts(b"  ");
            putln(&HISTORY_BUF[slot][..HISTORY_LENS[slot]]);
        }
    }
    set_status(0);
}

fn exec_line(raw_buf: &[u8], s: usize, e: usize) -> bool {
    if s >= e {
        return true;
    }
    let mut exp_line = [0u8; LINE + 1];
    let elen = expand_segment(&raw_buf[s..e], &mut exp_line);
    let (s, e) = trim(&exp_line, 0, elen);
    if s >= e {
        return true;
    }

    let slice = &exp_line[s..e];
    if slice == b"exit" || slice.starts_with(b"exit ") {
        if slice.len() > 5 {
            let mut code = 0i32;
            let (cs, ce) = trim(&exp_line, s + 5, e);
            for &b in &exp_line[cs..ce] {
                if b >= b'0' && b <= b'9' {
                    code = code * 10 + (b - b'0') as i32;
                }
            }
            set_status(code);
        }
        return false;
    }
    if slice == b"help" {
        print_help();
        set_status(0);
        return true;
    }
    if slice == b"clear" {
        puts(b"\x1b[2J\x1b[H");
        set_status(0);
        return true;
    }
    if slice == b"true" {
        set_status(0);
        return true;
    }
    if slice == b"false" {
        set_status(1);
        return true;
    }
    if slice.starts_with(b"sudo ") {
        let (ss, se) = trim(&exp_line, s + 5, e);
        if ss < se {
            return exec_line(&exp_line, ss, se);
        }
        return true;
    }
    if slice == b"history" {
        builtin_history();
        return true;
    }

    if find_top(&exp_line, s, e, b'|').is_none() && find_top(&exp_line, s, e, b'>').is_none() {
        if slice == b"pwd" || slice.starts_with(b"pwd ") {
            builtin_pwd();
            return true;
        }
        if slice == b"cd" || slice.starts_with(b"cd ") {
            builtin_cd(slice);
            return true;
        }
        if slice == b"export" {
            builtin_export(&[]);
            return true;
        }
        if slice.starts_with(b"export ") {
            builtin_export(&slice[7..]);
            return true;
        }
        if slice.starts_with(b"unset ") {
            let (ks, ke) = trim(&slice[6..], 0, slice.len() - 6);
            if ks < ke {
                env_unset(&slice[6 + ks..6 + ke]);
                sync_env_file();
            }
            set_status(0);
            return true;
        }
        if slice.starts_with(b". ") || slice.starts_with(b"source ") {
            let skip = if slice[0] == b'.' { 2 } else { 7 };
            let (ps, pe) = trim(&slice[skip..], 0, slice.len() - skip);
            if let Some(spath) = ascii_str(&slice[skip + ps..skip + pe]) {
                let st = run_script(spath);
                set_status(st);
            }
            return true;
        }
        if let Some((k, v)) = is_assignment_only(slice) {
            env_set(k, v);
            sync_env_file();
            set_status(0);
            return true;
        }
    }

    let mut ce = e;
    let mut is_bg = false;
    if ce > s && exp_line[ce - 1] == b'&' {
        is_bg = true;
        ce -= 1;
        let t = trim(&exp_line, s, ce);
        ce = t.1;
    }

    let mut stages = [(0usize, 0usize); MAX_STAGES];
    let mut nst = 0usize;
    let mut a = s;
    loop {
        let cut = find_top(&exp_line, a, ce, b'|');
        let b = cut.unwrap_or(ce);
        if nst >= MAX_STAGES {
            putln(b"sh: too many pipeline stages");
            set_status(1);
            return true;
        }
        stages[nst] = (a, b);
        nst += 1;
        match cut {
            Some(k) => a = k + 1,
            None => break,
        }
    }
    run_stages(&mut exp_line, &stages[..nst], is_bg);
    true
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ChainOp {
    Always,
    And,
    Or,
}

fn run_line(buf: &mut [u8], s: usize, e: usize) -> bool {
    let mut seg_start = s;
    let mut prev_op = ChainOp::Always;
    let mut i = s;
    let mut quote: u8 = 0;

    loop {
        let split = if i >= e {
            Some((e, 0usize, ChainOp::Always))
        } else {
            let c = buf[i];
            if quote != 0 {
                if c == quote {
                    quote = 0;
                }
                None
            } else if c == b'\'' || c == b'"' {
                quote = c;
                None
            } else if c == b';' {
                Some((i, 1usize, ChainOp::Always))
            } else if c == b'&' && i + 1 < e && buf[i + 1] == b'&' {
                Some((i, 2usize, ChainOp::And))
            } else if c == b'|' && i + 1 < e && buf[i + 1] == b'|' {
                Some((i, 2usize, ChainOp::Or))
            } else {
                None
            }
        };

        if let Some((pos, adv, next_op)) = split {
            let st = get_status();
            let run = prev_op == ChainOp::Always
                || (prev_op == ChainOp::And && st == 0)
                || (prev_op == ChainOp::Or && st != 0);
            if run {
                let (ss, se) = trim(buf, seg_start, pos);
                if ss < se && !exec_line(buf, ss, se) {
                    return false;
                }
            }
            if pos >= e {
                break;
            }
            prev_op = next_op;
            seg_start = pos + adv;
            i = pos + adv;
        } else {
            i += 1;
        }
    }
    true
}

fn builtin_pwd() {
    let mut cwd = [0u8; PATH_MAX];
    let n = getcwd(&mut cwd);
    if n >= 0 {
        putln(&cwd[..n as usize]);
        set_status(0);
    } else {
        putln(b"pwd: error");
        set_status(1);
    }
}

fn builtin_cd(line: &[u8]) {
    let (ts, te) = if line.len() > 2 {
        trim(&line[2..], 0, line.len() - 2)
    } else {
        (0, 0)
    };
    let mut hbuf = [0u8; PATH_MAX];
    let target_bytes = if ts >= te || &line[2 + ts..2 + te] == b"~" {
        env_get(b"HOME", &mut hbuf).unwrap_or(b"/")
    } else {
        &line[2 + ts..2 + te]
    };
    let target = ascii_str(target_bytes).unwrap_or("/");
    let r = chdir(target);
    if r < 0 {
        puts(b"cd: ");
        puts(target.as_bytes());
        if r == -20 {
            putln(b": not a directory");
        } else {
            putln(b": no such file or directory");
        }
        set_status(1);
    } else {
        set_status(0);
    }
}

fn prompt_bytes(out: &mut [u8]) -> usize {
    let mut cwd = [0u8; PATH_MAX];
    let mut o = 0;
    unsafe {
        if USER_LEN > 0 {
            let u = &(&*core::ptr::addr_of!(USER_BUF))[..USER_LEN];
            if o + u.len() + 1 <= out.len() {
                out[o..o + u.len()].copy_from_slice(u);
                o += u.len();
                out[o] = b'@';
                o += 1;
            }
        }
    }

    let cn = getcwd(&mut cwd);
    if cn == 1 && cwd[0] == b'/' {
        let p = b"EspressoOS:~$ ";
        if o + p.len() <= out.len() {
            out[o..o + p.len()].copy_from_slice(p);
            o += p.len();
        }
    } else if cn > 0 {
        let p1 = b"EspressoOS:";
        let s = &cwd[..cn as usize];
        let p2 = b"$ ";
        if o + p1.len() + s.len() + p2.len() <= out.len() {
            out[o..o + p1.len()].copy_from_slice(p1);
            o += p1.len();
            out[o..o + s.len()].copy_from_slice(s);
            o += s.len();
            out[o..o + p2.len()].copy_from_slice(p2);
            o += p2.len();
        }
    } else {
        let p = b"EspressoOS:?$ ";
        if o + p.len() <= out.len() {
            out[o..o + p.len()].copy_from_slice(p);
            o += p.len();
        }
    }
    o
}

fn print_prompt() {
    let mut buf = [0u8; 128];
    let n = prompt_bytes(&mut buf);
    puts(&buf[..n]);
}

fn redraw_line(buf: &[u8], len: usize, pos: usize) {
    let mut scratch = [0u8; 384];
    let mut o = 0;
    scratch[o] = b'\r';
    o += 1;
    o += prompt_bytes(&mut scratch[o..]);
    if len > 0 {
        let chunk_len = len.min(LINE).min(scratch.len().saturating_sub(o + 24));
        scratch[o..o + chunk_len].copy_from_slice(&buf[..chunk_len]);
        o += chunk_len;
    }
    scratch[o..o + 3].copy_from_slice(b"\x1b[K");
    o += 3;

    if pos < len {
        let back = len - pos;
        let mut num_buf = [0u8; 16];
        let bbytes = fmt_usize(back, &mut num_buf);
        scratch[o..o + 2].copy_from_slice(b"\x1b[");
        o += 2;
        scratch[o..o + bbytes.len()].copy_from_slice(bbytes);
        o += bbytes.len();
        scratch[o] = b'D';
        o += 1;
    }
    puts(&scratch[..o]);
}

fn try_read_byte() -> Option<u8> {
    let mut c = [0u8; 1];
    let start = uptime_ms();
    loop {
        if read(0, &mut c) > 0 {
            return Some(c[0]);
        }
        if uptime_ms().saturating_sub(start) > 20 {
            return None;
        }
        yield_now();
    }
}

const BUILTINS: &[&[u8]] = &[
    b"help", b"clear", b"exit", b"cd", b"pwd", b"sudo", b"export", b"unset", b"source",
    b"history", b"true", b"false",
];

fn rfind_slash(b: &[u8]) -> Option<usize> {
    let mut i = b.len();
    while i > 0 {
        i -= 1;
        if b[i] == b'/' {
            return Some(i);
        }
    }
    None
}

fn handle_tab_completion(buf: &mut [u8], len: &mut usize, pos: &mut usize) {
    let dir_buf = unsafe { &mut *core::ptr::addr_of_mut!(DIR_BUF) };
    let mut idx = *pos;
    while idx > 0 && buf[idx - 1] != b' ' {
        idx -= 1;
    }
    let word_start = idx;
    let mut pfx_copy = [0u8; 64];
    let plen = (*pos - word_start).min(64);
    pfx_copy[..plen].copy_from_slice(&buf[word_start..word_start + plen]);
    let prefix = &pfx_copy[..plen];

    let mut matches: [(&[u8], &[u8]); 24] = [(&[], &[]); 24];
    let mut match_count = 0usize;

    if word_start == 0 && rfind_slash(prefix).is_none() {
        for &b in BUILTINS {
            if b.starts_with(prefix) && match_count < 24 {
                matches[match_count] = (b, b" ");
                match_count += 1;
            }
        }
        let n = readdir("/bin", dir_buf);
        if n > 0 {
            let limit = n as usize;
            let mut p = 0;
            while p + 11 <= limit {
                let nl = u16::from_le_bytes([dir_buf[p + 9], dir_buf[p + 10]]) as usize;
                p += 11;
                if p + nl > limit {
                    break;
                }
                let name = &dir_buf[p..p + nl];
                if name.starts_with(prefix) && match_count < 24 {
                    let mut exists = false;
                    for k in 0..match_count {
                        if matches[k].0 == name {
                            exists = true;
                            break;
                        }
                    }
                    if !exists {
                        matches[match_count] = (name, b" ");
                        match_count += 1;
                    }
                }
                p += nl;
            }
        }
    } else {
        let (dir_b, file_pfx) = match rfind_slash(prefix) {
            Some(sp) => (&prefix[..sp + 1], &prefix[sp + 1..]),
            None => (&b"."[..], prefix),
        };
        let dstr = ascii_str(if dir_b.is_empty() { b"." } else { dir_b }).unwrap_or(".");
        let n = readdir(dstr, dir_buf);
        if n > 0 {
            let limit = n as usize;
            let mut p = 0;
            while p + 11 <= limit {
                let kind = dir_buf[p + 8];
                let nl = u16::from_le_bytes([dir_buf[p + 9], dir_buf[p + 10]]) as usize;
                p += 11;
                if p + nl > limit {
                    break;
                }
                let name = &dir_buf[p..p + nl];
                if name != b"." && name != b".." && name.starts_with(file_pfx) && match_count < 24 {
                    matches[match_count] = (name, if kind == 2 { b"/" } else { b" " });
                    match_count += 1;
                }
                p += nl;
            }
        }
    }

    if match_count == 1 {
        let (name, suff) = matches[0];
        let sub_pfx = if word_start != 0 {
            match rfind_slash(prefix) {
                Some(sp) => &prefix[sp + 1..],
                None => prefix,
            }
        } else {
            prefix
        };
        if name.len() >= sub_pfx.len() {
            let comp = &name[sub_pfx.len()..];
            let add = comp.len() + suff.len();
            if *len + add <= buf.len() {
                buf.copy_within(*pos..*len, *pos + add);
                buf[*pos..*pos + comp.len()].copy_from_slice(comp);
                buf[*pos + comp.len()..*pos + add].copy_from_slice(suff);
                *len += add;
                *pos += add;
                redraw_line(buf, *len, *pos);
            }
        }
    } else if match_count > 1 {
        putln(b"");
        for k in 0..match_count {
            puts(matches[k].0);
            puts(b"  ");
        }
        putln(b"");
        redraw_line(buf, *len, *pos);
    }
}

fn read_line(buf: &mut [u8]) -> usize {
    let mut len = 0usize;
    let mut pos = 0usize;

    unsafe {
        let mut history_idx = HISTORY_COUNT;
        let mut draft_buf = [0u8; LINE];
        let mut draft_len = 0usize;

        loop {
            let mut c = [0u8; 1];
            if read(0, &mut c) <= 0 {
                yield_now();
                continue;
            }
            let ch = c[0];
            let mut dirty = false;

            match ch {
                b'\n' | b'\r' => {
                    putln(b"");
                    return len;
                }
                0x08 | 0x7f if pos > 0 => {
                    buf.copy_within(pos..len, pos - 1);
                    len -= 1;
                    pos -= 1;
                    dirty = true;
                }
                0x09 => handle_tab_completion(buf, &mut len, &mut pos),
                0x01 => {
                    pos = 0;
                    dirty = true;
                }
                0x05 => {
                    pos = len;
                    dirty = true;
                }
                0x15 if pos > 0 => {
                    buf.copy_within(pos..len, 0);
                    len -= pos;
                    pos = 0;
                    dirty = true;
                }
                0x0b if pos < len => {
                    len = pos;
                    dirty = true;
                }
                0x17 if pos > 0 => {
                    let mut wstart = pos;
                    while wstart > 0 && buf[wstart - 1] == b' ' {
                        wstart -= 1;
                    }
                    while wstart > 0 && buf[wstart - 1] != b' ' {
                        wstart -= 1;
                    }
                    buf.copy_within(pos..len, wstart);
                    len -= pos - wstart;
                    pos = wstart;
                    dirty = true;
                }
                0x0c => {
                    puts(b"\x1b[2J\x1b[H");
                    dirty = true;
                }
                0x04 => {
                    if len == 0 {
                        putln(b"exit");
                        return usize::MAX;
                    } else if pos < len {
                        buf.copy_within(pos + 1..len, pos);
                        len -= 1;
                        dirty = true;
                    }
                }
                0x03 => {
                    putln(b"^C");
                    len = 0;
                    pos = 0;
                    set_status(130);
                    print_prompt();
                }
                0x1b => {
                    if try_read_byte() == Some(b'[') {
                        if let Some(code) = try_read_byte() {
                            match code {
                                b'A' => {
                                    let min_idx = HISTORY_COUNT.saturating_sub(HISTORY_MAX);
                                    if history_idx > min_idx {
                                        if history_idx == HISTORY_COUNT {
                                            draft_buf[..len].copy_from_slice(&buf[..len]);
                                            draft_len = len;
                                        }
                                        history_idx -= 1;
                                        let h_idx = history_idx % HISTORY_MAX;
                                        let h_len = HISTORY_LENS[h_idx];
                                        buf[..h_len].copy_from_slice(&HISTORY_BUF[h_idx][..h_len]);
                                        len = h_len;
                                        pos = h_len;
                                        dirty = true;
                                    }
                                }
                                b'B' if history_idx < HISTORY_COUNT => {
                                    history_idx += 1;
                                    let (src, slen): (&[u8], usize) = if history_idx == HISTORY_COUNT {
                                        (&draft_buf, draft_len)
                                    } else {
                                        let h_idx = history_idx % HISTORY_MAX;
                                        (&HISTORY_BUF[h_idx], HISTORY_LENS[h_idx])
                                    };
                                    buf[..slen].copy_from_slice(&src[..slen]);
                                    len = slen;
                                    pos = slen;
                                    dirty = true;
                                }
                                b'C' if pos < len => {
                                    pos += 1;
                                    dirty = true;
                                }
                                b'D' if pos > 0 => {
                                    pos -= 1;
                                    dirty = true;
                                }
                                b'H' | b'1' => {
                                    if code == b'1' { let _ = try_read_byte(); }
                                    pos = 0;
                                    dirty = true;
                                }
                                b'F' | b'4' => {
                                    if code == b'4' { let _ = try_read_byte(); }
                                    pos = len;
                                    dirty = true;
                                }
                                b'3' => {
                                    let _ = try_read_byte();
                                    if pos < len {
                                        buf.copy_within(pos + 1..len, pos);
                                        len -= 1;
                                        dirty = true;
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
                _ if (0x20..0x7f).contains(&ch) && len < buf.len() => {
                    buf.copy_within(pos..len, pos + 1);
                    buf[pos] = ch;
                    len += 1;
                    pos += 1;
                    dirty = true;
                }
                _ => {}
            }

            if dirty {
                redraw_line(buf, len, pos);
            }
        }
    }
}

fn tokenize(
    buf: &mut [u8],
    start: usize,
    end: usize,
    argv: &mut [*const u8; MAX_ARGV],
) -> Option<usize> {
    let mut offs = [0usize; MAX_ARGV];
    let mut n = 0;
    let mut r = start;
    let mut w = start;
    while r < end {
        while r < end && (buf[r] == b' ' || buf[r] == b'\t') {
            r += 1;
        }
        if r >= end {
            break;
        }
        if n + 1 >= MAX_ARGV {
            return None;
        }
        offs[n] = w;
        n += 1;
        let mut quote: u8 = 0;
        while r < end {
            let c = buf[r];
            if quote != 0 {
                if c == quote {
                    quote = 0;
                } else {
                    buf[w] = c;
                    w += 1;
                }
                r += 1;
            } else if c == b'"' || c == b'\'' {
                quote = c;
                r += 1;
            } else if c == b' ' || c == b'\t' {
                break;
            } else {
                buf[w] = c;
                w += 1;
                r += 1;
            }
        }
        buf[w] = 0;
        w += 1;
        if r < end {
            r += 1;
        }
    }
    let base = buf.as_ptr();
    for k in 0..n {
        argv[k] = unsafe { base.add(offs[k]) };
    }
    argv[n] = core::ptr::null();
    Some(n)
}

fn find_top(buf: &[u8], from: usize, to: usize, delim: u8) -> Option<usize> {
    let mut i = from;
    let mut quote: u8 = 0;
    while i < to {
        let c = buf[i];
        if quote != 0 {
            if c == quote {
                quote = 0;
            }
        } else if c == b'\'' || c == b'"' {
            quote = c;
        } else if c == delim {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn prepare<'a>(
    buf: &mut [u8],
    range: (usize, usize),
    argv: &mut [*const u8; MAX_ARGV],
    path_buf: &'a mut [u8],
) -> Option<&'a str> {
    match tokenize(buf, range.0, range.1, argv) {
        None => {
            putln(b"sh: too many arguments");
            return None;
        }
        Some(0) => return None,
        Some(_) => {}
    }
    let path = resolve_path(unsafe { arg(argv.as_ptr(), 0) }, path_buf);
    if path.is_empty() {
        putln(b"sh: invalid command path");
        return None;
    }
    Some(path)
}

#[derive(Clone, Copy)]
struct RedirSpec {
    stdin: Option<(usize, usize)>,
    stdout: Option<(usize, usize, bool)>,
    stderr: Option<(usize, usize, bool)>,
    err_to_out: bool,
}

fn take_redir_tok(orig_buf: &[u8], clean_buf: &mut [u8], op_start: usize, after: usize, b: usize) -> (usize, usize, usize) {
    let (ts, te0) = trim(orig_buf, after, b);
    let te = find_top(orig_buf, ts, te0, b' ').unwrap_or(te0);
    for k in op_start..te {
        clean_buf[k] = b' ';
    }
    (ts, te, te)
}

fn extract_redirs(orig_buf: &[u8], clean_buf: &mut [u8], a: usize, b: usize) -> RedirSpec {
    let mut spec = RedirSpec {
        stdin: None,
        stdout: None,
        stderr: None,
        err_to_out: false,
    };
    let mut i = a;
    let mut quote: u8 = 0;

    while i < b {
        let c = orig_buf[i];
        if quote != 0 {
            if c == quote { quote = 0; }
            i += 1;
            continue;
        }
        if c == b'\'' || c == b'"' {
            quote = c;
            i += 1;
            continue;
        }

        if c == b'2' && i + 1 < b && orig_buf[i + 1] == b'>' && (i == a || orig_buf[i - 1] == b' ') {
            if i + 3 < b && orig_buf[i + 2] == b'&' && orig_buf[i + 3] == b'1' {
                spec.err_to_out = true;
                for k in i..i + 4 { clean_buf[k] = b' '; }
                i += 4;
                continue;
            }
            let append = i + 2 < b && orig_buf[i + 2] == b'>';
            let (ts, te, next_i) = take_redir_tok(orig_buf, clean_buf, i, if append { i + 3 } else { i + 2 }, b);
            spec.stderr = Some((ts, te, append));
            i = next_i;
            continue;
        }

        if c == b'>' {
            let append = i + 1 < b && orig_buf[i + 1] == b'>';
            let (ts, te, next_i) = take_redir_tok(orig_buf, clean_buf, i, if append { i + 2 } else { i + 1 }, b);
            spec.stdout = Some((ts, te, append));
            i = next_i;
            continue;
        }

        if c == b'<' {
            let (ts, te, next_i) = take_redir_tok(orig_buf, clean_buf, i, i + 1, b);
            spec.stdin = Some((ts, te));
            i = next_i;
            continue;
        }
        i += 1;
    }
    spec
}

fn unquote_slice(buf: &[u8], ts: usize, te: usize) -> Option<&str> {
    let (mut a, mut b) = (ts, te);
    if b > a + 1 && (buf[a] == b'"' || buf[a] == b'\'') && buf[b - 1] == buf[a] {
        a += 1;
        b -= 1;
    }
    if a >= b {
        return None;
    }
    ascii_str(&buf[a..b])
}

fn open_redir(buf: &[u8], ts: usize, te: usize, flags: u32) -> i32 {
    match unquote_slice(buf, ts, te) {
        Some(p) => {
            let fd = open(p, flags);
            if fd >= 0 { fd as i32 } else { -1 }
        }
        None => -1,
    }
}

/// Unified executor for single commands and multi-stage pipelines (`1..=MAX_STAGES`).
fn run_stages(buf: &mut [u8], stages: &[(usize, usize)], is_bg: bool) {
    let n = stages.len();
    let saved_out = dup2(1, 10) as i32;
    let saved_in = dup2(0, 11) as i32;
    let saved_err = dup2(2, 13) as i32;

    let mut prev_read = -1i32;
    let mut pids = [-1i32; MAX_STAGES];
    let mut nspawned = 0usize;
    let mut leader_pgid = 0u32;

    for i in 0..n {
        let (a, b) = stages[i];
        let mut clean = [0u8; LINE + 1];
        clean[a..b].copy_from_slice(&buf[a..b]);
        let spec = extract_redirs(buf, &mut clean, a, b);

        let mut in_rfd = -1i32;
        let mut out_rfd = -1i32;
        let mut err_rfd = -1i32;
        let mut redir_fail = false;

        if let Some((ts, te)) = spec.stdin {
            in_rfd = open_redir(buf, ts, te, O_RDONLY);
            if in_rfd < 0 {
                redir_fail = true;
            }
        }
        if !redir_fail {
            if let Some((ts, te, app)) = spec.stdout {
                let fl = O_WRONLY | O_CREATE | if app { O_APPEND } else { O_TRUNC };
                out_rfd = open_redir(buf, ts, te, fl);
                if out_rfd < 0 {
                    redir_fail = true;
                }
            }
        }
        if !redir_fail {
            if let Some((ts, te, app)) = spec.stderr {
                let fl = O_WRONLY | O_CREATE | if app { O_APPEND } else { O_TRUNC };
                err_rfd = open_redir(buf, ts, te, fl);
                if err_rfd < 0 {
                    redir_fail = true;
                }
            }
        }
        if redir_fail {
            putln(b"sh: cannot open redirection target");
            if in_rfd >= 0 { close(in_rfd); }
            if out_rfd >= 0 { close(out_rfd); }
            if err_rfd >= 0 { close(err_rfd); }
            set_status(1);
            break;
        }

        let mut argv = [core::ptr::null(); MAX_ARGV];
        let mut path_buf = [0u8; PATH_MAX];
        let path = match prepare(&mut clean, (a, b), &mut argv, &mut path_buf) {
            Some(p) => p,
            None => {
                if in_rfd >= 0 { close(in_rfd); }
                if out_rfd >= 0 { close(out_rfd); }
                if err_rfd >= 0 { close(err_rfd); }
                break;
            }
        };

        let mut next_read = -1i32;
        let mut this_write = -1i32;
        if i + 1 < n {
            let mut pp = [0i32; 2];
            if pipe(&mut pp) < 0 {
                putln(b"sh: pipe error");
                if in_rfd >= 0 { close(in_rfd); }
                if out_rfd >= 0 { close(out_rfd); }
                if err_rfd >= 0 { close(err_rfd); }
                break;
            }
            next_read = pp[0];
            this_write = pp[1];
        }

        let _ = dup2(if in_rfd >= 0 { in_rfd } else if prev_read >= 0 { prev_read } else { saved_in }, 0);
        let _ = dup2(if out_rfd >= 0 { out_rfd } else if this_write >= 0 { this_write } else { saved_out }, 1);
        if err_rfd >= 0 {
            let _ = dup2(err_rfd, 2);
        } else if spec.err_to_out {
            let _ = dup2(1, 2);
        } else {
            let _ = dup2(saved_err, 2);
        }

        let pid = spawn(path, argv.as_ptr());
        if pid >= 0 {
            let upid = pid as u32;
            if leader_pgid == 0 {
                leader_pgid = upid;
            }
            let _ = setpgid(upid, leader_pgid);
            pids[nspawned] = pid as i32;
            nspawned += 1;
        } else {
            puts(b"Error executing: ");
            putln(path.as_bytes());
        }

        if in_rfd >= 0 { close(in_rfd); }
        if out_rfd >= 0 { close(out_rfd); }
        if err_rfd >= 0 { close(err_rfd); }
        if prev_read >= 0 { close(prev_read); }
        if this_write >= 0 { close(this_write); }
        prev_read = next_read;
    }

    if prev_read >= 0 {
        close(prev_read);
    }

    let _ = dup2(saved_out, 1);
    let _ = dup2(saved_in, 0);
    let _ = dup2(saved_err, 2);
    close(saved_out);
    close(saved_in);
    close(saved_err);

    if nspawned == 0 {
        set_status(127);
        return;
    }

    if is_bg {
        puts(b"[1] ");
        put_i32(pids[nspawned - 1]);
        putln(b"");
        set_status(0);
        return;
    }

    if leader_pgid != 0 {
        let _ = tcsetpgrp(leader_pgid);
    }
    let mut last_st = 0;
    for k in 0..nspawned {
        let mut status = 0;
        let _ = waitpid(pids[k], &mut status, 0);
        last_st = status;
    }
    let _ = tcsetpgrp(0);
    set_status(last_st);
}

fn print_help() {
    putln(b"EspressoOS shell builtins: help clear cd pwd export unset source history exit");
    putln(b"Operators: ;  &&  ||  &  |  <  >  >>  2>  2>&1  $VAR  $?  $$");
    putln(b"Programs in /bin:");
    let buf = unsafe { &mut *core::ptr::addr_of_mut!(DIR_BUF) };
    let n = readdir("/bin", buf);
    if n <= 0 {
        return;
    }
    let mut pos = 0;
    let limit = n as usize;
    let mut col = 0usize;
    while pos + 11 <= limit {
        let nl = u16::from_le_bytes([buf[pos + 9], buf[pos + 10]]) as usize;
        pos += 11;
        if pos + nl > limit {
            break;
        }
        let name = &buf[pos..pos + nl];
        if name != b"." && name != b".." {
            puts(b"  ");
            puts(name);
            let pad = 12usize.saturating_sub(nl);
            for _ in 0..pad {
                puts(b" ");
            }
            col += 1;
            if col == 5 {
                putln(b"");
                col = 0;
            }
        }
        pos += nl;
    }
    if col != 0 {
        putln(b"");
    }
}

fn resolve_path<'a>(cmd: &str, out_buf: &'a mut [u8]) -> &'a str {
    let cb = cmd.as_bytes();
    let (s, e) = trim(cb, 0, cb.len());
    let cb = &cb[s..e];
    if cb.is_empty() {
        return "";
    }
    if rfind_slash(cb).is_some() {
        if cb.len() < out_buf.len() {
            out_buf[..cb.len()].copy_from_slice(cb);
            ascii_str(&out_buf[..cb.len()]).unwrap_or("")
        } else {
            ""
        }
    } else {
        let p = b"/bin/";
        if p.len() + cb.len() < out_buf.len() {
            out_buf[..p.len()].copy_from_slice(p);
            out_buf[p.len()..p.len() + cb.len()].copy_from_slice(cb);
            ascii_str(&out_buf[..p.len() + cb.len()]).unwrap_or("")
        } else {
            ""
        }
    }
}
