#![no_std]
#![no_main]

use libc::{
    arg, close, open, println, read, readdir, rename, unlink, write, O_CREATE, O_RDONLY, O_TRUNC,
    O_WRONLY, PATH_MAX,
};

fn remove_dir_contents(path: &str) {
    let mut dbuf = [0u8; 512];
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
                        cbuf[..pb.len()].copy_from_slice(pb);
                        cbuf[pb.len()] = b'/';
                        cbuf[pb.len() + 1..pb.len() + 1 + nb.len()].copy_from_slice(nb);
                        if let Ok(full) = core::str::from_utf8(&cbuf[..pb.len() + 1 + nb.len()]) {
                            let _ = unlink(full);
                        }
                    }
                }
            }
            p += nlen;
        }
    }
    let _ = unlink(path);
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        println!("usage: userdel [-r] USERNAME");
        return 1;
    }

    let mut remove_home = false;
    let mut username = "";
    let mut i = 1;
    while i < argc {
        let a = unsafe { arg(argv, i) };
        if a == "-h" || a == "--help" {
            println!("usage: userdel [-r] USERNAME");
            return 0;
        } else if a == "-r" || a == "--remove" {
            remove_home = true;
        } else if !a.starts_with('-') {
            username = a;
        }
        i += 1;
    }

    if username.is_empty() {
        println!("userdel: missing username");
        return 1;
    }
    if username == "root" {
        println!("userdel: refusing to delete root account");
        return 1;
    }

    let pfd = open("/etc/passwd", O_RDONLY);
    if pfd < 0 {
        println!("userdel: user '{}' does not exist", username);
        return 6;
    }
    let mut pbuf = [0u8; 512];
    let n = read(pfd as i32, &mut pbuf);
    close(pfd as i32);
    if n <= 0 {
        println!("userdel: user '{}' does not exist", username);
        return 6;
    }

    let mut home_buf = [0u8; 64];
    let mut home_len = 0usize;
    let mut found = false;

    let tfd = open("/etc/passwd.tmp", O_WRONLY | O_CREATE | O_TRUNC);
    if tfd < 0 {
        println!("userdel: cannot write /etc/passwd.tmp");
        return 1;
    }
    if let Ok(text) = core::str::from_utf8(&pbuf[..n as usize]) {
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let mut p = trimmed.split(':');
            let u = p.next().unwrap_or("");
            if u == username {
                found = true;
                let _pw = p.next();
                let _uid = p.next();
                let _gid = p.next();
                let _gecos = p.next();
                if let Some(h) = p.next() {
                    let c = h.len().min(home_buf.len());
                    home_buf[..c].copy_from_slice(&h.as_bytes()[..c]);
                    home_len = c;
                }
            } else {
                let _ = write(tfd as i32, trimmed.as_bytes());
                let _ = write(tfd as i32, b"\n");
            }
        }
    }
    close(tfd as i32);

    if !found {
        let _ = unlink("/etc/passwd.tmp");
        println!("userdel: user '{}' does not exist", username);
        return 6;
    }
    let _ = rename("/etc/passwd.tmp", "/etc/passwd");

    // Also prune matching primary group in /etc/group
    let gfd = open("/etc/group", O_RDONLY);
    if gfd >= 0 {
        let mut gbuf = [0u8; 512];
        let gn = read(gfd as i32, &mut gbuf);
        close(gfd as i32);
        if gn > 0 {
            let gtfd = open("/etc/group.tmp", O_WRONLY | O_CREATE | O_TRUNC);
            if gtfd >= 0 {
                if let Ok(gtext) = core::str::from_utf8(&gbuf[..gn as usize]) {
                    for line in gtext.lines() {
                        let t = line.trim();
                        if t.is_empty() {
                            continue;
                        }
                        if t.split(':').next() != Some(username) {
                            let _ = write(gtfd as i32, t.as_bytes());
                            let _ = write(gtfd as i32, b"\n");
                        }
                    }
                }
                close(gtfd as i32);
                let _ = rename("/etc/group.tmp", "/etc/group");
            }
        }
    }

    if remove_home && home_len > 0 {
        if let Ok(h) = core::str::from_utf8(&home_buf[..home_len]) {
            if h.starts_with("/home/") && h.len() > 6 {
                remove_dir_contents(h);
            }
        }
    }

    println!("userdel: user '{}' removed", username);
    0
}

