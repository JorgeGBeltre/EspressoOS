#![no_std]
#![no_main]

use libc::{arg, lstat, print, println, readdir, readlink, StatBuf, PATH_MAX};

fn mode_chars(kind: u8, mode: u16, out: &mut [u8; 10]) {
    out[0] = match kind {
        2 => b'd',
        3 => b'c',
        4 => b'l',
        5 => b'p',
        _ => b'-',
    };
    out[1] = if mode & 0o400 != 0 { b'r' } else { b'-' };
    out[2] = if mode & 0o200 != 0 { b'w' } else { b'-' };
    out[3] = if mode & 0o100 != 0 { b'x' } else { b'-' };
    out[4] = if mode & 0o040 != 0 { b'r' } else { b'-' };
    out[5] = if mode & 0o020 != 0 { b'w' } else { b'-' };
    out[6] = if mode & 0o010 != 0 { b'x' } else { b'-' };
    out[7] = if mode & 0o004 != 0 { b'r' } else { b'-' };
    out[8] = if mode & 0o002 != 0 { b'w' } else { b'-' };
    out[9] = if mode & 0o1000 != 0 {
        if mode & 0o001 != 0 { b't' } else { b'T' }
    } else if mode & 0o001 != 0 {
        b'x'
    } else {
        b'-'
    };
}

fn join_path<'a>(dir: &str, name: &str, buf: &'a mut [u8; PATH_MAX]) -> &'a str {
    let mut k: usize;
    if dir == "." {
        let nb = name.as_bytes();
        let c = nb.len().min(buf.len());
        buf[..c].copy_from_slice(&nb[..c]);
        return core::str::from_utf8(&buf[..c]).unwrap_or("");
    }
    let db = dir.trim_end_matches('/').as_bytes();
    if db.is_empty() {
        buf[0] = b'/';
        k = 1;
    } else {
        let c = db.len().min(buf.len().saturating_sub(2));
        buf[..c].copy_from_slice(&db[..c]);
        k = c;
        buf[k] = b'/';
        k += 1;
    }
    let nb = name.as_bytes();
    let c = nb.len().min(buf.len().saturating_sub(k));
    buf[k..k + c].copy_from_slice(&nb[..c]);
    k += c;
    core::str::from_utf8(&buf[..k]).unwrap_or("")
}

fn print_long_entry(full_path: &str, display_name: &str, fallback_kind: u8) {
    let mut st = StatBuf::zeroed();
    if lstat(full_path, &mut st) < 0 {
        st.kind = fallback_kind;
        st.mode = if fallback_kind == 2 { 0o755 } else { 0o644 };
        st.nlink = 1;
    }
    let mut m = [0u8; 10];
    mode_chars(st.kind, st.mode, &mut m);
    let sym = core::str::from_utf8(&m).unwrap_or("----------");
    print!("{} {:>2} {:>4} {:>4} {:>7} {}", sym, st.nlink, st.uid, st.gid, st.size, display_name);
    if st.kind == 4 {
        let mut lbuf = [0u8; PATH_MAX];
        let ln = readlink(full_path, &mut lbuf);
        if ln > 0 {
            if let Ok(target) = core::str::from_utf8(&lbuf[..ln as usize]) {
                print!(" -> {}", target);
            }
        }
    }
    println!("");
}

static mut DIR_BUF: [u8; 4096] = [0; 4096];

fn list(path: &str, long: bool, show_all: bool) -> i32 {
    let buf = unsafe { &mut *core::ptr::addr_of_mut!(DIR_BUF) };
    let n = readdir(path, buf);
    if n < 0 {
        // Might be a single file or symlink rather than a directory.
        let mut st = StatBuf::zeroed();
        if lstat(path, &mut st) == 0 {
            if long {
                print_long_entry(path, path, st.kind as u8);
            } else {
                println!("{}", path);
            }
            return 0;
        }
        println!("ls: {}: cannot read", path);
        return 1;
    }

    if show_all {
        if long {
            print_long_entry(path, ".", 2);
        } else {
            println!("./");
            println!("../");
        }
    }

    let limit = n as usize;
    let mut pos = 0;
    while pos + 11 <= limit {
        let kind = buf[pos + 8];
        let name_len = u16::from_le_bytes([buf[pos + 9], buf[pos + 10]]) as usize;
        pos += 11;

        if pos + name_len > limit {
            break;
        }
        if let Ok(name) = core::str::from_utf8(&buf[pos..pos + name_len]) {
            if !show_all && name.starts_with('.') {
                pos += name_len;
                continue;
            }
            if long {
                let mut fbuf = [0u8; PATH_MAX];
                let full = join_path(path, name, &mut fbuf);
                print_long_entry(full, name, kind);
            } else if kind == 2 {
                println!("{}/", name);
            } else if kind == 3 || kind == 4 {
                println!("{}@", name);
            } else if kind == 5 {
                println!("{}|", name);
            } else {
                println!("{}", name);
            }
        }
        pos += name_len;
    }
    0
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut long = false;
    let mut show_all = false;
    let mut paths = [""; 12];
    let mut pcount = 0usize;

    let mut i = 1;
    while i < argc {
        let a = unsafe { arg(argv, i) };
        if a == "-h" || a == "--help" {
            println!("Usage: ls [-l] [-a] [PATH...]");
            return 0;
        } else if a.starts_with('-') && a.len() > 1 {
            for &b in &a.as_bytes()[1..] {
                match b {
                    b'l' => long = true,
                    b'a' => show_all = true,
                    b'h' | b'1' | b'F' => {}
                    _ => {}
                }
            }
        } else if pcount < paths.len() {
            paths[pcount] = a;
            pcount += 1;
        }
        i += 1;
    }

    if pcount == 0 {
        return list(".", long, show_all);
    }

    let mut status = 0;
    for k in 0..pcount {
        if pcount > 1 {
            println!("{}:", paths[k]);
        }
        if list(paths[k], long, show_all) != 0 {
            status = 1;
        }
    }
    status
}
