#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read, O_RDONLY};

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut show_heading = false;
    let mut i = 1;
    while i < argc {
        let s = unsafe { arg(argv, i) };
        if s == "-h" || s == "--help" {
            println!("Usage: who [-H]");
            println!("Show who is logged on");
            println!("  -H, --heading  Print line of column headings");
            return 0;
        } else if s == "-H" || s == "--heading" {
            show_heading = true;
        }
        i += 1;
    }

    if show_heading {
        println!("NAME     LINE     STATUS   ID");
    }

    let fd = open("/proc/sessions", O_RDONLY);
    if fd < 0 {
        // Fallback default: serial console is always active
        println!("console  tty0     active   (id: 0)");
        return 0;
    }

    let mut buf = [0u8; 512];
    let mut line_buf = [0u8; 128];
    let mut line_len = 0usize;
    let mut line_num = 0usize;

    loop {
        let n = read(fd as i32, &mut buf);
        if n <= 0 {
            break;
        }
        for &b in &buf[..n as usize] {
            if b == b'\n' {
                if line_num > 0 && line_len > 0 {
                    // Skip header line (line_num == 0)
                    // Format in /proc/sessions: ID\tUSER\tTTY\tSTATUS
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

                    let id = parts[0];
                    let user = parts[1];
                    let tty = parts[2];
                    let status = parts[3];

                    if !user.is_empty() {
                        print!("{}", user);
                        let pad = 9usize.saturating_sub(user.len());
                        for _ in 0..pad { print!(" "); }
                        print!("{}", tty);
                        let pad2 = 9usize.saturating_sub(tty.len());
                        for _ in 0..pad2 { print!(" "); }
                        print!("{}", status);
                        let pad3 = 9usize.saturating_sub(status.len());
                        for _ in 0..pad3 { print!(" "); }
                        print!("(id: ");
                        print!("{}", id);
                        println!(")");
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
    0
}
