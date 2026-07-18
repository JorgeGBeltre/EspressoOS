#![no_std]
#![no_main]

use libc::{arg, mkdir, println};

/// mkdir(1). Crea uno o más directorios. Paridad con el builtin `mkdir DIR...`.
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc <= 1 {
        println!("usage: mkdir DIR...");
        return 1;
    }
    let mut status = 0;
    for i in 1..argc {
        let path = unsafe { arg(argv, i) };
        if mkdir(path) < 0 {
            println!("mkdir: {}: cannot create", path);
            status = 1;
        }
    }
    status
}
