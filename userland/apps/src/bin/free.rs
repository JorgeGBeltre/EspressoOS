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

/// Valor numérico de la línea con `prefix` en el texto.
fn field(text: &str, prefix: &str) -> u64 {
    for line in text.split('\n') {
        if let Some(rest) = line.strip_prefix(prefix) {
            return parse_u64(rest);
        }
    }
    0
}

/// free(1): lee `/proc/meminfo` (D-8: heap + slots de PSRAM) con el formato del builtin.
#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    let mut buf = [0u8; 256];
    let n = read_file("/proc/meminfo", &mut buf);
    let text = core::str::from_utf8(&buf[..n]).unwrap_or("");

    let total = field(text, "MemTotal:");
    let used = field(text, "MemUsed:");
    let free = field(text, "MemFree:");
    let st = field(text, "SlotsTotal:");
    let su = field(text, "SlotsUsed:");

    println!("            total         used         free");
    println!("heap  {:>11}  {:>11}  {:>11}", total, used, free);
    println!("slots {:>11}  {:>11}  {:>11}", st, su, st.saturating_sub(su));
    0
}
