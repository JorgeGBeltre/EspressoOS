#![no_std]
#![no_main]

use libc::{arg, print, println, sleep_ms, spawn, wait};

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        println!("usage: watch [-n interval] [-c count] <command>");
        return 1;
    }

    let mut interval_sec = 2usize;
    let mut max_count = 10usize; // default safe bounded iteration
    let mut cmd_idx = 1;

    let mut i = 1;
    while i < argc {
        let a = unsafe { arg(argv, i) };
        if a == "-n" && i + 1 < argc {
            i += 1;
            let val = unsafe { arg(argv, i) };
            if let Ok(n) = val.parse::<usize>() {
                interval_sec = n.max(1);
            }
        } else if a == "-c" && i + 1 < argc {
            i += 1;
            let val = unsafe { arg(argv, i) };
            if let Ok(n) = val.parse::<usize>() {
                max_count = n.max(1);
            }
        } else if !a.starts_with('-') {
            cmd_idx = i;
            break;
        }
        i += 1;
    }

    if cmd_idx >= argc {
        println!("watch: missing command to execute");
        return 1;
    }

    let cmd_name = unsafe { arg(argv, cmd_idx) };
    let mut bin_path_buf = [0u8; 64];

    let full_path = if cmd_name.starts_with('/') {
        cmd_name
    } else {
        // Prepend /bin/
        let mut idx = 0;
        for &b in b"/bin/" {
            bin_path_buf[idx] = b;
            idx += 1;
        }
        for &b in cmd_name.as_bytes() {
            if idx < 63 {
                bin_path_buf[idx] = b;
                idx += 1;
            }
        }
        bin_path_buf[idx] = 0;
        match core::str::from_utf8(&bin_path_buf[..idx]) {
            Ok(s) => s,
            Err(_) => cmd_name,
        }
    };

    let mut path_c = [0u8; 64];
    let plen = full_path.len().min(63);
    path_c[..plen].copy_from_slice(&full_path.as_bytes()[..plen]);
    path_c[plen] = 0;

    for iteration in 1..=max_count {
        // Clear terminal screen and reset cursor position (ANSI escape code)
        print!("\x1b[2J\x1b[H");
        println!("Every {}s: {} (cycle {}/{})", interval_sec, full_path, iteration, max_count);
        println!("--------------------------------------------------");

        let argv_child = [path_c.as_ptr(), core::ptr::null()];
        let pid = spawn(full_path, argv_child.as_ptr());
        if pid >= 0 {
            let mut status = 0;
            let _ = wait(&mut status);
        } else {
            println!("watch: failed to execute {}: error {}", full_path, pid);
            return 1;
        }

        if iteration < max_count {
            sleep_ms(interval_sec * 1000);
        }
    }

    0
}
