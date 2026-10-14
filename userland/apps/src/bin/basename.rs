#![no_std]
#![no_main]

use libc::{arg, println};

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        println!("Usage: basename <path> [suffix]");
        return 1;
    }

    let raw_path = unsafe { arg(argv, 1) };
    if raw_path == "-h" || raw_path == "--help" {
        println!("Usage: basename <path> [suffix]");
        println!("Strip directory and suffix from filenames");
        return 0;
    }

    // Strip trailing slashes
    let mut path = raw_path;
    while path.len() > 1 && path.ends_with('/') {
        path = &path[..path.len() - 1];
    }

    // Find last '/'
    let base = match path.rfind('/') {
        Some(idx) => &path[idx + 1..],
        None => path,
    };

    let result = if base.is_empty() {
        "/"
    } else if argc >= 3 {
        let suffix = unsafe { arg(argv, 2) };
        if !suffix.is_empty() && base.ends_with(suffix) && base.len() > suffix.len() {
            &base[..base.len() - suffix.len()]
        } else {
            base
        }
    } else {
        base
    };

    println!("{}", result);
    0
}
