#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read, O_RDONLY};

fn fnv1a(bytes: &[u8]) -> u32 {
    let mut hash = 0x811c9dc5u32;
    for &b in bytes {
        hash ^= b as u32;
        hash = hash.wrapping_mul(0x01000193);
    }
    hash
}

fn print_hex8(val: u32) {
    const HEX: &[u8] = b"0123456789abcdef";
    for i in (0..8).rev() {
        let nibble = ((val >> (i * 4)) & 0x0F) as usize;
        let c = HEX[nibble] as char;
        print!("{}", c);
    }
    println!("");
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc > 1 {
        let a = unsafe { arg(argv, 1) };
        if a == "--help" || a == "-h" {
            println!("Usage: hostid");
            println!("Print the numeric identifier (in hexadecimal) for the current host");
            return 0;
        }
    }

    let mut name_buf = [0u8; 64];
    let mut nlen = 0;

    let fd = open("/etc/hostname", O_RDONLY);
    if fd >= 0 {
        let n = read(fd as i32, &mut name_buf);
        let _ = close(fd as i32);
        if n > 0 {
            let mut end = n as usize;
            while end > 0 && (name_buf[end - 1] == b'\n' || name_buf[end - 1] == b'\r' || name_buf[end - 1] == b' ') {
                end -= 1;
            }
            nlen = end;
        }
    }

    let id = if nlen > 0 {
        fnv1a(&name_buf[..nlen])
    } else {
        fnv1a(b"esp32s3")
    };

    print_hex8(id);
    0
}
