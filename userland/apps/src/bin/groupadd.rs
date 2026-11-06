#![no_std]
#![no_main]

use libc::{arg, close, open, println, read, write, O_APPEND, O_CREATE, O_RDONLY, O_WRONLY};

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
        println!("usage: groupadd [-g GID] GROUP");
        return 1;
    }

    let mut req_gid: Option<u32> = None;
    let mut group = "";
    let mut i = 1;
    while i < argc {
        let a = unsafe { arg(argv, i) };
        if a == "-h" || a == "--help" {
            println!("usage: groupadd [-g GID] GROUP");
            return 0;
        } else if a == "-g" && i + 1 < argc {
            i += 1;
            req_gid = parse_u32(unsafe { arg(argv, i) });
        } else if !a.starts_with('-') {
            group = a;
        }
        i += 1;
    }

    if group.is_empty() || group.contains(':') || group.contains('\n') {
        println!("groupadd: invalid or missing group name");
        return 1;
    }

    let mut max_gid = 999u32;
    let fd_r = open("/etc/group", O_RDONLY);
    if fd_r >= 0 {
        let mut gbuf = [0u8; 512];
        let n = read(fd_r as i32, &mut gbuf);
        close(fd_r as i32);
        if n > 0 {
            if let Ok(text) = core::str::from_utf8(&gbuf[..n as usize]) {
                for line in text.lines() {
                    let mut p = line.trim().split(':');
                    let g = p.next().unwrap_or("");
                    if g == group {
                        println!("groupadd: group '{}' already exists", group);
                        return 9;
                    }
                    let _pw = p.next();
                    if let Some(id) = p.next().and_then(parse_u32) {
                        if id >= 1000 && id < 60000 && id > max_gid {
                            max_gid = id;
                        }
                    }
                }
            }
        }
    }

    let gid = req_gid.unwrap_or(max_gid + 1);
    let fd_w = open("/etc/group", O_WRONLY | O_CREATE | O_APPEND);
    if fd_w < 0 {
        println!("groupadd: cannot open /etc/group");
        return 1;
    }

    let mut id_buf = [0u8; 16];
    let id_bytes = fmt_u32(gid, &mut id_buf);
    let _ = write(fd_w as i32, group.as_bytes());
    let _ = write(fd_w as i32, b":x:");
    let _ = write(fd_w as i32, id_bytes);
    let _ = write(fd_w as i32, b":\n");
    close(fd_w as i32);

    println!("groupadd: group '{}' added with GID {}", group, gid);
    0
}

