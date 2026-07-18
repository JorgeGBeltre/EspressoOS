#![no_std]
#![no_main]

use libc::{arg, close, ioctl, open, print, println};

const O_RDONLY: u32 = 1;
const SPI_TRANSFER: u32 = 0;

/// Espejo del struct del kernel (drivers::spi::SpiReq, D-1).
#[repr(C)]
struct SpiReq {
    buf_ptr: usize,
    len: usize,
}

fn parse_hex_u8(s: &str) -> Option<u8> {
    let t = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")).unwrap_or(s);
    let mut v: u32 = 0;
    let mut any = false;
    for b in t.bytes() {
        let d = match b {
            b'0'..=b'9' => b - b'0',
            b'a'..=b'f' => b - b'a' + 10,
            b'A'..=b'F' => b - b'A' + 10,
            _ => return None,
        };
        v = v * 16 + d as u32;
        if v > 0xff {
            return None;
        }
        any = true;
    }
    if any {
        Some(v as u8)
    } else {
        None
    }
}

/// spi(1): `spi transfer B0 [B1 ...]` — full-duplex; imprime los bytes recibidos.
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 3 || unsafe { arg(argv, 1) } != "transfer" {
        println!("usage: spi transfer B0 [B1 ...]");
        return 1;
    }
    let mut buf = [0u8; 64];
    let mut n = 0usize;
    for i in 2..argc {
        if n >= 64 {
            break;
        }
        match parse_hex_u8(unsafe { arg(argv, i) }) {
            Some(b) => {
                buf[n] = b;
                n += 1;
            }
            None => {
                println!("spi: bad byte '{}'", unsafe { arg(argv, i) });
                return 1;
            }
        }
    }
    if n == 0 {
        println!("usage: spi transfer B0 [B1 ...]");
        return 1;
    }

    let fd = open("/dev/spi0", O_RDONLY);
    if fd < 0 {
        println!("spi: cannot open /dev/spi0");
        return 1;
    }
    let req = SpiReq { buf_ptr: buf.as_ptr() as usize, len: n };
    let rc = ioctl(fd as i32, SPI_TRANSFER, &req as *const SpiReq as usize);
    close(fd as i32);

    if rc < 0 {
        println!("spi transfer failed ({})", rc);
        return 1;
    }
    for b in &buf[..n] {
        print!("{:02x} ", b);
    }
    println!("");
    0
}
