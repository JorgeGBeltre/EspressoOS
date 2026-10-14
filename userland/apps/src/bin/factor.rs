#![no_std]
#![no_main]

use libc::{arg, print, println, read};

fn factorize(mut n: u64) {
    print!("{}:", n);
    if n <= 1 {
        println!();
        return;
    }

    // Factor out 2s
    while n % 2 == 0 {
        print!(" 2");
        n /= 2;
    }

    // Factor out 3s
    while n % 3 == 0 {
        print!(" 3");
        n /= 3;
    }

    // Factor out 6k +/- 1 up to sqrt(n)
    let mut d = 5u64;
    while d * d <= n {
        while n % d == 0 {
            print!(" {}", d);
            n /= d;
        }
        let d2 = d + 2;
        while n % d2 == 0 {
            print!(" {}", d2);
            n /= d2;
        }
        d += 6;
    }

    if n > 1 {
        print!(" {}", n);
    }
    println!();
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc > 1 {
        for i in 1..argc {
            let s = unsafe { arg(argv, i) };
            if s == "-h" || s == "--help" {
                println!("usage: factor [number...]");
                println!("       Print prime factors of each number");
                return 0;
            }
            if let Ok(num) = s.parse::<u64>() {
                factorize(num);
            } else {
                println!("factor: '{}' is not a valid number", s);
            }
        }
        return 0;
    }

    // Read from stdin
    let mut buf = [0u8; 128];
    let n = read(0, &mut buf);
    if n > 0 {
        if let Ok(s) = core::str::from_utf8(&buf[..n as usize]) {
            for word in s.split_whitespace() {
                if let Ok(num) = word.parse::<u64>() {
                    factorize(num);
                }
            }
        }
    }

    0
}
