#![no_std]
#![no_main]

use libc::{arg, close, open, println, read, write};

const O_RDONLY: u32 = 1;
const O_WRONLY: u32 = 0x0002;
const O_CREATE: u32 = 0x0100;
const O_TRUNC: u32 = 0x0400;

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc != 3 {
        println!("usage: cp SOURCE DEST");
        return 1;
    }
    let src = unsafe { arg(argv, 1) };
    let dst = unsafe { arg(argv, 2) };

    let sfd = open(src, O_RDONLY);
    if sfd < 0 {
        println!("cp: cannot open source '{}'", src);
        return 1;
    }

    let dfd = open(dst, O_WRONLY | O_CREATE | O_TRUNC);
    if dfd < 0 {
        println!("cp: cannot create destination '{}'", dst);
        let _ = close(sfd as i32);
        return 1;
    }

    let mut buf = [0u8; 256];
    let mut status = 0;
    loop {
        let n = read(sfd as i32, &mut buf);
        if n < 0 {
            println!("cp: error reading '{}'", src);
            status = 1;
            break;
        }
        if n == 0 {
            break;
        }
        let mut written = 0usize;
        while written < n as usize {
            let w = write(dfd as i32, &buf[written..n as usize]);
            if w <= 0 {
                println!("cp: error writing '{}'", dst);
                status = 1;
                break;
            }
            written += w as usize;
        }
        if status != 0 {
            break;
        }
    }

    let _ = close(sfd as i32);
    let _ = close(dfd as i32);
    status
}
