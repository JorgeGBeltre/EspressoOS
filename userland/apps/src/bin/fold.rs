#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read, O_RDONLY};

const MAX_BUF: usize = 2048;

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut width: usize = 80;
    let mut break_spaces = false;
    let mut file_opt: Option<&str> = None;

    let mut i = 1;
    while i < argc {
        let a = unsafe { arg(argv, i) };
        if a == "-w" {
            i += 1;
            if i < argc {
                if let Ok(w) = unsafe { arg(argv, i) }.parse::<usize>() {
                    width = if w > 0 { w } else { 80 };
                }
            }
        } else if a == "-s" {
            break_spaces = true;
        } else if a == "-h" || a == "--help" {
            println!("usage: fold [-w width] [-s] [file]");
            println!("       Wrap input lines to fit in specified width");
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
                println!("fold: cannot open {}", path);
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
        if line.len() <= width {
            println!("{}", line);
            continue;
        }

        let mut rem = line;
        while !rem.is_empty() {
            if rem.len() <= width {
                println!("{}", rem);
                break;
            }

            let mut split_at = width;
            if break_spaces {
                if let Some(space_idx) = rem[..width].rfind(' ') {
                    if space_idx > 0 {
                        split_at = space_idx;
                    }
                }
            }

            println!("{}", &rem[..split_at]);
            rem = rem[split_at..].trim_start();
        }
    }

    0
}
