#![no_std]
#![no_main]

use libc::{arg, println, unlink};

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc != 2 {
        println!("Usage: unlink FILE");
        println!("Call the unlink system call to remove the specified FILE");
        return 1;
    }

    let file = unsafe { arg(argv, 1) };
    if file == "-h" || file == "--help" {
        println!("Usage: unlink FILE");
        println!("Call the unlink system call to remove the specified FILE");
        return 0;
    }

    if unlink(file) < 0 {
        println!("unlink: cannot unlink '{}': No such file or directory", file);
        return 1;
    }

    0
}
