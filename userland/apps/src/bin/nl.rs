#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read, O_RDONLY};

const MAX_BUF: usize = 2048;

fn print_num(mut n: usize, width: usize) {
    let mut digits = [0u8; 10];
    let mut len = 0;
    if n == 0 {
        digits[0] = b'0';
        len = 1;
    } else {
        while n > 0 && len < 10 {
            digits[len] = b'0' + (n % 10) as u8;
            n /= 10;
            len += 1;
        }
    }

    let pad = if width > len { width - len } else { 0 };
    for _ in 0..pad {
        print!(" ");
    }
    for i in (0..len).rev() {
        let s = core::slice::from_ref(&digits[i]);
        if let Ok(c) = core::str::from_utf8(s) {
            print!("{}", c);
        }
    }
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut number_empty = false;
    let mut file_opt: Option<&str> = None;
    let mut width = 6usize;

    let mut i = 1;
    while i < argc {
        let a = unsafe { arg(argv, i) };
        if a == "-ba" {
            number_empty = true;
        } else if a == "-bt" {
            number_empty = false;
        } else if a == "-w" {
            i += 1;
            if i < argc {
                if let Ok(w) = unsafe { arg(argv, i) }.parse::<usize>() {
                    width = w;
                }
            }
        } else if a == "-h" || a == "--help" {
            println!("usage: nl [-ba] [-bt] [-w width] [file]");
            println!("       Number lines of files");
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
                println!("nl: cannot open {}", path);
                return 1;
            }
            let count = read(fd as i32, &mut buf);
            close(fd as i32);
            count
        }
        None => read(0, &mut buf), // read from stdin
    };

    if n <= 0 {
        return 0;
    }

    let content = match core::str::from_utf8(&buf[..n as usize]) {
        Ok(s) => s,
        Err(_) => return 1,
    };

    let mut line_no = 1;
    for line in content.lines() {
        if line.is_empty() && !number_empty {
            println!();
        } else {
            print_num(line_no, width);
            println!("\t{}", line);
            line_no += 1;
        }
    }

    0
}
