#![no_std]
#![no_main]

use libc::{arg, kill, kill_pgid, println};

fn parse_i32(s: &str) -> Option<i32> {
    if s.is_empty() {
        return None;
    }
    let bytes = s.as_bytes();
    let (neg, start) = if bytes[0] == b'-' {
        if bytes.len() == 1 {
            return None;
        }
        (true, 1)
    } else {
        (false, 0)
    };
    let mut val = 0i32;
    for &b in &bytes[start..] {
        if b >= b'0' && b <= b'9' {
            val = val.saturating_mul(10).saturating_add((b - b'0') as i32);
        } else {
            return None;
        }
    }
    Some(if neg { -val } else { val })
}

fn parse_sig(sig_str: &str) -> Option<i32> {
    match sig_str {
        "1" | "HUP" | "SIGHUP" => Some(1),
        "2" | "INT" | "SIGINT" => Some(2),
        "3" | "QUIT" | "SIGQUIT" => Some(3),
        "6" | "ABRT" | "SIGABRT" => Some(6),
        "9" | "KILL" | "SIGKILL" => Some(9),
        "15" | "TERM" | "SIGTERM" => Some(15),
        "18" | "CONT" | "SIGCONT" => Some(18),
        "19" | "STOP" | "SIGSTOP" => Some(19),
        "20" | "TSTP" | "SIGTSTP" => Some(20),
        _ => parse_i32(sig_str).filter(|&n| n > 0),
    }
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        println!("Usage: kill [-SIG] <pid|-pgid>...");
        return 1;
    }

    let arg1 = unsafe { arg(argv, 1) };
    if arg1 == "-l" {
        println!("1) SIGHUP  2) SIGINT  3) SIGQUIT  6) SIGABRT  9) SIGKILL  15) SIGTERM  18) SIGCONT  19) SIGSTOP  20) SIGTSTP");
        return 0;
    }

    let mut sig = 15;
    let mut pid_idx = 1;

    if arg1 == "--" {
        pid_idx = 2;
    } else if arg1 == "-s" && argc >= 4 {
        let sname = unsafe { arg(argv, 2) };
        match parse_sig(sname) {
            Some(n) => sig = n,
            None => {
                println!("kill: invalid signal '{}'", sname);
                return 1;
            }
        }
        pid_idx = 3;
    } else if arg1.starts_with('-') && arg1.len() > 1 && argc >= 3 {
        let sig_str = &arg1[1..];
        if let Some(n) = parse_sig(sig_str) {
            sig = n;
            pid_idx = 2;
        }
    }

    if pid_idx >= argc {
        println!("Usage: kill [-SIG] <pid|-pgid>...");
        return 1;
    }

    let mut status = 0;
    for i in pid_idx..argc {
        let pid_str = unsafe { arg(argv, i) };
        let target = match parse_i32(pid_str) {
            Some(p) if p != 0 => p,
            _ => {
                println!("kill: invalid pid '{}'", pid_str);
                status = 1;
                continue;
            }
        };
        let ret = if target < 0 {
            kill_pgid((-target) as u32, sig)
        } else {
            kill(target as u32, sig)
        };
        if ret < 0 {
            println!("kill: ({}) failed (errno {})", target, ret);
            status = 1;
        }
    }
    status
}
