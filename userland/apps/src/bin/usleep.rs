#![no_std]
#![no_main]

use libc::{arg, println, sleep_ms, yield_now};

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        println!("Usage: usleep MICROSECONDS");
        println!("Pause execution for MICROSECONDS");
        return 1;
    }

    let a = unsafe { arg(argv, 1) };
    if a == "-h" || a == "--help" {
        println!("Usage: usleep MICROSECONDS");
        println!("Pause execution for MICROSECONDS");
        return 0;
    }

    let usecs = match a.parse::<usize>() {
        Ok(n) => n,
        Err(_) => {
            println!("usleep: invalid number of microseconds: {}", a);
            return 1;
        }
    };

    if usecs == 0 {
        yield_now();
        return 0;
    }

    let ms = (usecs + 999) / 1000;
    sleep_ms(ms);
    0
}
