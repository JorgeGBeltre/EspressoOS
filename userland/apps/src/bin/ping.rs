#![no_std]
#![no_main]

use libc::{arg, println};

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let target = if argc > 1 {
        unsafe { arg(argv, 1) }
    } else {
        "<target>"
    };

    println!("ping: ICMP (RAW) sockets are not supported by the smoltcp/kernel network stack in EspressoOS.");
    println!("Notice: Standard ICMP ping cannot be performed.");
    println!("To test TCP connectivity, please use:");
    println!("  tcping {} 80", target);
    1
}
