#![no_std]
#![no_main]

use libc::{arg, close, open, println, read, write};

const O_RDONLY: u32 = 1;
const O_WRONLY: u32 = 2;
const O_CREATE: u32 = 0x0100;
const O_TRUNC: u32 = 0x0400;

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        let fd = open("/etc/hostname", O_RDONLY);
        if fd >= 0 {
            let mut buf = [0u8; 64];
            let n = read(fd as i32, &mut buf);
            close(fd as i32);
            if n > 0 {
                let s = core::str::from_utf8(&buf[..n as usize]).unwrap_or("");
                let trimmed = s.trim();
                if !trimmed.is_empty() {
                    println!("{}", trimmed);
                    return 0;
                }
            }
        }
        println!("esp32s3");
        return 0;
    }

    let first = unsafe { arg(argv, 1) };
    if first == "-h" || first == "--help" {
        println!("Usage: hostname [NAME]");
        println!("Show or set the system's host name.");
        return 0;
    }

    // Set hostname
    let fd = open("/etc/hostname", O_WRONLY | O_CREATE | O_TRUNC);
    if fd < 0 {
        println!("hostname: cannot open /etc/hostname for writing");
        return 1;
    }
    let _ = write(fd as i32, first.as_bytes());
    let _ = write(fd as i32, b"\n");
    close(fd as i32);
    0
}
