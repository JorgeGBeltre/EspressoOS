#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read, spawn, wait, write, O_APPEND, O_CREATE, O_RDONLY, O_WRONLY};

const CRONTAB_PATH: &str = "/etc/crontab";

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let cmd = if argc > 1 {
        unsafe { arg(argv, 1) }
    } else {
        "list"
    };

    match cmd {
        "list" | "-l" => list_crontab(),
        "add" | "-a" => {
            if argc < 4 {
                println!("usage: cron add <interval_sec> <command...>");
                return 1;
            }
            let interval = unsafe { arg(argv, 2) };
            let command = unsafe { arg(argv, 3) };
            add_cron_job(interval, command)
        }
        "run" => run_crontab_jobs(),
        "help" | "-h" | "--help" => {
            println!("Usage: cron [list | add <sec> <cmd> | run | help]");
            println!("Periodic task scheduler and /etc/crontab manager.");
            0
        }
        _ => {
            println!("cron: unknown command '{}'. Try 'cron help'", cmd);
            1
        }
    }
}

fn list_crontab() -> i32 {
    let fd = open(CRONTAB_PATH, O_RDONLY);
    if fd < 0 {
        println!("cron: cannot open {}", CRONTAB_PATH);
        return 1;
    }

    println!("# Active EspressoOS Scheduled Tasks ({})", CRONTAB_PATH);
    let mut buf = [0u8; 128];
    loop {
        let n = read(fd as i32, &mut buf);
        if n <= 0 {
            break;
        }
        for &b in &buf[..n as usize] {
            print!("{}", b as char);
        }
    }
    close(fd as i32);
    0
}

fn add_cron_job(interval: &str, command: &str) -> i32 {
    let fd = open(CRONTAB_PATH, O_WRONLY | O_APPEND | O_CREATE);
    if fd < 0 {
        println!("cron: cannot open {} for appending", CRONTAB_PATH);
        return 1;
    }

    let _ = write(fd as i32, interval.as_bytes());
    let _ = write(fd as i32, b" ");
    let _ = write(fd as i32, command.as_bytes());
    let _ = write(fd as i32, b"\n");
    close(fd as i32);

    println!("cron: added scheduled task every {}s: {}", interval, command);
    0
}

fn run_crontab_jobs() -> i32 {
    let fd = open(CRONTAB_PATH, O_RDONLY);
    if fd < 0 {
        println!("cron: no crontab file found at {}", CRONTAB_PATH);
        return 1;
    }

    let mut buf = [0u8; 1024];
    let n = read(fd as i32, &mut buf);
    close(fd as i32);

    if n <= 0 {
        println!("cron: crontab is empty");
        return 0;
    }

    let text = match core::str::from_utf8(&buf[..n as usize]) {
        Ok(s) => s,
        Err(_) => {
            println!("cron: invalid crontab encoding");
            return 1;
        }
    };

    let mut executed = 0;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Format: <interval> <bin_path> [args]
        let mut parts = trimmed.split_whitespace();
        let interval_str = match parts.next() {
            Some(i) => i,
            None => continue,
        };
        let bin = match parts.next() {
            Some(b) => b,
            None => continue,
        };

        let mut bin_buf = [0u8; 64];
        let blen = bin.len().min(63);
        bin_buf[..blen].copy_from_slice(&bin.as_bytes()[..blen]);
        bin_buf[blen] = 0;

        println!("[cron] Triggering task (every {}s): {}", interval_str, bin);
        let argv = [bin_buf.as_ptr(), core::ptr::null()];
        let pid = spawn(bin, argv.as_ptr());
        if pid >= 0 {
            let mut status = 0;
            let _ = wait(&mut status);
            executed += 1;
        } else {
            println!("[cron] Failed to spawn {}: code {}", bin, pid);
        }
    }

    println!("cron: completed execution cycle ({} jobs triggered)", executed);
    0
}
