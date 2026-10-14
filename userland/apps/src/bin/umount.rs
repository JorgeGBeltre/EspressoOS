#![no_std]
#![no_main]

use libc::{arg, println};

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        println!("usage: umount <mountpoint>");
        return 1;
    }
    let target = unsafe { arg(argv, 1) };
    if target == "/" {
        println!("umount: cannot unmount root filesystem");
        return 1;
    }
    println!("umount: {}: target is busy or system-managed", target);
    1
}
