#![no_std]
#![no_main]

use libc::{arg, println, umount};

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        println!("Usage: umount <mountpoint>");
        return 1;
    }
    let target = unsafe { arg(argv, 1) };
    if target == "-h" || target == "--help" {
        println!("Usage: umount <mountpoint>");
        return 0;
    }
    let rc = umount(target);
    if rc < 0 {
        let msg = match rc {
            -1 => "Operation not permitted",
            -2 => "not mounted",
            -16 => "target is busy or protected system mount",
            _ => "Invalid mountpoint",
        };
        println!("umount: {}: {}", target, msg);
        return 1;
    }
    0
}
