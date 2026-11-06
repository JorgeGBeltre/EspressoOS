#![no_std]
#![no_main]

use libc::{arg, chdir, close, getuid_gid, open, println, read, setuid_gid, spawn, waitpid, O_RDONLY};

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
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut login_shell = false;
    let mut target_user = "root";

    let mut i = 1;
    while i < argc {
        let a = unsafe { arg(argv, i) };
        if a == "-" || a == "-l" {
            login_shell = true;
        } else if !a.starts_with('-') {
            target_user = a;
        }
        i += 1;
    }

    let mut uid = 0u32;
    let mut gid = 0u32;
    let mut home_buf = [0u8; 64];
    let mut home_len = 5usize;
    home_buf[..5].copy_from_slice(b"/root");
    let mut shell_buf = [0u8; 64];
    let mut shell_len = 7usize;
    shell_buf[..7].copy_from_slice(b"/bin/sh");
    let mut found = target_user == "root";

    let fd = open("/etc/passwd", O_RDONLY);
    if fd >= 0 {
        let mut pbuf = [0u8; 512];
        let n = read(fd as i32, &mut pbuf);
        close(fd as i32);
        if n > 0 {
            if let Ok(text) = core::str::from_utf8(&pbuf[..n as usize]) {
                for line in text.lines() {
                    let mut p = line.trim().split(':');
                    let u = p.next().unwrap_or("");
                    let _pw = p.next();
                    if u == target_user {
                        uid = p.next().and_then(parse_u32).unwrap_or(0);
                        gid = p.next().and_then(parse_u32).unwrap_or(uid);
                        let _gecos = p.next();
                        if let Some(h) = p.next() {
                            if !h.is_empty() {
                                let c = h.len().min(home_buf.len());
                                home_buf[..c].copy_from_slice(&h.as_bytes()[..c]);
                                home_len = c;
                            }
                        }
                        if let Some(sh) = p.next() {
                            if !sh.is_empty() {
                                let c = sh.len().min(shell_buf.len());
                                shell_buf[..c].copy_from_slice(&sh.as_bytes()[..c]);
                                shell_len = c;
                            }
                        }
                        found = true;
                        break;
                    }
                }
            }
        }
    }

    if !found {
        println!("su: user '{}' does not exist", target_user);
        return 1;
    }

    let (caller_uid, _) = getuid_gid();
    if caller_uid != 0 && caller_uid != uid {
        println!("su: Authentication failure (UID {} cannot switch without root)", caller_uid);
        return 1;
    }

    if setuid_gid(uid, gid) < 0 {
        println!("su: setuid_gid({}:{}) failed", uid, gid);
        return 1;
    }

    if login_shell {
        if let Ok(home) = core::str::from_utf8(&home_buf[..home_len]) {
            let _ = chdir(home);
        }
    }

    let mut user_buf = [0u8; 32];
    let ulen = target_user.len().min(31);
    user_buf[..ulen].copy_from_slice(&target_user.as_bytes()[..ulen]);
    user_buf[ulen] = 0;

    let shell_path = core::str::from_utf8(&shell_buf[..shell_len]).unwrap_or("/bin/sh");
    let child_argv = [
        b"sh\0".as_ptr(),
        b"--user\0".as_ptr(),
        user_buf.as_ptr(),
        core::ptr::null(),
    ];

    let pid = spawn(shell_path, child_argv.as_ptr());
    if pid >= 0 {
        let mut status = 0;
        let _ = waitpid(pid as i32, &mut status, 0);
        status
    } else {
        println!("su: failed to spawn shell: {}", pid);
        1
    }
}
