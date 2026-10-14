#![no_std]
#![no_main]

use libc::{close, open, println, read, write};

const O_RDONLY: u32 = 1;
const STDOUT: i32 = 1;

#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    let fd = open("/proc/dmesg", O_RDONLY);
    if fd < 0 {
        println!("dmesg: cannot open /proc/dmesg");
        return 1;
    }
    let mut buf = [0u8; 256];
    loop {
        let n = read(fd as i32, &mut buf);
        if n <= 0 {
            break;
        }
        let _ = write(STDOUT, &buf[..n as usize]);
    }
    let _ = close(fd as i32);
    0
}
