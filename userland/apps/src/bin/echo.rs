#![no_std]
#![no_main]

use libc::{arg, print, println};

/// echo(1). argv[0] is the program name, so the words start at 1.
///
/// `-n` suppresses the trailing newline, matching the shell built-in of the same
/// name -- that one answers a bare `echo`; this is what `/bin/echo` runs.
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut first = 1;
    let mut newline = true;
    if argc > 1 && unsafe { arg(argv, 1) } == "-n" {
        newline = false;
        first = 2;
    }

    for i in first..argc {
        if i > first {
            print!(" ");
        }
        print!("{}", unsafe { arg(argv, i) });
    }
    if newline {
        println!();
    }
    0
}
