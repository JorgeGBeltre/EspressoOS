#![no_std]
#![no_main]

use libc::{arg, print, println, spawn, waitpid};

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

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc == 1 {
        // Without arguments, print default/current nice value
        println!("0");
        return 0;
    }

    let mut cmd_idx = 1;
    let mut _increment = 10i32;

    let first = unsafe { arg(argv, 1) };
    if first == "-h" || first == "--help" {
        println!("Usage: nice [-n adjustment] [COMMAND [ARG...]]");
        println!("Run a program with modified scheduling priority (niceness)");
        println!("  -n ADJUST  Add integer ADJUST to the priority (default: 10)");
        return 0;
    }

    if first == "-n" {
        if argc > 2 {
            if let Some(adj) = parse_i32(unsafe { arg(argv, 2) }) {
                _increment = adj;
                cmd_idx = 3;
            } else {
                cmd_idx = 2;
            }
        } else {
            println!("nice: option requires an argument -- 'n'");
            return 1;
        }
    } else if first.starts_with("-n") && first.len() > 2 {
        if let Some(adj) = parse_i32(&first[2..]) {
            _increment = adj;
            cmd_idx = 2;
        }
    } else if first.starts_with('-') && first.len() > 1 && first.as_bytes()[1] >= b'0' && first.as_bytes()[1] <= b'9' {
        if let Some(adj) = parse_i32(&first[1..]) {
            _increment = adj;
            cmd_idx = 2;
        }
    }

    if cmd_idx >= argc {
        println!("0");
        return 0;
    }

    let cmd = unsafe { arg(argv, cmd_idx) };
    let mut path_buf = [0u8; 64];
    let path = if cmd.starts_with('/') {
        cmd
    } else {
        let prefix = b"/bin/";
        let cmd_bytes = cmd.as_bytes();
        let total = prefix.len() + cmd_bytes.len();
        if total < path_buf.len() {
            path_buf[..prefix.len()].copy_from_slice(prefix);
            path_buf[prefix.len()..total].copy_from_slice(cmd_bytes);
            core::str::from_utf8(&path_buf[..total]).unwrap_or(cmd)
        } else {
            cmd
        }
    };

    let child_argv = unsafe { argv.offset(cmd_idx as isize) };
    let mut pid = spawn(path, child_argv);
    if pid < 0 {
        pid = spawn(cmd, child_argv);
    }
    if pid < 0 {
        print!("nice: cannot run '");
        print!("{}", cmd);
        println!("': No such file or directory");
        return 127;
    }

    let mut status = 0i32;
    let ret = waitpid(pid as i32, &mut status, 0);
    if ret < 0 {
        return 1;
    }
    status
}
