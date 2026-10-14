#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read, O_RDONLY};

fn get_current_user(buf: &mut [u8; 32]) -> &str {
    let fd = open("/etc/passwd", O_RDONLY);
    if fd >= 0 {
        let mut pbuf = [0u8; 128];
        let n = read(fd as i32, &mut pbuf);
        let _ = close(fd as i32);
        if n > 0 {
            if let Ok(s) = core::str::from_utf8(&pbuf[..n as usize]) {
                if let Some(first_line) = s.split('\n').next() {
                    if let Some(user) = first_line.split(':').next() {
                        let trimmed = user.trim();
                        if !trimmed.is_empty() {
                            let len = core::cmp::min(trimmed.len(), buf.len());
                            buf[..len].copy_from_slice(&trimmed.as_bytes()[..len]);
                            return core::str::from_utf8(&buf[..len]).unwrap_or("youareme");
                        }
                    }
                }
            }
        }
    }
    "youareme"
}

fn print_user_groups(user: &str, print_prefix: bool) {
    if print_prefix {
        print!("{} :", user);
    }

    let fd = open("/etc/group", O_RDONLY);
    let mut found_count = 0;

    if fd >= 0 {
        let mut gbuf = [0u8; 512];
        let n = read(fd as i32, &mut gbuf);
        let _ = close(fd as i32);
        if n > 0 {
            if let Ok(s) = core::str::from_utf8(&gbuf[..n as usize]) {
                for line in s.split('\n') {
                    let trimmed = line.trim();
                    if trimmed.is_empty() || trimmed.starts_with('#') {
                        continue;
                    }
                    let mut parts = trimmed.split(':');
                    let gname = parts.next().unwrap_or("");
                    let _pass = parts.next();
                    let _gid = parts.next();
                    let members = parts.next().unwrap_or("");

                    let mut is_member = gname.eq_ignore_ascii_case(user);
                    if !is_member {
                        for m in members.split(',') {
                            if m.trim().eq_ignore_ascii_case(user) {
                                is_member = true;
                                break;
                            }
                        }
                    }

                    if is_member && !gname.is_empty() {
                        print!(" ");
                        print!("{}", gname);
                        found_count += 1;
                    }
                }
            }
        }
    }

    if found_count == 0 {
        print!(" ");
        print!("{}", user);
    }
    println!("");
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc > 1 {
        let first = unsafe { arg(argv, 1) };
        if first == "--help" || first == "-h" {
            println!("Usage: groups [USERNAME...]");
            println!("Print group memberships for USERNAME or the current user");
            return 0;
        }

        let mut i = 1;
        while i < argc {
            let u = unsafe { arg(argv, i) };
            if !u.is_empty() {
                print_user_groups(u, true);
            }
            i += 1;
        }
    } else {
        let mut ubuf = [0u8; 32];
        let u = get_current_user(&mut ubuf);
        print_user_groups(u, false);
    }

    0
}
