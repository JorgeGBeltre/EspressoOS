#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, uptime_ms, write, O_APPEND, O_CREATE, O_WRONLY};

fn write_num(buf: &mut [u8], mut idx: usize, mut val: usize, min_digits: usize) -> usize {
    let mut digits = [b'0'; 10];
    let mut d_idx = 0;
    if val == 0 {
        digits[0] = b'0';
        d_idx = 1;
    } else {
        while val > 0 && d_idx < 10 {
            digits[d_idx] = b'0' + (val % 10) as u8;
            val /= 10;
            d_idx += 1;
        }
    }
    while d_idx < min_digits && d_idx < 10 {
        digits[d_idx] = b'0';
        d_idx += 1;
    }
    while d_idx > 0 && idx < buf.len() {
        d_idx -= 1;
        buf[idx] = digits[d_idx];
        idx += 1;
    }
    idx
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        println!("usage: logger [-s] [-t TAG] MESSAGE...");
        return 1;
    }

    let mut echo_stderr = false;
    let mut tag = "";
    let mut i = 1;

    while i < argc {
        let a = unsafe { arg(argv, i) };
        if a == "-h" || a == "--help" {
            println!("usage: logger [-s] [-t TAG] MESSAGE...");
            println!("Write entries to /dev/log and /var/log/syslog");
            return 0;
        } else if a == "-s" {
            echo_stderr = true;
            i += 1;
        } else if a == "-t" && i + 1 < argc {
            tag = unsafe { arg(argv, i + 1) };
            i += 2;
        } else {
            break;
        }
    }

    if i >= argc {
        println!("usage: logger [-s] [-t TAG] MESSAGE...");
        return 1;
    }

    let ms = uptime_ms();
    let sec = ms / 1000;
    let frac = ms % 1000;

    let mut log_buf = [0u8; 256];
    let mut idx = 0;

    log_buf[idx] = b'[';
    idx += 1;
    idx = write_num(&mut log_buf, idx, sec, 1);
    log_buf[idx] = b'.';
    idx += 1;
    idx = write_num(&mut log_buf, idx, frac, 3);
    log_buf[idx] = b']';
    idx += 1;
    log_buf[idx] = b' ';
    idx += 1;

    if !tag.is_empty() {
        for &b in tag.as_bytes() {
            if idx < log_buf.len() - 4 {
                log_buf[idx] = b;
                idx += 1;
            }
        }
        log_buf[idx] = b':';
        idx += 1;
        log_buf[idx] = b' ';
        idx += 1;
    }

    for k in i..argc {
        let word = unsafe { arg(argv, k) };
        if k > i && idx < log_buf.len() - 2 {
            log_buf[idx] = b' ';
            idx += 1;
        }
        for &b in word.as_bytes() {
            if idx < log_buf.len() - 2 {
                log_buf[idx] = b;
                idx += 1;
            }
        }
    }

    log_buf[idx] = b'\n';
    idx += 1;
    let log_slice = &log_buf[..idx];

    if echo_stderr {
        if let Ok(s) = core::str::from_utf8(log_slice) {
            print!("{}", s);
        }
    }

    let mut wrote = false;
    let dev_fd = open("/dev/log", O_WRONLY);
    if dev_fd >= 0 {
        let _ = write(dev_fd as i32, log_slice);
        close(dev_fd as i32);
        wrote = true;
    }

    let sys_fd = open("/var/log/syslog", O_WRONLY | O_CREATE | O_APPEND);
    if sys_fd >= 0 {
        let _ = write(sys_fd as i32, log_slice);
        close(sys_fd as i32);
        wrote = true;
    }

    if wrote { 0 } else { 1 }
}
