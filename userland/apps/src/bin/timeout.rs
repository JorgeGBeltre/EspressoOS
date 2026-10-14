#![no_std]
#![no_main]

use libc::{arg, kill, println, sleep_ms, spawn, uptime_ms, waitpid, WNOHANG};

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 3 {
        println!("usage: timeout <duration_sec> <command> [args...]");
        return 1;
    }

    let dur_str = unsafe { arg(argv, 1) };
    let duration_sec = match dur_str.parse::<usize>() {
        Ok(n) if n > 0 => n,
        _ => {
            println!("timeout: invalid duration '{}'", dur_str);
            return 1;
        }
    };

    let cmd_name = unsafe { arg(argv, 2) };
    let mut bin_path_buf = [0u8; 64];

    let full_path = if cmd_name.starts_with('/') {
        cmd_name
    } else {
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

    // Forward remaining arguments
    let mut child_argv: [*const u8; 16] = [core::ptr::null(); 16];
    let mut argv_idx = 0;
    for i in 2..argc {
        if argv_idx < 15 {
            child_argv[argv_idx] = unsafe { *argv.offset(i as isize) };
            argv_idx += 1;
        }
    }
    child_argv[argv_idx] = core::ptr::null();

    let pid = spawn(full_path, child_argv.as_ptr());
    if pid < 0 {
        println!("timeout: failed to execute {}: code {}", full_path, pid);
        return 1;
    }

    let start_ms = uptime_ms();
    let limit_ms = duration_sec.saturating_mul(1000);
    let mut status = 0;

    loop {
        let ret = waitpid(pid as i32, &mut status, WNOHANG);
        if ret > 0 {
            // Child exited normally
            return status;
        } else if ret < 0 {
            // Error or process vanished
            return 0;
        }

        let elapsed = uptime_ms().wrapping_sub(start_ms);
        if elapsed >= limit_ms {
            // Timeout reached! Send SIGKILL (9)
            println!("timeout: command '{}' timed out after {}s", cmd_name, duration_sec);
            let _ = kill(pid as u32, 9);
            // Non-blocking reap
            let _ = waitpid(pid as i32, &mut status, WNOHANG);
            return 124; // Standard POSIX timeout exit code
        }

        sleep_ms(50);
    }
}
