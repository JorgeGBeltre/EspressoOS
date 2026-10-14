#![no_std]
#![no_main]

use libc::{arg, close, open, println, read, write};

const O_RDONLY: u32 = 1;
const STDIN: i32 = 0;
const STDOUT: i32 = 1;

fn run_head(fd: i32, max_lines: usize) {
    let mut buf = [0u8; 128];
    let mut lines = 0usize;
    loop {
        let n = read(fd, &mut buf);
        if n <= 0 {
            break;
        }
        let len = n as usize;
        let mut start = 0usize;
        for i in 0..len {
            if buf[i] == b'\n' {
                lines += 1;
                if lines >= max_lines {
                    let _ = write(STDOUT, &buf[start..=i]);
                    return;
                }
            }
        }
        let _ = write(STDOUT, &buf[start..len]);
    }
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut max_lines = 10usize;
    let mut file_start = 1;

    if argc > 1 {
        let first = unsafe { arg(argv, 1) };
        if first.starts_with("-n") {
            if first.len() > 2 {
                if let Ok(n) = parse_usize(&first[2..]) {
                    max_lines = n;
                    file_start = 2;
                }
            } else if argc > 2 {
                let second = unsafe { arg(argv, 2) };
                if let Ok(n) = parse_usize(second) {
                    max_lines = n;
                    file_start = 3;
                }
            }
        }
    }

    if file_start >= argc {
        run_head(STDIN, max_lines);
        return 0;
    }

    for i in file_start..argc {
        let path = unsafe { arg(argv, i) };
        let fd = open(path, O_RDONLY);
        if fd < 0 {
            println!("head: cannot open '{}'", path);
            continue;
        }
        run_head(fd as i32, max_lines);
        let _ = close(fd as i32);
    }
    0
}

fn parse_usize(s: &str) -> Result<usize, ()> {
    let mut val = 0usize;
    for b in s.bytes() {
        if b >= b'0' && b <= b'9' {
            val = val * 10 + (b - b'0') as usize;
        } else {
            return Err(());
        }
    }
    Ok(val)
}
