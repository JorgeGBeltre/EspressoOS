#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read, O_RDONLY};

const MAX_BUF: usize = 2048;

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut file_opt: Option<&str> = None;

    for i in 1..argc {
        let a = unsafe { arg(argv, i) };
        if a == "-h" || a == "--help" {
            println!("usage: rev [file]");
            println!("       Reverse lines character-wise");
            return 0;
        } else if !a.starts_with('-') {
            file_opt = Some(a);
        }
    }

    let mut buf = [0u8; MAX_BUF];
    let n = match file_opt {
        Some(path) => {
            let fd = open(path, O_RDONLY);
            if fd < 0 {
                println!("rev: cannot open {}", path);
                return 1;
            }
            let count = read(fd as i32, &mut buf);
            close(fd as i32);
            count
        }
        None => read(0, &mut buf),
    };

    if n <= 0 {
        return 0;
    }

    let content = match core::str::from_utf8(&buf[..n as usize]) {
        Ok(s) => s,
        Err(_) => return 1,
    };

    for line in content.lines() {
        let bytes = line.as_bytes();
        for i in (0..bytes.len()).rev() {
            let s = core::slice::from_ref(&bytes[i]);
            if let Ok(c) = core::str::from_utf8(s) {
                print!("{}", c);
            }
        }
        println!();
    }

    0
}
