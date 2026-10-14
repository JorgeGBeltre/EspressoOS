#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read, O_RDONLY};

const MAX_BUF: usize = 2048;

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut tabstop: usize = 8;
    let mut file_opt: Option<&str> = None;

    let mut i = 1;
    while i < argc {
        let a = unsafe { arg(argv, i) };
        if a == "-t" {
            i += 1;
            if i < argc {
                if let Ok(ts) = unsafe { arg(argv, i) }.parse::<usize>() {
                    tabstop = if ts > 0 { ts } else { 8 };
                }
            }
        } else if a == "-h" || a == "--help" {
            println!("usage: unexpand [-t tabstop] [file]");
            println!("       Convert spaces to tabs");
            return 0;
        } else if !a.starts_with('-') {
            file_opt = Some(a);
        }
        i += 1;
    }

    let mut buf = [0u8; MAX_BUF];
    let n = match file_opt {
        Some(path) => {
            let fd = open(path, O_RDONLY);
            if fd < 0 {
                println!("unexpand: cannot open {}", path);
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
        let mut col = 0;
        let mut pending_spaces = 0;

        for c in line.chars() {
            if c == ' ' {
                pending_spaces += 1;
                col += 1;
                if col % tabstop == 0 {
                    print!("\t");
                    pending_spaces = 0;
                }
            } else {
                for _ in 0..pending_spaces {
                    print!(" ");
                }
                pending_spaces = 0;
                print!("{}", c);
                col += 1;
            }
        }
        for _ in 0..pending_spaces {
            print!(" ");
        }
        println!();
    }

    0
}
