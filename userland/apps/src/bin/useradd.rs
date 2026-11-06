#![no_std]
#![no_main]

use libc::{
    arg, chmod, chown, close, mkdir, open, println, read, write, O_APPEND, O_CREATE, O_RDONLY,
    O_WRONLY,
};

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

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        println!("usage: useradd [-m] [-u UID] [-g GID] [-d HOME] [-s SHELL] USERNAME");
        return 1;
    }

    let mut username = "";
    let mut req_uid: Option<u32> = None;
    let mut req_gid: Option<u32> = None;
    let mut home = "";
    let mut shell = "/bin/sh";
    let mut create_home = true;

    let mut i = 1;
    while i < argc {
        let a = unsafe { arg(argv, i) };
        if a == "-h" || a == "--help" {
            println!("usage: useradd [-m] [-u UID] [-g GID] [-d HOME] [-s SHELL] USERNAME");
            return 0;
        } else if a == "-m" {
            create_home = true;
        } else if a == "-M" {
            create_home = false;
        } else if a == "-u" && i + 1 < argc {
            i += 1;
            req_uid = parse_u32(unsafe { arg(argv, i) });
        } else if a == "-g" && i + 1 < argc {
            i += 1;
            req_gid = parse_u32(unsafe { arg(argv, i) });
        } else if a == "-d" && i + 1 < argc {
            i += 1;
            home = unsafe { arg(argv, i) };
        } else if a == "-s" && i + 1 < argc {
            i += 1;
            shell = unsafe { arg(argv, i) };
        } else if !a.starts_with('-') {
            username = a;
        }
        i += 1;
    }

    if username.is_empty() || username.contains(':') || username.contains('\n') {
        println!("useradd: invalid or missing username");
        return 1;
    }

    let mut max_uid = 999u32;
    let pfd = open("/etc/passwd", O_RDONLY);
    if pfd >= 0 {
        let mut pbuf = [0u8; 512];
        let n = read(pfd as i32, &mut pbuf);
        close(pfd as i32);
        if n > 0 {
            if let Ok(text) = core::str::from_utf8(&pbuf[..n as usize]) {
                for line in text.lines() {
                    let mut p = line.trim().split(':');
                    let u = p.next().unwrap_or("");
                    if u == username {
                        println!("useradd: user '{}' already exists", username);
                        return 9;
                    }
                    let _pw = p.next();
                    if let Some(id) = p.next().and_then(parse_u32) {
                        if id >= 1000 && id < 60000 && id > max_uid {
                            max_uid = id;
                        }
                    }
                }
            }
        }
    }

    let uid = req_uid.unwrap_or(max_uid + 1);
    let gid = req_gid.unwrap_or(uid);

    let mut home_buf = [0u8; 64];
    let default_home = if home.is_empty() {
        let prefix = b"/home/";
        let mut idx = 0;
        for &b in prefix {
            home_buf[idx] = b;
            idx += 1;
        }
        for &b in username.as_bytes() {
            if idx < 63 {
                home_buf[idx] = b;
                idx += 1;
            }
        }
        core::str::from_utf8(&home_buf[..idx]).unwrap_or("/home")
    } else {
        home
    };

    let fd_passwd = open("/etc/passwd", O_WRONLY | O_CREATE | O_APPEND);
    if fd_passwd < 0 {
        println!("useradd: failed to open /etc/passwd");
        return 1;
    }
    let mut ubuf = [0u8; 16];
    let mut gbuf = [0u8; 16];
    let ubytes = fmt_u32(uid, &mut ubuf);
    let gbytes = fmt_u32(gid, &mut gbuf);

    // Locked account (`!`) until `passwd` sets a PBKDF2-HMAC-SHA256 hash.
    let _ = write(fd_passwd as i32, username.as_bytes());
    let _ = write(fd_passwd as i32, b":!:");
    let _ = write(fd_passwd as i32, ubytes);
    let _ = write(fd_passwd as i32, b":");
    let _ = write(fd_passwd as i32, gbytes);
    let _ = write(fd_passwd as i32, b":");
    let _ = write(fd_passwd as i32, username.as_bytes());
    let _ = write(fd_passwd as i32, b":");
    let _ = write(fd_passwd as i32, default_home.as_bytes());
    let _ = write(fd_passwd as i32, b":");
    let _ = write(fd_passwd as i32, shell.as_bytes());
    let _ = write(fd_passwd as i32, b"\n");
    close(fd_passwd as i32);

    // Ensure group entry exists in /etc/group
    let mut group_exists = false;
    let gfd_r = open("/etc/group", O_RDONLY);
    if gfd_r >= 0 {
        let mut grp_buf = [0u8; 512];
        let n = read(gfd_r as i32, &mut grp_buf);
        close(gfd_r as i32);
        if n > 0 {
            if let Ok(text) = core::str::from_utf8(&grp_buf[..n as usize]) {
                for line in text.lines() {
                    if line.trim().split(':').next() == Some(username) {
                        group_exists = true;
                        break;
                    }
                }
            }
        }
    }
    if !group_exists {
        let fd_group = open("/etc/group", O_WRONLY | O_CREATE | O_APPEND);
        if fd_group >= 0 {
            let _ = write(fd_group as i32, username.as_bytes());
            let _ = write(fd_group as i32, b":x:");
            let _ = write(fd_group as i32, gbytes);
            let _ = write(fd_group as i32, b":\n");
            close(fd_group as i32);
        }
    }

    if create_home {
        let _ = mkdir("/home");
        let _ = mkdir(default_home);
        let _ = chown(default_home, uid, gid);
        let _ = chmod(default_home, 0o750);
    }

    println!(
        "useradd: user '{}' added (UID={}, GID={}, HOME='{}')",
        username, uid, gid, default_home
    );
    0
}
