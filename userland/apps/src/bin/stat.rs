#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, readdir, seek};

const O_RDONLY: u32 = 1;

fn print_dec(mut n: usize) {
    if n == 0 {
        print!("0");
        return;
    }
    let mut buf = [0u8; 16];
    let mut i = 0;
    while n > 0 {
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        i += 1;
    }
    while i > 0 {
        i -= 1;
        let s = core::slice::from_ref(&buf[i]);
        if let Ok(c) = core::str::from_utf8(s) {
            print!("{}", c);
        }
    }
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        println!("Usage: stat <file_or_dir>");
        return 1;
    }

    let path = unsafe { arg(argv, 1) };
    let fd = open(path, O_RDONLY);

    if fd >= 0 {
        let size = seek(fd as i32, 0, 2);
        close(fd as i32);

        print!("  File: ");
        println!("{}", path);
        print!("  Size: ");
        if size >= 0 {
            print_dec(size as usize);
            println!(" bytes");
        } else {
            println!("0 bytes (device/stream)");
        }
        if path.starts_with("/dev/") {
            println!("  Type: character device");
        } else if path.starts_with("/proc/") || path.starts_with("/sys/") {
            println!("  Type: pseudo file");
        } else {
            println!("  Type: regular file");
        }
        return 0;
    }

    // Try readdir to see if it is a directory
    let mut dir_buf = [0u8; 256];
    if readdir(path, &mut dir_buf) >= 0 {
        print!("  File: ");
        println!("{}", path);
        println!("  Type: directory");
        return 0;
    }

    println!("stat: cannot stat '{}': No such file or directory", path);
    1
}
