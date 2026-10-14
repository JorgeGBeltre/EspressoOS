#![no_std]
#![no_main]

use libc::{arg, println};

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        println!("Usage: dirname <path>");
        return 1;
    }

    let raw_path = unsafe { arg(argv, 1) };
    if raw_path == "-h" || raw_path == "--help" {
        println!("Usage: dirname <path>");
        println!("Strip last component from file name");
        return 0;
    }

    // Strip trailing slashes
    let mut path = raw_path;
    while path.len() > 1 && path.ends_with('/') {
        path = &path[..path.len() - 1];
    }

    if path == "/" {
        println!("/");
        return 0;
    }

    let result = match path.rfind('/') {
        Some(0) => "/",
        Some(idx) => &path[..idx],
        None => ".",
    };

    println!("{}", result);
    0
}
