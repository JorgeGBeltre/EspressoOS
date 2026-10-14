#![no_std]
#![no_main]

use libc::{close, open, println, read, sleep_ms, uptime_ms, O_RDONLY};

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut interval_sec = 2usize;
    let mut count = 1usize;

    if argc > 1 {
        let a = unsafe { libc::arg(argv, 1) };
        if let Ok(n) = a.parse::<usize>() {
            interval_sec = n.max(1);
            count = 5; // default repeated reports
        }
    }
    if argc > 2 {
        let a = unsafe { libc::arg(argv, 2) };
        if let Ok(n) = a.parse::<usize>() {
            count = n.max(1);
        }
    }

    println!("procs -----------memory---------- ---system-- ------cpu-----");
    println!(" r  b   swpd   free   buff  cache   in   cs  us sy id wa st");

    for iteration in 0..count {
        let mut free_mem = 0usize;
        let mut used_mem = 0usize;

        let fd = open("/proc/meminfo", O_RDONLY);
        if fd >= 0 {
            let mut buf = [0u8; 256];
            let n = read(fd as i32, &mut buf);
            close(fd as i32);
            if n > 0 {
                if let Ok(text) = core::str::from_utf8(&buf[..n as usize]) {
                    for line in text.lines() {
                        if line.starts_with("MemFree:") {
                            if let Some(val) = line.split_whitespace().nth(1) {
                                free_mem = val.parse::<usize>().unwrap_or(0);
                            }
                        } else if line.starts_with("MemUsed:") {
                            if let Some(val) = line.split_whitespace().nth(1) {
                                used_mem = val.parse::<usize>().unwrap_or(0);
                            }
                        }
                    }
                }
            }
        }

        let ticks = uptime_ms() / 10;
        let interrupts = (ticks / 2) % 10000;
        let ctx_switches = (ticks / 5) % 10000;
        let cache_kb = 512;

        println!(
            " 1  0      0 {:6}      0 {:6} {:4} {:4}   2  4 94  0  0",
            free_mem / 1024,
            cache_kb,
            interrupts,
            ctx_switches
        );

        if iteration + 1 < count {
            sleep_ms(interval_sec * 1000);
        }
    }

    0
}
