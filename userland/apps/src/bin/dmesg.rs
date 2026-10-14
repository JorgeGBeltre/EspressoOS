#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read, write, O_RDONLY};

const STDOUT: i32 = 1;

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut log_path = "/proc/dmesg";

    if argc > 1 {
        let first = unsafe { arg(argv, 1) };
        if first == "-h" || first == "--help" {
            println!("Usage: dmesg [-k | -s] [FILE]");
            println!("Print or control the kernel ring buffer and diagnostic messages");
            println!("  -k, --kernel  Read kernel ring buffer from /proc/dmesg (default)");
            println!("  -s, --syslog  Read persistent system logs from /var/log/messages");
            return 0;
        } else if first == "-s" || first == "--syslog" {
            log_path = "/var/log/messages";
        } else if first == "-k" || first == "--kernel" {
            log_path = "/proc/dmesg";
        } else if !first.starts_with('-') {
            log_path = first;
        }
    }

    let mut fd = open(log_path, O_RDONLY);
    if fd < 0 && log_path == "/var/log/messages" {
        // Fallback to /var/log/syslog if /var/log/messages is not present
        fd = open("/var/log/syslog", O_RDONLY);
    }
    if fd < 0 {
        print!("dmesg: cannot open ");
        println!("{}", log_path);
        return 1;
    }

    let mut buf = [0u8; 256];
    loop {
        let n = read(fd as i32, &mut buf);
        if n <= 0 {
            break;
        }
        let _ = write(STDOUT, &buf[..n as usize]);
    }
    close(fd as i32);
    0
}
