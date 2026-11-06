#![no_std]
#![no_main]

use libc::{arg, close, open, println, read, write, O_APPEND, O_CREATE, O_TRUNC, O_WRONLY};

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut append = false;
    let mut files = [""; 8];
    let mut file_count = 0usize;

    let mut i = 1;
    while i < argc {
        let a = unsafe { arg(argv, i) };
        if a == "-h" || a == "--help" {
            println!("Usage: tee [-a] [FILE...]");
            return 0;
        } else if a == "-a" || a == "--append" {
            append = true;
        } else if file_count < files.len() {
            files[file_count] = a;
            file_count += 1;
        }
        i += 1;
    }

    let flags = if append {
        O_WRONLY | O_CREATE | O_APPEND
    } else {
        O_WRONLY | O_CREATE | O_TRUNC
    };

    let mut fds = [-1i32; 8];
    let mut status = 0;
    for k in 0..file_count {
        let fd = open(files[k], flags);
        if fd < 0 {
            println!("tee: cannot open '{}'", files[k]);
            status = 1;
        } else {
            fds[k] = fd as i32;
        }
    }

    let mut buf = [0u8; 256];
    loop {
        let n = read(0, &mut buf);
        if n <= 0 {
            break;
        }
        let chunk = &buf[..n as usize];
        let _ = write(1, chunk);
        for k in 0..file_count {
            if fds[k] >= 0 {
                let _ = write(fds[k], chunk);
            }
        }
    }

    for k in 0..file_count {
        if fds[k] >= 0 {
            close(fds[k]);
        }
    }
    status
}
