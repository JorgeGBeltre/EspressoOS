#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, uptime_ms, write};

const O_WRONLY_CREATE_APPEND: u32 = 2 | 0x100 | 0x200;

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
        println!("usage: logger [-s] MESSAGE...");
        return 1;
    }

    let mut start_idx = 1;
    let mut echo_stderr = false;

    let first = unsafe { arg(argv, 1) };
    if first == "-h" || first == "--help" {
        println!("usage: logger [-s] MESSAGE...");
        println!("Add entries to the system log (/var/log/syslog)");
        println!("  -s  Log the message to standard error as well as system log");
        return 0;
    }
    if first == "-s" {
        echo_stderr = true;
        start_idx = 2;
        if argc < 3 {
            println!("usage: logger [-s] MESSAGE...");
            return 1;
        }
    }

    let ms = uptime_ms();
    let sec = ms / 1000;
    let frac = ms % 1000;

    let mut log_buf = [0u8; 256];
    let mut idx = 0;

    // Format: "[sec.frac] "
    if idx < log_buf.len() { log_buf[idx] = b'['; idx += 1; }
    idx = write_num(&mut log_buf, idx, sec, 1);
    if idx < log_buf.len() { log_buf[idx] = b'.'; idx += 1; }
    idx = write_num(&mut log_buf, idx, frac, 3);
    if idx < log_buf.len() { log_buf[idx] = b']'; idx += 1; }
    if idx < log_buf.len() { log_buf[idx] = b' '; idx += 1; }

    // Append arguments
    for i in start_idx..argc {
        let word = unsafe { arg(argv, i) };
        if i > start_idx && idx < log_buf.len() - 2 {
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

    if idx < log_buf.len() {
        log_buf[idx] = b'\n';
        idx += 1;
    }

    let log_slice = &log_buf[..idx];

    if echo_stderr {
        if let Ok(s) = core::str::from_utf8(log_slice) {
            print!("{}", s);
        }
    }

    let fd = open("/var/log/syslog", O_WRONLY_CREATE_APPEND);
    if fd >= 0 {
        let _ = write(fd as i32, log_slice);
        close(fd as i32);
        0
    } else {
        println!("logger: cannot open /var/log/syslog");
        1
    }
}
