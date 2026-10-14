#![no_std]
#![no_main]

use libc::{close, open, print, println, read, uptime_ms, O_RDONLY};

fn print_dec(mut n: usize) {
    if n == 0 {
        print!("0");
        return;
    }
    let mut buf = [0u8; 16];
    let mut i = 0;
    while n > 0 {
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        i += 1;
    }
    while i > 0 {
        i -= 1;
        let s = core::slice::from_ref(&buf[i]);
        if let Ok(c) = core::str::from_utf8(s) {
            print!("{}", c);
        }
    }
}

#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    let ms = uptime_ms();
    let sec = ms / 1000;
    let min = sec / 60;
    let hrs = min / 60;
    let rem_min = min % 60;

    print!(" up ");
    if hrs > 0 {
        print_dec(hrs);
        print!(":");
        if rem_min < 10 {
            print!("0");
        }
        print_dec(rem_min);
    } else {
        print_dec(rem_min);
        print!(" min");
    }

    // Read load average from /proc/loadavg
    let l_fd = open("/proc/loadavg", O_RDONLY);
    if l_fd >= 0 {
        let mut l_buf = [0u8; 64];
        let n = read(l_fd as i32, &mut l_buf);
        close(l_fd as i32);
        if n > 0 {
            let l_str = core::str::from_utf8(&l_buf[..n as usize]).unwrap_or("");
            // e.g. "0.04 0.02 0.01 1/7 12"
            let mut parts = l_str.split_whitespace();
            let l1 = parts.next().unwrap_or("0.00");
            let l2 = parts.next().unwrap_or("0.00");
            let l3 = parts.next().unwrap_or("0.00");
            print!(",  load average: ");
            print!("{}, {}, {}", l1, l2, l3);
        }
    }
    println!("");

    println!("USER     TTY      FROM             LOGIN@   IDLE   WHAT");

    let fd = open("/proc/sessions", O_RDONLY);
    if fd < 0 {
        println!("console  tty0     uart0            boot     0.00s  sh");
        return 0;
    }

    let mut buf = [0u8; 512];
    let mut line_buf = [0u8; 128];
    let mut line_len = 0usize;
    let mut line_num = 0usize;
    let mut session_count = 0usize;

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
                    let tty = parts[2];
                    if !user.is_empty() {
                        session_count += 1;
                        print!("{}", user);
                        for _ in 0..9usize.saturating_sub(user.len()) { print!(" "); }
                        print!("{}", tty);
                        for _ in 0..9usize.saturating_sub(tty.len()) { print!(" "); }
                        let from = if tty == "tty0" { "uart0" } else { "ssh" };
                        print!("{}", from);
                        for _ in 0..17usize.saturating_sub(from.len()) { print!(" "); }
                        print!("boot     0.00s  sh\n");
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

    if session_count == 0 {
        println!("console  tty0     uart0            boot     0.00s  sh");
    }
    0
}
