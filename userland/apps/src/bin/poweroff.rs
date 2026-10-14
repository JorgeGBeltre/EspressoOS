#![no_std]
#![no_main]

use libc::{arg, close, ioctl, open, println};

const O_RDONLY: u32 = 1;
const POWER_DEEP_SLEEP: u32 = 1;

#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    println!("Powering down system...");
    let fd = open("/dev/power", O_RDONLY);
    if fd < 0 {
        println!("poweroff: cannot open /dev/power");
        return 1;
    }
    let rc = ioctl(fd as i32, POWER_DEEP_SLEEP, 86400);
    close(fd as i32);
    if rc < 0 {
        println!("poweroff: failed ({})", rc);
        return 1;
    }
    0
}
