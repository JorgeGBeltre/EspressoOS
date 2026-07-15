#![no_std]
#![no_main]

use libc::{arg, close, open, println, read, write};

const O_RDONLY: u32 = 1;
const STDIN: i32 = 0;
const STDOUT: i32 = 1;

/// Copies a fd to stdout until EOF. Returns false on a read error.
fn drain(fd: i32) -> bool {
    let mut buf = [0u8; 128];
    loop {
        let n = read(fd, &mut buf);
        if n < 0 {
            return false;
        }
        if n == 0 {
            return true;
        }
        let mut done = 0usize;
        while done < n as usize {
            let w = write(STDOUT, &buf[done..n as usize]);
            if w <= 0 {
                return false;
            }
            done += w as usize;
        }
    }
}

/// cat(1). No arguments means stdin, which is the whole point of it in a pipeline.
///
/// It used to open a hardcoded /etc/hosts and never look at fd 0, so `x | cat`
/// printed an error about a file nobody asked for and the pipe was never read.
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc <= 1 {
        return if drain(STDIN) {
            0
        } else {
            println!("cat: read error");
            1
        };
    }

    let mut status = 0;
    for i in 1..argc {
        let path = unsafe { arg(argv, i) };
        let fd = open(path, O_RDONLY);
        if fd < 0 {
            println!("cat: {}: cannot open", path);
            status = 1;
            continue;
        }
        if !drain(fd as i32) {
            println!("cat: {}: read error", path);
            status = 1;
        }
        let _ = close(fd as i32);
    }
    status
}
