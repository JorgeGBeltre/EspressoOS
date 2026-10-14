#![no_std]
#![no_main]

use libc::{arg, close, open, println, read, O_RDONLY};

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc > 1 {
        let a = unsafe { arg(argv, 1) };
        if a == "--help" || a == "-h" {
            println!("Usage: logname");
            println!("Print user's login name");
            return 0;
        }
    }

    let fd = open("/etc/passwd", O_RDONLY);
    if fd >= 0 {
        let mut buf = [0u8; 128];
        let n = read(fd as i32, &mut buf);
        let _ = close(fd as i32);
        if n > 0 {
            if let Ok(s) = core::str::from_utf8(&buf[..n as usize]) {
                if let Some(first_line) = s.split('\n').next() {
                    if let Some(user) = first_line.split(':').next() {
                        let trimmed = user.trim();
                        if !trimmed.is_empty() {
                            println!("{}", trimmed);
                            return 0;
                        }
                    }
                }
            }
        }
    }

    println!("youareme");
    0
}
