#![no_std]
#![no_main]

use libc::{arg, close, mount, open, println, read, O_RDONLY};

fn show_mounts() -> i32 {
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

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc <= 1 {
        return show_mounts();
    }

    let mut fstype = "";
    let mut pos = [""; 2];
    let mut pcount = 0usize;

    let mut i = 1;
    while i < argc {
        let a = unsafe { arg(argv, i) };
        if a == "-h" || a == "--help" {
            println!("Usage: mount [-t fstype] [source] [target]");
            println!("Supported types: ramfs (tmpfs), littlefs, procfs, sysfs, devfs");
            return 0;
        } else if a == "-l" || a == "-a" {
            return show_mounts();
        } else if a == "-t" && i + 1 < argc {
            fstype = unsafe { arg(argv, i + 1) };
            i += 1;
        } else if a == "-o" && i + 1 < argc {
            i += 1;
        } else if !a.starts_with('-') && pcount < 2 {
            pos[pcount] = a;
            pcount += 1;
        }
        i += 1;
    }

    if pcount == 0 {
        return show_mounts();
    }

    let (src, target) = if pcount == 1 {
        ("none", pos[0])
    } else {
        (pos[0], pos[1])
    };

    let fs = if !fstype.is_empty() {
        fstype
    } else if src == "littlefs" || src == "lfs" || src == "/dev/mtd_lfs" {
        "littlefs"
    } else {
        "ramfs"
    };

    let rc = mount(src, target, fs);
    if rc < 0 {
        println!("mount: mounting {} on {} ({}) failed (errno {})", src, target, fs, rc);
        return 1;
    }
    0
}
