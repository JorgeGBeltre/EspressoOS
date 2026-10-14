#![no_std]
#![no_main]

use libc::{arg, close, open, println, read, O_RDONLY};

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut all = false;

    let mut i = 1;
    while i < argc {
        let a = unsafe { arg(argv, i) };
        if a == "--help" || a == "-h" {
            println!("Usage: nproc [OPTION]...");
            println!("Print the number of processing units available to the current process");
            println!("  --all       print the number of installed processors");
            return 0;
        } else if a == "--all" {
            all = true;
        }
        i += 1;
    }

    if all {
        println!("2");
        return 0;
    }

    // Check if SMP is active via /sys/smp
    let fd = open("/sys/smp", O_RDONLY);
    if fd >= 0 {
        let mut buf = [0u8; 64];
        let n = read(fd as i32, &mut buf);
        let _ = close(fd as i32);
        if n > 0 {
            if let Ok(s) = core::str::from_utf8(&buf[..n as usize]) {
                if s.contains("smp: enabled") {
                    println!("2");
                    return 0;
                }
            }
        }
    }

    println!("1");
    0
}
