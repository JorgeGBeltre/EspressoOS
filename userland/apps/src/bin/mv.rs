#![no_std]
#![no_main]

use libc::{arg, close, open, println, read, unlink, write};

const O_RDONLY: u32 = 1;
const O_WRONLY_CREATE_TRUNC: u32 = 2 | 4 | 8;

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 3 {
        println!("Usage: mv <source> <destination>");
        return 1;
    }

    let src = unsafe { arg(argv, 1) };
    let dst = unsafe { arg(argv, 2) };

    if src == dst {
        println!("mv: '{}' and '{}' are the same file", src, dst);
        return 1;
    }

    let in_fd = open(src, O_RDONLY);
    if in_fd < 0 {
        println!("mv: cannot stat '{}': No such file or directory", src);
        return 1;
    }

    let out_fd = open(dst, O_WRONLY_CREATE_TRUNC);
    if out_fd < 0 {
        println!("mv: cannot create destination '{}'", dst);
        close(in_fd as i32);
        return 1;
    }

    let mut buf = [0u8; 512];
    let mut err = false;

    loop {
        let n = read(in_fd as i32, &mut buf);
        if n < 0 {
            println!("mv: error reading from '{}'", src);
            err = true;
            break;
        }
        if n == 0 {
            break;
        }
        let w = write(out_fd as i32, &buf[..n as usize]);
        if w < n {
            println!("mv: error writing to '{}'", dst);
            err = true;
            break;
        }
    }

    close(in_fd as i32);
    close(out_fd as i32);

    if err {
        let _ = unlink(dst);
        return 1;
    }

    if unlink(src) < 0 {
        println!("mv: warning: copied to '{}' but failed to remove source '{}'", dst, src);
        return 1;
    }

    0
}
