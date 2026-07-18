#![no_std]
#![no_main]

use libc::{arg, println, unlink};

/// rm(1). Borra uno o más ficheros. Paridad con el builtin `rm FILE...`.
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc <= 1 {
        println!("usage: rm FILE...");
        return 1;
    }
    let mut status = 0;
    for i in 1..argc {
        let path = unsafe { arg(argv, i) };
        if unlink(path) < 0 {
            println!("rm: {}: cannot remove", path);
            status = 1;
        }
    }
    status
}
