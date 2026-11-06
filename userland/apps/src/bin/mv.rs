#![no_std]
#![no_main]

use libc::{
    arg, chmod, close, lstat, open, println, read, rename, stat, unlink, write, StatBuf,
    O_CREATE, O_RDONLY, O_TRUNC, O_WRONLY, PATH_MAX,
};

fn basename(p: &str) -> &str {
    let trimmed = p.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(idx) => &trimmed[idx + 1..],
        None => trimmed,
    }
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut verbose = false;
    let mut pos = [""; 2];
    let mut pcount = 0usize;

    let mut i = 1;
    while i < argc {
        let a = unsafe { arg(argv, i) };
        if a == "-h" || a == "--help" {
            println!("Usage: mv [-f] [-v] <source> <destination>");
            return 0;
        } else if a == "-v" || a == "--verbose" {
            verbose = true;
        } else if a == "-f" || a == "--force" {
            // default overwrite behavior
        } else if pcount < 2 {
            pos[pcount] = a;
            pcount += 1;
        }
        i += 1;
    }

    if pcount < 2 {
        println!("Usage: mv [-f] [-v] <source> <destination>");
        return 1;
    }

    let src = pos[0];
    let raw_dst = pos[1];
    if src == raw_dst {
        println!("mv: '{}' and '{}' are the same file", src, raw_dst);
        return 1;
    }

    let mut src_st = StatBuf::zeroed();
    if lstat(src, &mut src_st) < 0 {
        println!("mv: cannot stat '{}': No such file or directory", src);
        return 1;
    }

    // If destination is an existing directory, move into dst/basename(src).
    let mut dst_buf = [0u8; PATH_MAX];
    let mut dst_st = StatBuf::zeroed();
    let dst: &str = if stat(raw_dst, &mut dst_st) == 0 && dst_st.kind == 2 {
        let base = basename(src);
        let dbytes = raw_dst.trim_end_matches('/').as_bytes();
        let bbytes = base.as_bytes();
        if dbytes.len() + 1 + bbytes.len() < dst_buf.len() {
            let mut k = 0;
            dst_buf[..dbytes.len()].copy_from_slice(dbytes);
            k += dbytes.len();
            dst_buf[k] = b'/';
            k += 1;
            dst_buf[k..k + bbytes.len()].copy_from_slice(bbytes);
            k += bbytes.len();
            core::str::from_utf8(&dst_buf[..k]).unwrap_or(raw_dst)
        } else {
            raw_dst
        }
    } else {
        raw_dst
    };

    let rc = rename(src, dst);
    if rc == 0 {
        if verbose {
            println!("renamed '{}' -> '{}'", src, dst);
        }
        return 0;
    }

    // Cross-mount fallback (e.g. /tmp ramfs -> / espfs): copy regular file + unlink.
    if rc != -95 {
        println!("mv: cannot move '{}' to '{}' (errno {})", src, dst, rc);
        return 1;
    }

    let in_fd = open(src, O_RDONLY);
    if in_fd < 0 {
        println!("mv: cannot open '{}'", src);
        return 1;
    }
    let out_fd = open(dst, O_WRONLY | O_CREATE | O_TRUNC);
    if out_fd < 0 {
        close(in_fd as i32);
        println!("mv: cannot create destination '{}'", dst);
        return 1;
    }

    let mut buf = [0u8; 512];
    let mut err = false;
    loop {
        let n = read(in_fd as i32, &mut buf);
        if n < 0 {
            err = true;
            break;
        }
        if n == 0 {
            break;
        }
        if write(out_fd as i32, &buf[..n as usize]) < n {
            err = true;
            break;
        }
    }
    close(in_fd as i32);
    close(out_fd as i32);

    if err {
        let _ = unlink(dst);
        println!("mv: I/O error moving '{}' to '{}'", src, dst);
        return 1;
    }

    let _ = chmod(dst, src_st.mode as u32);
    if unlink(src) < 0 {
        println!("mv: copied to '{}' but failed to remove '{}'", dst, src);
        return 1;
    }
    if verbose {
        println!("copied '{}' -> '{}'", src, dst);
    }
    0
}
