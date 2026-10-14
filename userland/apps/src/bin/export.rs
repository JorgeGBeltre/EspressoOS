#![no_std]
#![no_main]

use libc::{arg, print, println};

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc == 1 {
        println!("export PATH=/bin:/sbin");
        println!("export OS=EspressoOS");
        return 0;
    }

    let first = unsafe { arg(argv, 1) };
    if first == "-h" || first == "--help" {
        println!("Usage: export [NAME[=VALUE]...]");
        println!("Set export attribute for shell variables");
        return 0;
    }

    for i in 1..argc {
        let var = unsafe { arg(argv, i) };
        print!("export ");
        println!("{}", var);
    }
    0
}
