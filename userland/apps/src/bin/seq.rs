#![no_std]
#![no_main]

use libc::{arg, print, println};

fn parse_i64(s: &str) -> Option<i64> {
    let s = s.trim_ascii();
    if s.is_empty() {
        return None;
    }
    let bytes = s.as_bytes();
    let (neg, start) = if bytes[0] == b'-' {
        (true, 1)
    } else if bytes[0] == b'+' {
        (false, 1)
    } else {
        (false, 0)
    };
    if start >= bytes.len() {
        return None;
    }
    let mut val: i64 = 0;
    for &b in &bytes[start..] {
        if !b.is_ascii_digit() {
            return None;
        }
        val = val.checked_mul(10)?.checked_add((b - b'0') as i64)?;
    }
    if neg {
        Some(-val)
    } else {
        Some(val)
    }
}

fn digit_count(mut n: i64) -> usize {
    if n == 0 {
        return 1;
    }
    let mut count = 0;
    if n < 0 {
        count += 1;
        n = -n;
    }
    while n > 0 {
        count += 1;
        n /= 10;
    }
    count
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut sep = "\n";
    let mut equal_width = false;
    let mut positional: [&str; 4] = ["", "", "", ""];
    let mut pos_count = 0;

    let mut i = 1;
    while i < argc {
        let a = unsafe { arg(argv, i) };
        if a == "--help" || a == "-h" {
            println!("Usage: seq [-s SEP] [-w] [FIRST [INCR]] LAST");
            println!("Print numbers from FIRST to LAST, in steps of INCR.");
            return 0;
        } else if a == "-w" {
            equal_width = true;
        } else if a == "-s" {
            i += 1;
            if i < argc {
                sep = unsafe { arg(argv, i) };
            } else {
                println!("seq: option requires an argument -- 's'");
                return 1;
            }
        } else if a.starts_with("-s") && a.len() > 2 {
            sep = &a[2..];
        } else {
            if pos_count < 4 {
                positional[pos_count] = a;
                pos_count += 1;
            }
        }
        i += 1;
    }

    let (first, step, last) = match pos_count {
        1 => {
            let last = match parse_i64(positional[0]) {
                Some(v) => v,
                None => {
                    println!("seq: invalid argument: '{}'", positional[0]);
                    return 1;
                }
            };
            (1i64, 1i64, last)
        }
        2 => {
            let first = match parse_i64(positional[0]) {
                Some(v) => v,
                None => {
                    println!("seq: invalid argument: '{}'", positional[0]);
                    return 1;
                }
            };
            let last = match parse_i64(positional[1]) {
                Some(v) => v,
                None => {
                    println!("seq: invalid argument: '{}'", positional[1]);
                    return 1;
                }
            };
            (first, 1i64, last)
        }
        3 => {
            let first = match parse_i64(positional[0]) {
                Some(v) => v,
                None => {
                    println!("seq: invalid argument: '{}'", positional[0]);
                    return 1;
                }
            };
            let step = match parse_i64(positional[1]) {
                Some(v) => v,
                None => {
                    println!("seq: invalid argument: '{}'", positional[1]);
                    return 1;
                }
            };
            let last = match parse_i64(positional[2]) {
                Some(v) => v,
                None => {
                    println!("seq: invalid argument: '{}'", positional[2]);
                    return 1;
                }
            };
            if step == 0 {
                println!("seq: zero increment");
                return 1;
            }
            (first, step, last)
        }
        _ => {
            println!("Usage: seq [-s SEP] [-w] [FIRST [INCR]] LAST");
            return 1;
        }
    };

    let max_width = if equal_width {
        let w1 = digit_count(first);
        let w2 = digit_count(last);
        core::cmp::max(w1, w2)
    } else {
        0
    };

    let mut current = first;
    let mut printed = false;

    let should_continue = |cur: i64| -> bool {
        if step > 0 {
            cur <= last
        } else {
            cur >= last
        }
    };

    while should_continue(current) {
        if printed {
            print!("{}", sep);
        }
        if equal_width {
            let cur_w = digit_count(current);
            if current < 0 {
                print!("-");
                for _ in cur_w..max_width {
                    print!("0");
                }
                print!("{}", -current);
            } else {
                for _ in cur_w..max_width {
                    print!("0");
                }
                print!("{}", current);
            }
        } else {
            print!("{}", current);
        }
        printed = true;

        match current.checked_add(step) {
            Some(next) => current = next,
            None => break,
        }
    }

    if printed {
        println!();
    }
    0
}
