#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read, uptime_ms, yield_now};

const O_RDONLY: u32 = 1;

fn read_file<'a>(path: &str, buf: &'a mut [u8]) -> &'a str {
    let fd = open(path, O_RDONLY);
    if fd < 0 {
        return "";
    }
    let mut total = 0usize;
    while total < buf.len() {
        let n = read(fd as i32, &mut buf[total..]);
        if n <= 0 { break; }
        total += n as usize;
    }
    close(fd as i32);
    core::str::from_utf8(&buf[..total]).unwrap_or("")
}

fn sleep_s(secs: usize) {
    let ms = secs.saturating_mul(1000);
    let start = uptime_ms();
    while uptime_ms().wrapping_sub(start) < ms {
        yield_now();
    }
}

fn display_top() {
    let mut mem_buf = [0u8; 256];
    let mut tasks_buf = [0u8; 1024];

    let mem_str = read_file("/proc/meminfo", &mut mem_buf);
    let tasks_str = read_file("/proc/stacks", &mut tasks_buf);
    let ms = uptime_ms();

    // Clear screen and position cursor to top-left
    print!("\x1b[2J\x1b[H");

    println!("EspressoOS top - uptime: {}.{:03}s - CPU: Xtensa LX7 dual-core @ 240MHz", ms / 1000, ms % 1000);

    for line in mem_str.split('\n') {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            println!("  {}", trimmed);
        }
    }
    println!("--------------------------------------------------------------------------------");
    println!("Tasks & Execution Stacks:");
    for line in tasks_str.split('\n') {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            println!("  {}", trimmed);
        }
    }
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut iterations = 1usize;
    if argc >= 3 {
        let flag = unsafe { arg(argv, 1) };
        if flag == "-n" {
            let n_str = unsafe { arg(argv, 2) };
            let mut val = 0usize;
            for b in n_str.bytes() {
                if b >= b'0' && b <= b'9' {
                    val = val.saturating_mul(10).saturating_add((b - b'0') as usize);
                }
            }
            if val > 0 { iterations = val; }
        }
    } else if argc == 2 {
        let flag = unsafe { arg(argv, 1) };
        if flag == "-h" || flag == "--help" {
            println!("Usage: top [-n ITERATIONS]");
            println!("Display real-time EspressoOS kernel tasks, CPU, and memory metrics.");
            return 0;
        }
    }

    for i in 0..iterations {
        display_top();
        if i + 1 < iterations {
            sleep_s(1);
        }
    }
    0
}
