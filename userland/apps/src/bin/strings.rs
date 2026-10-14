#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read, O_RDONLY};

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut min_len = 4usize;
    let mut file_path: Option<&str> = None;

    let mut i = 1;
    while i < argc {
        let a = unsafe { arg(argv, i) };
        if a == "-n" && i + 1 < argc {
            i += 1;
            let val = unsafe { arg(argv, i) };
            if let Ok(n) = val.parse::<usize>() {
                min_len = n.max(1);
            }
        } else if !a.starts_with('-') && file_path.is_none() {
            file_path = Some(a);
        }
        i += 1;
    }

    let fd = match file_path {
        Some(path) => {
            let f = open(path, O_RDONLY);
            if f < 0 {
                println!("strings: cannot open {}", path);
                return 1;
            }
            f as i32
        }
        None => 0, // stdin
    };

    let mut read_buf = [0u8; 256];
    let mut str_buf = [0u8; 128];
    let mut str_len = 0usize;

    loop {
        let n = read(fd, &mut read_buf);
        if n <= 0 {
            break;
        }

        for &b in &read_buf[..n as usize] {
            if (32..=126).contains(&b) || b == b'\t' {
                if str_len < str_buf.len() {
                    str_buf[str_len] = b;
                    str_len += 1;
                }
            } else {
                if str_len >= min_len {
                    for &c in &str_buf[..str_len] {
                        print!("{}", c as char);
                    }
                    println!("");
                }
                str_len = 0;
            }
        }
    }

    // Flush any pending trailing string
    if str_len >= min_len {
        for &c in &str_buf[..str_len] {
            print!("{}", c as char);
        }
        println!("");
    }

    if fd > 0 {
        close(fd);
    }
    0
}
