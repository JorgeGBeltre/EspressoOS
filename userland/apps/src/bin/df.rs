#![no_std]
#![no_main]

use libc::{close, open, println, read, O_RDONLY};

fn parse_u64(s: &str) -> u64 {
    let mut v = 0u64;
    for &b in s.as_bytes() {
        if b.is_ascii_digit() {
            v = v.saturating_mul(10).saturating_add((b - b'0') as u64);
        } else {
            break;
        }
    }
    v
}

#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    println!("Filesystem     Type       1K-blocks      Used Available Use% Mounted on");

    let sfd = open("/proc/fsstat", O_RDONLY);
    if sfd >= 0 {
        let mut sbuf = [0u8; 512];
        let n = read(sfd as i32, &mut sbuf);
        close(sfd as i32);
        if n > 0 {
            if let Ok(text) = core::str::from_utf8(&sbuf[..n as usize]) {
                for line in text.lines() {
                    let mut it = line.split_whitespace();
                    let dev = match it.next() {
                        Some(v) => v,
                        None => continue,
                    };
                    let fstype = match it.next() {
                        Some(v) => v,
                        None => continue,
                    };
                    let total_b = parse_u64(it.next().unwrap_or("0"));
                    let used_b = parse_u64(it.next().unwrap_or("0"));
                    let mnt = match it.next() {
                        Some(v) => v,
                        None => continue,
                    };
                    let total_k = total_b / 1024;
                    let used_k = if used_b == 0 {
                        0
                    } else {
                        (used_b + 1023) / 1024
                    };
                    let avail_k = total_k.saturating_sub(used_k);
                    let pct = if total_b > 0 {
                        ((used_b.saturating_mul(100)) / total_b).min(100)
                    } else {
                        0
                    };
                    println!(
                        "{:<14} {:<10} {:>9} {:>9} {:>9} {:>3}% {}",
                        dev, fstype, total_k, used_k, avail_k, pct, mnt
                    );
                }
            }
        }
    }

    let fd = open("/proc/meminfo", O_RDONLY);
    if fd >= 0 {
        let mut buf = [0u8; 256];
        let n = read(fd as i32, &mut buf);
        if n > 0 {
            if let Ok(s) = core::str::from_utf8(&buf[..n as usize]) {
                println!("\nMemory subsystem status:\n{}", s.trim_end());
            }
        }
        close(fd as i32);
    }
    0
}
