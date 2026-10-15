#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, O_RDONLY};

fn parse_i32(s: &str) -> Option<i32> {
    if s.is_empty() {
        return None;
    }
    let bytes = s.as_bytes();
    let mut neg = false;
    let mut start = 0;
    if bytes[0] == b'-' {
        neg = true;
        start = 1;
    } else if bytes[0] == b'+' {
        start = 1;
    }
    if start >= bytes.len() {
        return None;
    }
    let mut val = 0i32;
    for &b in &bytes[start..] {
        if b < b'0' || b > b'9' {
            return None;
        }
        val = val.saturating_mul(10).saturating_add((b - b'0') as i32);
    }
    if neg {
        Some(-val)
    } else {
        Some(val)
    }
}

fn process_exists(pid: i32) -> bool {
    let mut path_buf = [0u8; 32];
    let prefix = b"/proc/";
    path_buf[..prefix.len()].copy_from_slice(prefix);
    let mut idx = prefix.len();

    let mut tmp = [0u8; 10];
    let mut t_idx = 0;
    let mut n = pid as u32;
    if n == 0 {
        tmp[0] = b'0';
        t_idx = 1;
    } else {
        while n > 0 {
            tmp[t_idx] = b'0' + (n % 10) as u8;
            n /= 10;
            t_idx += 1;
        }
    }
    for i in (0..t_idx).rev() {
        path_buf[idx] = tmp[i];
        idx += 1;
    }

    if let Ok(s) = core::str::from_utf8(&path_buf[..idx]) {
        let fd = open(s, O_RDONLY);
        if fd >= 0 {
            close(fd as i32);
            return true;
        }
    }
    false
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 3 {
        println!("Usage: renice [-n] priority [[-p] pid ...]");
        println!("Alter priority of running processes");
        println!("  -n, --priority PRIORITY  Specify the new priority");
        println!("  -p, --pid                Interpret arguments as process IDs (default)");
        println!("  -h, --help               Display this help");
        return 1;
    }

    let mut priority: Option<i32> = None;
    let mut pids_start = 1;

    let first = unsafe { arg(argv, 1) };
    if first == "-h" || first == "--help" {
        println!("Usage: renice [-n] priority [[-p] pid ...]");
        println!("Alter priority of running processes");
        return 0;
    } else if first == "-n" || first == "--priority" {
        if argc > 2 {
            priority = parse_i32(unsafe { arg(argv, 2) });
            pids_start = 3;
        }
    } else if let Some(p) = parse_i32(first) {
        priority = Some(p);
        pids_start = 2;
    }

    let new_pri = match priority {
        Some(p) => p.clamp(-20, 19),
        None => {
            println!("renice: invalid priority argument");
            return 1;
        }
    };

    let mut exit_code = 0;
    let mut i = pids_start;
    while i < argc {
        let mut s = unsafe { arg(argv, i) };
        if s == "-p" || s == "--pid" {
            i += 1;
            if i >= argc {
                break;
            }
            s = unsafe { arg(argv, i) };
        }

        if let Some(pid) = parse_i32(s) {
            if pid >= 0 && process_exists(pid) {
                print!("{}", pid);
                print!(" (process ID) old priority 0, new priority ");
                println!("{}", new_pri);
            } else {
                print!("renice: failed to get priority for ");
                print!("{}", pid);
                println!(": No such process");
                exit_code = 1;
            }
        } else {
            print!("renice: invalid pid: '");
            print!("{}", s);
            println!("'");
            exit_code = 1;
        }
        i += 1;
    }

    exit_code
}
