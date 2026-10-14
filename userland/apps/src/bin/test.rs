#![no_std]
#![no_main]

use libc::{arg, close, open, readdir, O_RDONLY};

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc <= 1 {
        // test with no arguments returns false (1)
        return 1;
    }

    let mut start_idx = 1;
    let mut end_idx = argc;

    // Handle invocation as `[ ... ]`
    let first = unsafe { arg(argv, 0) };
    if first == "[" || first.ends_with("/[") {
        let last = unsafe { arg(argv, argc - 1) };
        if last == "]" {
            end_idx -= 1;
        }
    }

    let count = end_idx - start_idx;
    if count == 0 {
        return 1;
    }

    if count == 1 {
        let s = unsafe { arg(argv, start_idx) };
        return if s.is_empty() { 1 } else { 0 };
    }

    if count == 2 {
        let op = unsafe { arg(argv, start_idx) };
        let val = unsafe { arg(argv, start_idx + 1) };
        match op {
            "-z" => return if val.is_empty() { 0 } else { 1 },
            "-n" => return if !val.is_empty() { 0 } else { 1 },
            "-f" => {
                let fd = open(val, O_RDONLY);
                if fd >= 0 {
                    close(fd as i32);
                    return 0;
                }
                return 1;
            }
            "-d" => {
                let mut buf = [0u8; 32];
                let n = readdir(val, &mut buf);
                return if n >= 0 { 0 } else { 1 };
            }
            "-e" => {
                let fd = open(val, O_RDONLY);
                if fd >= 0 {
                    close(fd as i32);
                    return 0;
                }
                let mut buf = [0u8; 32];
                let n = readdir(val, &mut buf);
                return if n >= 0 { 0 } else { 1 };
            }
            "!" => {
                return if val.is_empty() { 0 } else { 1 };
            }
            _ => return 2,
        }
    }

    if count == 3 {
        let a = unsafe { arg(argv, start_idx) };
        let op = unsafe { arg(argv, start_idx + 1) };
        let b = unsafe { arg(argv, start_idx + 2) };

        match op {
            "=" | "==" => return if a == b { 0 } else { 1 },
            "!=" => return if a != b { 0 } else { 1 },
            "-eq" | "-ne" | "-lt" | "-le" | "-gt" | "-ge" => {
                let n1 = match a.parse::<i64>() {
                    Ok(v) => v,
                    Err(_) => return 2,
                };
                let n2 = match b.parse::<i64>() {
                    Ok(v) => v,
                    Err(_) => return 2,
                };
                let res = match op {
                    "-eq" => n1 == n2,
                    "-ne" => n1 != n2,
                    "-lt" => n1 < n2,
                    "-le" => n1 <= n2,
                    "-gt" => n1 > n2,
                    "-ge" => n1 >= n2,
                    _ => false,
                };
                return if res { 0 } else { 1 };
            }
            _ => return 2,
        }
    }

    2
}
