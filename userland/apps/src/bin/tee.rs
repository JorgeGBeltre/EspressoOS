#![no_std]
#![no_main]

use libc::{arg, close, open, println, read, seek, write};

const O_WRONLY_CREATE: u32 = 2 | 4;
const O_TRUNC: u32 = 8;
const SEEK_END: i32 = 2;

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        println!("Usage: tee [-a] <file>");
        return 1;
    }

    let mut append = false;
    let mut filename = "";

    if argc >= 3 {
        let opt = unsafe { arg(argv, 1) };
        if opt == "-a" {
            append = true;
            filename = unsafe { arg(argv, 2) };
        } else {
            filename = unsafe { arg(argv, 1) };
        }
    } else {
        filename = unsafe { arg(argv, 1) };
    }

    let flags = if append {
        O_WRONLY_CREATE
    } else {
        O_WRONLY_CREATE | O_TRUNC
    };

    let fd = open(filename, flags);
    if fd < 0 {
        println!("tee: cannot open '{}'", filename);
        return 1;
    }

    if append {
        let _ = seek(fd as i32, 0, SEEK_END);
    }

    let mut buf = [0u8; 256];
    loop {
        let n = read(0, &mut buf);
        if n <= 0 {
            break;
        }
        let chunk = &buf[..n as usize];
        let _ = write(1, chunk);
        let _ = write(fd as i32, chunk);
    }

    close(fd as i32);
    0
}
