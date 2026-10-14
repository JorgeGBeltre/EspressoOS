#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read, O_RDONLY};

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc > 1 {
        let s = unsafe { arg(argv, 1) };
        if s == "-h" || s == "--help" {
            println!("Usage: users");
            println!("Print the user names of users currently logged in to the current host");
            return 0;
        }
    }

    let fd = open("/proc/sessions", O_RDONLY);
    if fd < 0 {
        println!("console");
        return 0;
    }

    let mut buf = [0u8; 512];
    let mut line_buf = [0u8; 128];
    let mut line_len = 0usize;
    let mut line_num = 0usize;
    let mut first = true;

    loop {
        let n = read(fd as i32, &mut buf);
        if n <= 0 {
            break;
        }
        for &b in &buf[..n as usize] {
            if b == b'\n' {
                if line_num > 0 && line_len > 0 {
                    let line_str = core::str::from_utf8(&line_buf[..line_len]).unwrap_or("");
                    let mut parts = ["", "", "", ""];
                    let mut p_idx = 0;
                    let mut start = 0;
                    for (ci, ch) in line_str.char_indices() {
                        if ch == '\t' || ch == ' ' {
                            if start < ci && p_idx < 4 {
                                parts[p_idx] = &line_str[start..ci];
                                p_idx += 1;
                            }
                            start = ci + 1;
                        }
                    }
                    if start < line_str.len() && p_idx < 4 {
                        parts[p_idx] = &line_str[start..];
                    }
                    let user = parts[1];
                    let status = parts[3];
                    if !user.is_empty() && (status.is_empty() || status == "active") {
                        if !first {
                            print!(" ");
                        }
                        print!("{}", user);
                        first = false;
                    }
                }
                line_len = 0;
                line_num += 1;
            } else if line_len < line_buf.len() {
                line_buf[line_len] = b;
                line_len += 1;
            }
        }
    }
    close(fd as i32);

    if first {
        println!("console");
    } else {
        println!("");
    }
    0
}
