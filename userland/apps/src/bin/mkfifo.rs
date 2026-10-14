#![no_std]
#![no_main]

use libc::{arg, mkfifo, println};

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc <= 1 {
        println!("usage: mkfifo NAME...");
        return 1;
    }

    let first = unsafe { arg(argv, 1) };
    if first == "-h" || first == "--help" {
        println!("usage: mkfifo NAME...");
        println!("Create named pipes (FIFOs) with the given NAMEs");
        return 0;
    }

    let mut status = 0;
    for i in 1..argc {
        let path = unsafe { arg(argv, i) };
        if mkfifo(path) < 0 {
            println!("mkfifo: cannot create fifo '{}'", path);
            status = 1;
        }
    }
    status
}
