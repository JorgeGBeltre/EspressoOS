#![no_std]
#![no_main]

use libc::{arg, lstat, print, println, readlink, stat, StatBuf, PATH_MAX};

fn mode_string(kind: u8, mode: u16, out: &mut [u8; 10]) {
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

fn kind_name(kind: u8) -> &'static str {
    match kind {
        1 => "regular file",
        2 => "directory",
        3 => "character device",
        4 => "symbolic link",
        5 => "fifo",
        _ => "unknown",
    }
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut deref = false;
    let mut files = [""; 8];
    let mut fcount = 0usize;

    let mut i = 1;
    while i < argc {
        let a = unsafe { arg(argv, i) };
        if a == "-h" || a == "--help" {
            println!("Usage: stat [-L] FILE...");
            return 0;
        } else if a == "-L" || a == "--dereference" {
            deref = true;
        } else if fcount < files.len() {
            files[fcount] = a;
            fcount += 1;
        }
        i += 1;
    }

    if fcount == 0 {
        println!("Usage: stat [-L] FILE...");
        return 1;
    }

    let mut status = 0;
    for k in 0..fcount {
        let path = files[k];
        let mut st = StatBuf::zeroed();
        let rc = if deref { stat(path, &mut st) } else { lstat(path, &mut st) };
        if rc < 0 {
            println!("stat: cannot stat '{}': No such file or directory", path);
            status = 1;
            continue;
        }

        if st.kind == 4 {
            let mut lbuf = [0u8; PATH_MAX];
            let n = readlink(path, &mut lbuf);
            if n > 0 {
                if let Ok(target) = core::str::from_utf8(&lbuf[..n as usize]) {
                    println!("  File: {} -> {}", path, target);
                } else {
                    println!("  File: {}", path);
                }
            } else {
                println!("  File: {}", path);
            }
        } else {
            println!("  File: {}", path);
        }

        println!(
            "  Size: {:<10} Inode: {:<8} Links: {}  Type: {}",
            st.size,
            st.ino,
            st.nlink,
            kind_name(st.kind)
        );

        let mut mstr = [0u8; 10];
        mode_string(st.kind, st.mode, &mut mstr);
        let sym = core::str::from_utf8(&mstr).unwrap_or("----------");
        print!("Access: ({:04o}/{})  ", st.mode & 0o7777, sym);
        println!("Uid: ({})   Gid: ({})", st.uid, st.gid);
        println!("Modify: {} s", st.mtime);
    }
    status
}
