#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read, write, uptime_ms, yield_now};

const O_RDONLY: u32 = 1;
const O_WRONLY: u32 = 2;

fn sleep_s(secs: usize) {
    let ms = secs.saturating_mul(1000);
    let start = uptime_ms();
    while uptime_ms().wrapping_sub(start) < ms {
        yield_now();
    }
}

fn print_status() -> i32 {
    let fd = open("/dev/watchdog", O_RDONLY);
    if fd < 0 {
        let pfd = open("/proc/watchdog", O_RDONLY);
        if pfd < 0 {
            println!("wdt: cannot open /dev/watchdog or /proc/watchdog");
            return 1;
        }
        let mut buf = [0u8; 256];
        let n = read(pfd as i32, &mut buf);
        close(pfd as i32);
        if n > 0 {
            let s = core::str::from_utf8(&buf[..n as usize]).unwrap_or("");
            print!("{}", s);
            return 0;
        }
        return 1;
    }

    let mut buf = [0u8; 256];
    let n = read(fd as i32, &mut buf);
    close(fd as i32);
    if n > 0 {
        let s = core::str::from_utf8(&buf[..n as usize]).unwrap_or("");
        print!("{}", s);
        0
    } else {
        println!("wdt: empty read from /dev/watchdog");
        1
    }
}

fn ping_watchdog() -> i32 {
    let fd = open("/dev/watchdog", O_WRONLY);
    if fd < 0 {
        println!("wdt: cannot open /dev/watchdog for writing");
        return 1;
    }
    let ping = [b'w'];
    let n = write(fd as i32, &ping);
    close(fd as i32);
    if n > 0 {
        println!("wdt: ping sent (hardware watchdog fed)");
        0
    } else {
        println!("wdt: failed to write ping to /dev/watchdog");
        1
    }
}

fn feed_loop(count: u32) -> i32 {
    let fd = open("/dev/watchdog", O_WRONLY);
    if fd < 0 {
        println!("wdt: cannot open /dev/watchdog for writing");
        return 1;
    }
    let ping = [b'w'];
    for i in 1..=count {
        let _ = write(fd as i32, &ping);
        println!("wdt: feed {}/{}", i, count);
        sleep_s(1);
    }
    close(fd as i32);
    0
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        return print_status();
    }

    let cmd = unsafe { arg(argv, 1) };
    match cmd {
        "status" => print_status(),
        "ping" | "feed" => {
            if argc >= 3 {
                let cnt_str = unsafe { arg(argv, 2) };
                let mut count = 0u32;
                for b in cnt_str.bytes() {
                    if b >= b'0' && b <= b'9' {
                        count = count.saturating_mul(10).saturating_add((b - b'0') as u32);
                    }
                }
                if count == 0 { count = 1; }
                feed_loop(count)
            } else {
                ping_watchdog()
            }
        }
        "help" | "--help" | "-h" => {
            println!("Usage: wdt [command] [args]");
            println!("Commands:");
            println!("  status        Show watchdog status, timeout, feeds count, reset reason");
            println!("  ping          Feed watchdog once");
            println!("  feed [N]      Feed watchdog N times (1s interval)");
            println!("  help          Show this help message");
            0
        }
        _ => {
            println!("wdt: unknown command '{}'. Try 'wdt help'", cmd);
            1
        }
    }
}
