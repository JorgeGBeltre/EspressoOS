#![no_std]
#![no_main]

use libc::{close, open, print, println, read, O_RDONLY};

#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    let mut buf = [0u8; 2048];
    let fd = open("/proc/net/sockets", O_RDONLY);
    if fd < 0 {
        println!("netstat: could not open /proc/net/sockets");
        return 1;
    }

    let n = read(fd as i32, &mut buf);
    close(fd as i32);
    if n > 0 {
        if let Ok(content) = core::str::from_utf8(&buf[..n as usize]) {
            print!("{}", content);
            if !content.ends_with('\n') {
                println!("");
            }
        }
    } else {
        println!("netstat: no active sockets");
    }
    0
}
