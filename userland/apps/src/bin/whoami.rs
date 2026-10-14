#![no_std]
#![no_main]

use libc::{close, open, println, read};

const O_RDONLY: u32 = 1;

#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    let fd = open("/etc/passwd", O_RDONLY);
    if fd >= 0 {
        let mut buf = [0u8; 128];
        let n = read(fd as i32, &mut buf);
        close(fd as i32);
        if n > 0 {
            let s = core::str::from_utf8(&buf[..n as usize]).unwrap_or("");
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

    println!("youareme");
    0
}
