#![no_std]
#![no_main]

use libc::{arg, chmod, lstat, println, readdir, stat, StatBuf, PATH_MAX};

fn parse_octal(s: &str) -> Option<u32> {
    if s.is_empty() || s.len() > 4 {
        return None;
    }
    let mut v = 0u32;
    for &b in s.as_bytes() {
        if b >= b'0' && b <= b'7' {
            v = (v << 3) | (b - b'0') as u32;
        } else {
            return None;
        }
    }
    Some(v)
}

fn eval_mode(spec: &str, current: u32) -> Option<u32> {
    if let Some(oct) = parse_octal(spec) {
        return Some(oct);
    }
    let mut mode = current & 0o7777;
    for clause in spec.split(',') {
        let bytes = clause.as_bytes();
        let mut idx = 0usize;
        let mut who_u = false;
        let mut who_g = false;
        let mut who_o = false;
        while idx < bytes.len() {
            match bytes[idx] {
                b'u' => who_u = true,
                b'g' => who_g = true,
                b'o' => who_o = true,
                b'a' => {
                    who_u = true;
                    who_g = true;
                    who_o = true;
                }
                _ => break,
            }
            idx += 1;
        }
        if !who_u && !who_g && !who_o {
            who_u = true;
            who_g = true;
            who_o = true;
        }
        if idx >= bytes.len() {
            return None;
        }
        let op = bytes[idx];
        if op != b'+' && op != b'-' && op != b'=' {
            return None;
        }
        idx += 1;
        let mut rwx = 0u32;
        let mut sticky = false;
        while idx < bytes.len() {
            match bytes[idx] {
                b'r' => rwx |= 0o4,
                b'w' => rwx |= 0o2,
                b'x' => rwx |= 0o1,
                b't' => sticky = true,
                _ => return None,
            }
            idx += 1;
        }
        let mut bits = 0u32;
        let mut clear_mask = 0u32;
        if who_u {
            bits |= rwx << 6;
            clear_mask |= 0o700;
        }
        if who_g {
            bits |= rwx << 3;
            clear_mask |= 0o070;
        }
        if who_o {
            bits |= rwx;
            clear_mask |= 0o007;
        }
        if sticky {
            bits |= 0o1000;
            clear_mask |= 0o1000;
        }
        match op {
            b'+' => mode |= bits,
            b'-' => mode &= !bits,
            b'=' => {
                mode &= !clear_mask;
                mode |= bits;
            }
            _ => return None,
        }
    }
    Some(mode)
}

fn apply_path(path: &str, mode_str: &str, recursive: bool, verbose: bool, depth: usize) -> bool {
    let mut st = StatBuf::zeroed();
    if lstat(path, &mut st) < 0 {
        println!("chmod: cannot access '{}': No such file or directory", path);
        return false;
    }
    if st.kind == 4 {
        if stat(path, &mut st) < 0 {
            return true;
        }
    }
    let new_mode = match eval_mode(mode_str, st.mode as u32) {
        Some(m) => m,
        None => {
            println!("chmod: invalid mode: '{}'", mode_str);
            return false;
        }
    };
    let rc = chmod(path, new_mode);
    if rc < 0 {
        println!("chmod: changing permissions of '{}' failed (errno {})", path, rc);
        return false;
    }
    if verbose {
        println!("mode of '{}' changed to {:04o}", path, new_mode);
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
                                if !apply_path(child, mode_str, true, verbose, depth + 1) {
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
        println!("Usage: chmod [-R] [-v] MODE FILE...");
        return 1;
    }

    let mut verbose = false;
    let mut recursive = false;
    let mut arg_idx = 1;

    while arg_idx < argc {
        let s = unsafe { arg(argv, arg_idx) };
        if s == "-h" || s == "--help" {
            println!("Usage: chmod [-R] [-v] MODE FILE...");
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
        println!("chmod: missing operand");
        return 1;
    }

    let mode_str = unsafe { arg(argv, arg_idx) };
    arg_idx += 1;

    let mut all_ok = true;
    for i in arg_idx..argc {
        let file = unsafe { arg(argv, i) };
        if !apply_path(file, mode_str, recursive, verbose, 0) {
            all_ok = false;
        }
    }
    if all_ok { 0 } else { 1 }
}
