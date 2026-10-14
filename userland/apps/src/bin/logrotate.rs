#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read, seek, write, O_CREATE, O_RDONLY, O_TRUNC, O_WRONLY};

const DEFAULT_LOG: &str = "/var/log/syslog";
const DEFAULT_MAX_SIZE: isize = 4096;

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut force = false;
    let mut verbose = false;
    let mut target_log = DEFAULT_LOG;

    for i in 1..argc {
        let a = unsafe { arg(argv, i) };
        if a == "-f" || a == "--force" {
            force = true;
        } else if a == "-v" || a == "--verbose" {
            verbose = true;
        } else if a == "-h" || a == "--help" {
            println!("usage: logrotate [-f] [-v] [logfile]");
            println!("       Rotates system log file when size threshold is reached");
            return 0;
        } else if !a.starts_with('-') {
            target_log = a;
        }
    }

    let fd = open(target_log, O_RDONLY);
    if fd < 0 {
        if verbose {
            println!("logrotate: log file {} does not exist, nothing to rotate", target_log);
        }
        return 0;
    }

    let size = seek(fd as i32, 0, 2);
    let _ = seek(fd as i32, 0, 0);

    if size < DEFAULT_MAX_SIZE && !force {
        if verbose {
            println!("logrotate: {} is {} bytes (< {}), skipping rotation", target_log, size, DEFAULT_MAX_SIZE);
        }
        close(fd as i32);
        return 0;
    }

    println!("logrotate: rotating {} ({} bytes)", target_log, size);

    // Read existing log
    let mut buf = [0u8; 4096];
    let n = read(fd as i32, &mut buf);
    close(fd as i32);

    // Prepare backup target path, e.g. /var/log/syslog.1
    let mut backup_path = [0u8; 64];
    let t_bytes = target_log.as_bytes();
    let mut p_len = 0;
    for &b in t_bytes {
        if p_len < 60 {
            backup_path[p_len] = b;
            p_len += 1;
        }
    }
    backup_path[p_len] = b'.';
    backup_path[p_len + 1] = b'1';
    p_len += 2;

    let backup_str = match core::str::from_utf8(&backup_path[..p_len]) {
        Ok(s) => s,
        Err(_) => "/tmp/syslog.1",
    };

    // Write to backup file
    let b_fd = open(backup_str, O_WRONLY | O_CREATE | O_TRUNC);
    if b_fd >= 0 {
        if n > 0 {
            let _ = write(b_fd as i32, &buf[..n as usize]);
        }
        close(b_fd as i32);
        if verbose {
            println!("logrotate: archived to {}", backup_str);
        }
    }

    // Truncate original log
    let t_fd = open(target_log, O_WRONLY | O_TRUNC);
    if t_fd >= 0 {
        close(t_fd as i32);
        if verbose {
            println!("logrotate: truncated {}", target_log);
        }
    }

    0
}
