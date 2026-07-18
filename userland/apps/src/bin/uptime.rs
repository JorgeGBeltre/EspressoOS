#![no_std]
#![no_main]

use libc::{close, open, println, read};

const O_RDONLY: u32 = 1;

fn read_file(path: &str, buf: &mut [u8]) -> usize {
    let fd = open(path, O_RDONLY);
    if fd < 0 {
        return 0;
    }
    let mut total = 0usize;
    while total < buf.len() {
        let n = read(fd as i32, &mut buf[total..]);
        if n <= 0 {
            break;
        }
        total += n as usize;
    }
    close(fd as i32);
    total
}

/// Primer entero decimal en `s` (salta el texto que lo precede).
fn parse_u64(s: &str) -> u64 {
    let mut v = 0u64;
    let mut seen = false;
    for b in s.bytes() {
        if b.is_ascii_digit() {
            v = v.wrapping_mul(10).wrapping_add((b - b'0') as u64);
            seen = true;
        } else if seen {
            break;
        }
    }
    v
}

/// uptime(1): lee `/proc/uptime` (D-8) y lo formatea en días/h/m/s como el builtin.
#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    let mut buf = [0u8; 64];
    let n = read_file("/proc/uptime", &mut buf);
    let ms = parse_u64(core::str::from_utf8(&buf[..n]).unwrap_or(""));
    let s = ms / 1000;
    println!(
        "up {} days, {:02}:{:02}:{:02}",
        s / 86_400,
        (s % 86_400) / 3_600,
        (s % 3_600) / 60,
        s % 60
    );
    0
}
