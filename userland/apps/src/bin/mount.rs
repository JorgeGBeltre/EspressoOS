#![no_std]
#![no_main]

use libc::{arg, close, open, println, read};

const O_RDONLY: u32 = 1;

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc > 1 {
        let first = unsafe { arg(argv, 1) };
        if first == "-h" || first == "--help" {
            println!("Usage: mount [-t fstype] [device] [dir]");
            return 0;
        }
    }

    let fd = open("/proc/mounts", O_RDONLY);
    if fd < 0 {
        println!("mount: cannot open /proc/mounts");
        return 1;
    }

    let mut buf = [0u8; 512];
    let mut total = 0usize;
    loop {
        let n = read(fd as i32, &mut buf[total..]);
        if n <= 0 {
            break;
        }
        total += n as usize;
        if total == buf.len() {
            break;
        }
    }
    close(fd as i32);

    let text = match core::str::from_utf8(&buf[..total]) {
        Ok(s) => s,
        Err(_) => {
            println!("mount: corrupt /proc/mounts");
            return 1;
        }
    };

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        let dev = parts.next().unwrap_or("none");
        let mnt = parts.next().unwrap_or("/");
        let fstype = parts.next().unwrap_or("unknown");
        let opts = parts.next().unwrap_or("rw");
        println!("{} on {} type {} ({})", dev, mnt, fstype, opts);
    }
    0
}
