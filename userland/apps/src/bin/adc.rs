#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read, write};

const O_RDONLY: u32 = 1;
const O_WRONLY: u32 = 2;

fn show_all() -> i32 {
    let fd = open("/dev/adc0", O_RDONLY);
    if fd < 0 {
        println!("adc: cannot open /dev/adc0");
        return 1;
    }
    let mut buf = [0u8; 1024];
    let n = read(fd as i32, &mut buf);
    close(fd as i32);
    if n > 0 {
        let s = core::str::from_utf8(&buf[..n as usize]).unwrap_or("");
        print!("{}", s);
        0
    } else {
        println!("adc: empty read from /dev/adc0");
        1
    }
}

fn read_channel(ch_str: &str) -> i32 {
    let fd_w = open("/dev/adc0", O_WRONLY);
    if fd_w >= 0 {
        let mut cmd = [0u8; 32];
        let mut idx = 0;
        let prefix = b"read ";
        for &b in prefix {
            cmd[idx] = b;
            idx += 1;
        }
        for &b in ch_str.as_bytes() {
            if idx < cmd.len() - 1 {
                cmd[idx] = b;
                idx += 1;
            }
        }
        let _ = write(fd_w as i32, &cmd[..idx]);
        close(fd_w as i32);
    }

    let fd_r = open("/dev/adc0", O_RDONLY);
    if fd_r < 0 {
        println!("adc: cannot open /dev/adc0 for reading");
        return 1;
    }
    let mut buf = [0u8; 1024];
    let n = read(fd_r as i32, &mut buf);
    close(fd_r as i32);

    if n > 0 {
        let s = core::str::from_utf8(&buf[..n as usize]).unwrap_or("");
        // Find line matching channel
        for line in s.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with(ch_str) {
                println!("{}", line);
                return 0;
            }
        }
        // Fallback: print full table
        print!("{}", s);
        0
    } else {
        println!("adc: channel read failed");
        1
    }
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        return show_all();
    }

    let first = unsafe { arg(argv, 1) };
    if first == "-h" || first == "--help" || first == "help" {
        println!("Usage: adc [options] [channel]");
        println!("ESP32-S3 SAR ADC reader (channels 0..9 on GPIO 1..10)");
        println!("  adc              Display readings for all channels");
        println!("  adc <ch>         Read specific ADC channel (0-9)");
        println!("  adc -a, --all    Dump all channels with gauge");
        return 0;
    }

    if first == "-a" || first == "--all" {
        show_all()
    } else {
        read_channel(first)
    }
}
