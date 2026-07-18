#![no_std]
#![no_main]

use libc::{arg, close, ioctl, open, print, println};

const O_RDONLY: u32 = 1;
const I2C_PROBE: u32 = 0;
const I2C_READ: u32 = 1;
const I2C_WRITE: u32 = 2;

/// Espejo del struct del kernel (drivers::i2c::I2cReq, D-1).
#[repr(C)]
struct I2cReq {
    addr: usize,
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

fn parse_dec(s: &str) -> Option<usize> {
    let mut v = 0usize;
    let mut any = false;
    for b in s.bytes() {
        if b.is_ascii_digit() {
            v = v * 10 + (b - b'0') as usize;
            any = true;
        } else {
            return None;
        }
    }
    if any {
        Some(v)
    } else {
        None
    }
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        println!("usage: i2c scan | read ADDR LEN | write ADDR B0 [B1 ...]");
        return 1;
    }
    let fd = open("/dev/i2c0", O_RDONLY);
    if fd < 0 {
        println!("i2c: cannot open /dev/i2c0");
        return 1;
    }
    let fd = fd as i32;

    let r = match unsafe { arg(argv, 1) } {
        "scan" => {
            println!("Scanning I2C bus (0x08..0x77)...");
            let mut found = 0;
            for addr in 0x08u32..=0x77 {
                if ioctl(fd, I2C_PROBE, addr as usize) > 0 {
                    println!("  device at 0x{:02x}", addr);
                    found += 1;
                }
            }
            println!("{} device(s) found", found);
            0
        }
        "read" => {
            let addr = if argc > 2 { parse_hex_u8(unsafe { arg(argv, 2) }) } else { None };
            let len = if argc > 3 { parse_dec(unsafe { arg(argv, 3) }) } else { None };
            match (addr, len) {
                (Some(a), Some(l)) if (1..=64).contains(&l) => {
                    let mut buf = [0u8; 64];
                    let req = I2cReq { addr: a as usize, buf_ptr: buf.as_ptr() as usize, len: l };
                    let rc = ioctl(fd, I2C_READ, &req as *const I2cReq as usize);
                    if rc < 0 {
                        println!("i2c read failed ({})", rc);
                        1
                    } else {
                        for b in &buf[..l] {
                            print!("{:02x} ", b);
                        }
                        println!("");
                        0
                    }
                }
                _ => {
                    println!("usage: i2c read ADDR LEN(1..64)");
                    1
                }
            }
        }
        "write" => {
            let addr = if argc > 2 { parse_hex_u8(unsafe { arg(argv, 2) }) } else { None };
            let mut buf = [0u8; 64];
            let mut n = 0usize;
            let mut bad = false;
            for i in 3..argc {
                if n >= 64 {
                    break;
                }
                match parse_hex_u8(unsafe { arg(argv, i) }) {
                    Some(b) => {
                        buf[n] = b;
                        n += 1;
                    }
                    None => {
                        bad = true;
                        break;
                    }
                }
            }
            match addr {
                Some(a) if n > 0 && !bad => {
                    let req = I2cReq { addr: a as usize, buf_ptr: buf.as_ptr() as usize, len: n };
                    let rc = ioctl(fd, I2C_WRITE, &req as *const I2cReq as usize);
                    if rc < 0 {
                        println!("i2c write failed ({})", rc);
                        1
                    } else {
                        println!("wrote {} bytes to 0x{:02x}", n, a);
                        0
                    }
                }
                _ => {
                    println!("usage: i2c write ADDR B0 [B1 ...]");
                    1
                }
            }
        }
        _ => {
            println!("usage: i2c scan | read ADDR LEN | write ADDR B0 [B1 ...]");
            1
        }
    };

    close(fd);
    r
}
