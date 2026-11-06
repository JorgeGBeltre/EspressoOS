#![no_std]
#![no_main]

use libc::{arg, chown, close, lstat, open, println, read, readdir, StatBuf, O_RDONLY, PATH_MAX};

fn parse_u32(s: &str) -> Option<u32> {
    if s.is_empty() {
        return None;
    }
    let mut v = 0u32;
    for &b in s.as_bytes() {
        if b >= b'0' && b <= b'9' {
            v = v.saturating_mul(10).saturating_add((b - b'0') as u32);
        } else {
            return None;
        }
    }
    Some(v)
}

fn lookup_gid(name: &str) -> Option<u32> {
    if let Some(id) = parse_u32(name) {
        return Some(id);
    }
    let fd = open("/etc/group", O_RDONLY);
    if fd >= 0 {
        let mut buf = [0u8; 512];
        let n = read(fd as i32, &mut buf);
        close(fd as i32);
        if n > 0 {
            if let Ok(text) = core::str::from_utf8(&buf[..n as usize]) {
                for line in text.lines() {
                    let mut p = line.trim().split(':');
                    let g = p.next().unwrap_or("");
                    let _pw = p.next();
                    if g == name {
                        if let Some(gid) = p.next().and_then(parse_u32) {
                            return Some(gid);
                        }
                    }
                }
            }
        }
    }
    match name {
        "root" => Some(0),
        "wheel" | "adm" => Some(10),
        "users" => Some(100),
        "nobody" | "nogroup" => Some(65534),
        _ => None,
    }
}

fn apply_chgrp(path: &str, gid: u32, recursive: bool, verbose: bool, depth: usize) -> bool {
    let mut st = StatBuf::zeroed();
    if lstat(path, &mut st) < 0 {
        println!("chgrp: cannot access '{}': No such file or directory", path);
        return false;
    }
    let rc = chown(path, st.uid as u32, gid);
    if rc < 0 {
        println!("chgrp: changing group of '{}' failed (errno {})", path, rc);
        return false;
    }
    if verbose {
        println!("group of '{}' changed to {}", path, gid);
    }

    let mut ok = true;
    if recursive && st.kind == 2 && depth < 6 {
        let mut dbuf = [0u8; 1024];
        let n = readdir(path, &mut dbuf);
        if n > 0 {
            let limit = n as usize;
            let mut p = 0usize;
            while p + 11 <= limit {
                let nlen = u16::from_le_bytes([dbuf[p + 9], dbuf[p + 10]]) as usize;
                p += 11;
                if p + nlen > limit {
                    break;
                }
                if let Ok(name) = core::str::from_utf8(&dbuf[p..p + nlen]) {
                    if name != "." && name != ".." {
                        let mut cbuf = [0u8; PATH_MAX];
                        let pb = path.trim_end_matches('/').as_bytes();
                        let nb = name.as_bytes();
                        if pb.len() + 1 + nb.len() < cbuf.len() {
                            let mut k: usize;
                            if pb.is_empty() {
                                cbuf[0] = b'/';
                                k = 1;
                            } else {
                                cbuf[..pb.len()].copy_from_slice(pb);
                                k = pb.len();
                                cbuf[k] = b'/';
                                k += 1;
                            }
                            cbuf[k..k + nb.len()].copy_from_slice(nb);
                            k += nb.len();
                            if let Ok(child) = core::str::from_utf8(&cbuf[..k]) {
                                if !apply_chgrp(child, gid, true, verbose, depth + 1) {
                                    ok = false;
                                }
                            }
                        }
                    }
                }
                p += nlen;
            }
        }
    }
    ok
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 3 {
        println!("Usage: chgrp [-R] [-v] GROUP FILE...");
        return 1;
    }

    let mut verbose = false;
    let mut recursive = false;
    let mut arg_idx = 1;

    while arg_idx < argc {
        let s = unsafe { arg(argv, arg_idx) };
        if s == "-h" || s == "--help" {
            println!("Usage: chgrp [-R] [-v] GROUP FILE...");
            return 0;
        } else if s == "-v" || s == "--verbose" {
            verbose = true;
            arg_idx += 1;
        } else if s == "-R" || s == "--recursive" {
            recursive = true;
            arg_idx += 1;
        } else if s == "-Rv" || s == "-vR" {
            recursive = true;
            verbose = true;
            arg_idx += 1;
        } else {
            break;
        }
    }

    if arg_idx + 1 >= argc {
        println!("chgrp: missing operand");
        return 1;
    }

    let group = unsafe { arg(argv, arg_idx) };
    arg_idx += 1;

    let gid = match lookup_gid(group) {
        Some(g) => g,
        None => {
            println!("chgrp: invalid group: '{}'", group);
            return 1;
        }
    };

    let mut all_ok = true;
    for i in arg_idx..argc {
        let file = unsafe { arg(argv, i) };
        if !apply_chgrp(file, gid, recursive, verbose, 0) {
            all_ok = false;
        }
    }
    if all_ok { 0 } else { 1 }
}
