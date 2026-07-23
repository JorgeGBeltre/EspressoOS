#![no_std]
#![no_main]

use libc::{arg, kill, println};

fn parse_i32(s: &str) -> Option<i32> {
    if s.is_empty() {
        return None;
    }
    let mut val = 0i32;
    for &b in s.as_bytes() {
        if b >= b'0' && b <= b'9' {
            val = val * 10 + (b - b'0') as i32;
        } else {
            return None;
        }
    }
    Some(val)
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        println!("Usage: kill [-SIG] <pid>");
        return 1;
    }

    let mut sig = 15; // Default SIGTERM
    let mut pid_idx = 1;

    let arg1 = unsafe { arg(argv, 1) };
    if arg1.starts_with('-') && arg1.len() > 1 {
        let sig_str = &arg1[1..];
        sig = match sig_str {
            "9" | "KILL" | "SIGKILL" => 9,
            "2" | "INT" | "SIGINT" => 2,
            "15" | "TERM" | "SIGTERM" => 15,
            _ => match parse_i32(sig_str) {
                Some(num) => num,
                None => {
                    println!("kill: invalid signal '{}'", sig_str);
                    return 1;
                }
            },
        };
        pid_idx = 2;
    }

    if pid_idx >= argc {
        println!("Usage: kill [-SIG] <pid>");
        return 1;
    }

    let pid_str = unsafe { arg(argv, pid_idx) };
    let pid = match parse_i32(pid_str) {
        Some(p) if p > 0 => p as u32,
        _ => {
            println!("kill: invalid pid '{}'", pid_str);
            return 1;
        }
    };

    let ret = kill(pid, sig);
    if ret < 0 {
        println!("kill: ({}) failed (errno {})", pid, ret);
        return 1;
    }

    0
}
