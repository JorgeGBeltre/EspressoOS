#![no_std]
#![no_main]

use libc::{arg, println};

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut word = "y";
    let mut limit: usize = 100; // bounded safety limit on microcontroller console

    let mut i = 1;
    while i < argc {
        let a = unsafe { arg(argv, i) };
        if a == "-n" {
            i += 1;
            if i < argc {
                if let Ok(n) = unsafe { arg(argv, i) }.parse::<usize>() {
                    limit = n;
                }
            }
        } else if a == "-h" || a == "--help" {
            println!("usage: yes [-n count] [string]");
            println!("       Repeatedly output a line with all specified STRING(s), or 'y'");
            return 0;
        } else if !a.starts_with('-') {
            word = a;
        }
        i += 1;
    }

    for _ in 0..limit {
        println!("{}", word);
    }

    0
}
