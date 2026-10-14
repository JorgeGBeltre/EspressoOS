#![no_std]
#![no_main]

use libc::{arg, close, open, println, read, write};

const O_RDONLY: u32 = 1;
const STDIN: i32 = 0;
const STDOUT: i32 = 1;

fn grep_fd(fd: i32, pattern: &str, prefix: Option<&str>) -> bool {
    let mut buf = [0u8; 256];
    let mut line_buf = [0u8; 256];
    let mut line_len = 0usize;
    let mut matched_any = false;
    let pat_bytes = pattern.as_bytes();

    loop {
        let n = read(fd, &mut buf);
        if n <= 0 {
            break;
        }
        for &b in &buf[..n as usize] {
            if b == b'\n' {
                if contains_sub(&line_buf[..line_len], pat_bytes) {
                    matched_any = true;
                    if let Some(p) = prefix {
                        let _ = write(STDOUT, p.as_bytes());
                        let _ = write(STDOUT, b":");
                    }
                    let _ = write(STDOUT, &line_buf[..line_len]);
                    let _ = write(STDOUT, b"\n");
                }
                line_len = 0;
            } else if line_len < line_buf.len() {
                line_buf[line_len] = b;
                line_len += 1;
            }
        }
    }
    if line_len > 0 && contains_sub(&line_buf[..line_len], pat_bytes) {
        matched_any = true;
        if let Some(p) = prefix {
            let _ = write(STDOUT, p.as_bytes());
            let _ = write(STDOUT, b":");
        }
        let _ = write(STDOUT, &line_buf[..line_len]);
        let _ = write(STDOUT, b"\n");
    }
    matched_any
}

fn contains_sub(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    if haystack.len() < needle.len() {
        return false;
    }
    for i in 0..=haystack.len() - needle.len() {
        if &haystack[i..i + needle.len()] == needle {
            return true;
        }
    }
    false
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        println!("usage: grep PATTERN [FILE...]");
        return 2;
    }

    let pattern = unsafe { arg(argv, 1) };

    if argc == 2 {
        return if grep_fd(STDIN, pattern, None) { 0 } else { 1 };
    }

    let multiple = argc > 3;
    let mut any_match = false;

    for i in 2..argc {
        let path = unsafe { arg(argv, i) };
        let fd = open(path, O_RDONLY);
        if fd < 0 {
            println!("grep: cannot open '{}'", path);
            continue;
        }
        let prefix = if multiple { Some(path) } else { None };
        if grep_fd(fd as i32, pattern, prefix) {
            any_match = true;
        }
        let _ = close(fd as i32);
    }

    if any_match { 0 } else { 1 }
}
