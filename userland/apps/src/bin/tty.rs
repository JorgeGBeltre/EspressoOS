#![no_std]
#![no_main]

use libc::{arg, println, seek};

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut silent = false;

    let mut i = 1;
    while i < argc {
        let a = unsafe { arg(argv, i) };
        if a == "-s" || a == "--silent" || a == "--quiet" {
            silent = true;
        } else if a == "--help" || a == "-h" {
            println!("Usage: tty [OPTION]...");
            println!("Print the file name of the terminal connected to standard input");
            println!("  -s, --silent, --quiet  print nothing, only return an exit status");
            return 0;
        }
        i += 1;
    }

    // On EspressoOS, character devices and streams fail seek with -ESPIPE (< 0).
    // Regular files return the current seek position (>= 0).
    let is_tty = seek(0, 0, 1) < 0;

    if is_tty {
        if !silent {
            println!("/dev/console");
        }
        0
    } else {
        if !silent {
            println!("not a tty");
        }
        1
    }
}
