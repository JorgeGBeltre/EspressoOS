#![no_std]
#![no_main]

use libc::{arg, close, open, println, read};

const O_RDONLY: u32 = 1;
const LINE_BUF_SIZE: usize = 128;

struct LineBuf {
    len: usize,
    buf: [u8; LINE_BUF_SIZE],
}

impl LineBuf {
    const fn new() -> Self {
        Self {
            len: 0,
            buf: [0u8; LINE_BUF_SIZE],
        }
    }

    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }

    fn equals(&self, other: &LineBuf) -> bool {
        self.len == other.len && self.buf[..self.len] == other.buf[..other.len]
    }

    fn copy_from(&mut self, other: &LineBuf) {
        self.len = other.len;
        self.buf[..self.len].copy_from_slice(&other.buf[..other.len]);
    }
}

fn emit_line(line: &LineBuf, count: usize, count_flag: bool, dup_only: bool) {
    if count == 0 {
        return;
    }
    if dup_only && count < 2 {
        return;
    }
    if count_flag {
        println!("{:4} {}", count, line.as_str());
    } else {
        println!("{}", line.as_str());
    }
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut count_flag = false;
    let mut dup_only = false;
    let mut file_idx = 0;

    for i in 1..argc {
        let a = unsafe { arg(argv, i) };
        if a == "-c" {
            count_flag = true;
        } else if a == "-d" {
            dup_only = true;
        } else if a == "-cd" || a == "-dc" {
            count_flag = true;
            dup_only = true;
        } else if a == "-h" || a == "--help" {
            println!("usage: uniq [-c] [-d] [FILE]");
            println!("Report or omit repeated lines");
            println!("  -c  Prefix lines by the number of occurrences");
            println!("  -d  Only print duplicate lines");
            return 0;
        } else if !a.starts_with('-') && file_idx == 0 {
            file_idx = i;
        }
    }

    let fd = if file_idx > 0 {
        let path = unsafe { arg(argv, file_idx) };
        let f = open(path, O_RDONLY);
        if f < 0 {
            println!("uniq: cannot open '{}'", path);
            return 1;
        }
        f as i32
    } else {
        0 // stdin
    };

    let mut prev_line = LineBuf::new();
    let mut cur_line = LineBuf::new();
    let mut repeat_count = 0usize;
    let mut has_prev = false;

    let mut chunk = [0u8; 128];

    loop {
        let n = read(fd, &mut chunk);
        if n <= 0 {
            break;
        }
        for &byte in &chunk[..n as usize] {
            if byte == b'\n' {
                if !has_prev {
                    prev_line.copy_from(&cur_line);
                    repeat_count = 1;
                    has_prev = true;
                } else if cur_line.equals(&prev_line) {
                    repeat_count += 1;
                } else {
                    emit_line(&prev_line, repeat_count, count_flag, dup_only);
                    prev_line.copy_from(&cur_line);
                    repeat_count = 1;
                }
                cur_line = LineBuf::new();
            } else if cur_line.len < LINE_BUF_SIZE {
                cur_line.buf[cur_line.len] = byte;
                cur_line.len += 1;
            }
        }
    }

    if cur_line.len > 0 {
        if !has_prev {
            prev_line.copy_from(&cur_line);
            repeat_count = 1;
            has_prev = true;
        } else if cur_line.equals(&prev_line) {
            repeat_count += 1;
        } else {
            emit_line(&prev_line, repeat_count, count_flag, dup_only);
            prev_line.copy_from(&cur_line);
            repeat_count = 1;
        }
    }

    if has_prev {
        emit_line(&prev_line, repeat_count, count_flag, dup_only);
    }

    if fd > 0 {
        close(fd);
    }

    0
}
