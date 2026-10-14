#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read};

const O_RDONLY: u32 = 1;

fn process_fd(fd: i32, delim: char, target_field: usize) {
    let mut buf = [0u8; 512];
    let mut line_buf = [0u8; 256];
    let mut line_pos = 0usize;

    loop {
        let n = read(fd, &mut buf);
        if n <= 0 { break; }
        for &b in &buf[..n as usize] {
            if b == b'\n' {
                let line = core::str::from_utf8(&line_buf[..line_pos]).unwrap_or("");
                let mut current_field = 1usize;
                let mut found = false;
                for part in line.split(delim) {
                    if current_field == target_field {
                        println!("{}", part);
                        found = true;
                        break;
                    }
                    current_field += 1;
                }
                if !found {
                    println!("");
                }
                line_pos = 0;
            } else if b != b'\r' {
                if line_pos < line_buf.len() {
                    line_buf[line_pos] = b;
                    line_pos += 1;
                }
            }
        }
    }
    if line_pos > 0 {
        let line = core::str::from_utf8(&line_buf[..line_pos]).unwrap_or("");
        let mut current_field = 1usize;
        let mut found = false;
        for part in line.split(delim) {
            if current_field == target_field {
                println!("{}", part);
                found = true;
                break;
            }
            current_field += 1;
        }
        if !found {
            println!("");
        }
    }
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut delim = '\t';
    let mut field_idx = 1usize;
    let mut file_path: Option<&str> = None;

    let mut i = 1;
    while i < argc {
        let a = unsafe { arg(argv, i) };
        if a == "-d" && i + 1 < argc {
            i += 1;
            let d_str = unsafe { arg(argv, i) };
            if let Some(c) = d_str.chars().next() {
                delim = c;
            }
        } else if a.starts_with("-d") && a.len() > 2 {
            if let Some(c) = a[2..].chars().next() {
                delim = c;
            }
        } else if a == "-f" && i + 1 < argc {
            i += 1;
            let f_str = unsafe { arg(argv, i) };
            let mut val = 0usize;
            for b in f_str.bytes() {
                if b >= b'0' && b <= b'9' {
                    val = val.saturating_mul(10).saturating_add((b - b'0') as usize);
                }
            }
            if val > 0 { field_idx = val; }
        } else if a.starts_with("-f") && a.len() > 2 {
            let mut val = 0usize;
            for b in a[2..].bytes() {
                if b >= b'0' && b <= b'9' {
                    val = val.saturating_mul(10).saturating_add((b - b'0') as usize);
                }
            }
            if val > 0 { field_idx = val; }
        } else if a == "-h" || a == "--help" {
            println!("Usage: cut -d DELIM -f FIELD [FILE]");
            println!("Print selected parts of lines from each FILE to standard output.");
            return 0;
        } else if file_path.is_none() && !a.starts_with("-") {
            file_path = Some(a);
        }
        i += 1;
    }

    if let Some(path) = file_path {
        let fd = open(path, O_RDONLY);
        if fd < 0 {
            println!("cut: cannot open {}", path);
            return 1;
        }
        process_fd(fd as i32, delim, field_idx);
        close(fd as i32);
    } else {
        process_fd(0, delim, field_idx);
    }
    0
}
