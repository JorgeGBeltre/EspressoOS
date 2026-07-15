#![no_std]
#![no_main]

use libc::{println, open, read, close};

#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    let mut buf = [0u8; 2048];
    let fd = open("/proc/net/sockets", 0);
    if fd < 0 {
        println!("netstat: could not open /proc/net/sockets");
        return 1;
    }
    
    let n = read(fd as i32, &mut buf);
    if n > 0 {
        let content = unsafe { core::str::from_utf8_unchecked(&buf[..n as usize]) };
        println!("{}", content);
    } else {
        println!("netstat: no active sockets or read error");
    }
    
    close(fd as i32);
    0
}
