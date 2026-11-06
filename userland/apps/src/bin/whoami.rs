#![no_std]
#![no_main]

use libc::{close, getuid_gid, open, println, read, O_RDONLY};

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

#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    let (uid, _gid) = getuid_gid();

    let fd = open("/etc/passwd", O_RDONLY);
    if fd >= 0 {
        let mut buf = [0u8; 512];
        let n = read(fd as i32, &mut buf);
        close(fd as i32);
        if n > 0 {
            if let Ok(text) = core::str::from_utf8(&buf[..n as usize]) {
                let mut legacy_first = "";
                for line in text.lines() {
                    let trimmed = line.trim();
                    if trimmed.is_empty() || trimmed.starts_with('#') {
                        continue;
                    }
                    let mut p = trimmed.split(':');
                    let uname = p.next().unwrap_or("");
                    let _pw = p.next();
                    if let Some(uid_str) = p.next() {
                        if parse_u32(uid_str) == Some(uid) && !uname.is_empty() {
                            println!("{}", uname);
                            return 0;
                        }
                    } else if legacy_first.is_empty() && !uname.is_empty() {
                        legacy_first = uname;
                    }
                }
                if uid == 0 && !legacy_first.is_empty() {
                    println!("{}", legacy_first);
                    return 0;
                }
            }
        }
    }

    if uid == 0 {
        println!("root");
    } else {
        println!("uid{}", uid);
    }
    0
}
