#![no_std]
#![no_main]

use libc::{arg, println};

const DEFAULT_ENV: &[(&str, &str)] = &[
    ("PATH", "/bin:/usr/bin"),
    ("USER", "youareme"),
    ("HOME", "/home/youareme"),
    ("SHELL", "/bin/sh"),
    ("TERM", "xterm-256color"),
    ("OS", "EspressoOS"),
    ("ARCH", "xtensa-lx7"),
    ("VERSION", "0.2.0-dev"),
];

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc <= 1 {
        for &(k, v) in DEFAULT_ENV {
            println!("{}={}", k, v);
        }
        return 0;
    }

    let mut found_all = true;
    for i in 1..argc {
        let var_name = unsafe { arg(argv, i) };
        let mut match_found = false;
        for &(k, v) in DEFAULT_ENV {
            if k == var_name {
                println!("{}", v);
                match_found = true;
                break;
            }
        }
        if !match_found {
            found_all = false;
        }
    }

    if found_all {
        0
    } else {
        1
    }
}
