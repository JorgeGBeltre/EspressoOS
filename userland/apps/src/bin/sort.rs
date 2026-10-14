#![no_std]
#![no_main]

use libc::{arg, close, open, println, read};

const O_RDONLY: u32 = 1;
const MAX_LINES: usize = 128;
const LINE_BUF_SIZE: usize = 96;

struct LineEntry {
    len: usize,
    buf: [u8; LINE_BUF_SIZE],
}

impl LineEntry {
    const fn new() -> Self {
        Self {
            len: 0,
            buf: [0u8; LINE_BUF_SIZE],
        }
    }

    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }

    fn parse_num(&self) -> i32 {
        let s = self.as_str().trim_start();
        let mut sign = 1;
        let mut idx = 0;
        let b = s.as_bytes();
        if idx < b.len() && b[idx] == b'-' {
            sign = -1;
            idx += 1;
        } else if idx < b.len() && b[idx] == b'+' {
            idx += 1;
        }
        let mut val = 0i32;
        while idx < b.len() && b[idx] >= b'0' && b[idx] <= b'9' {
            val = val.saturating_mul(10).saturating_add((b[idx] - b'0') as i32);
            idx += 1;
        }
        val * sign
    }
}

fn compare_lines(a: &LineEntry, b: &LineEntry, numeric: bool) -> core::cmp::Ordering {
    if numeric {
        let na = a.parse_num();
        let nb = b.parse_num();
        na.cmp(&nb)
    } else {
        a.as_str().cmp(b.as_str())
    }
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut reverse = false;
    let mut numeric = false;
    let mut file_idx = 0;

    for i in 1..argc {
        let a = unsafe { arg(argv, i) };
        if a == "-r" {
            reverse = true;
        } else if a == "-n" {
            numeric = true;
        } else if a == "-rn" || a == "-nr" {
            reverse = true;
            numeric = true;
        } else if a == "-h" || a == "--help" {
            println!("usage: sort [-r] [-n] [FILE]");
            println!("Sort lines of text files or standard input");
            println!("  -r  Reverse the result of comparisons");
            println!("  -n  Compare according to string numerical value");
            return 0;
        } else if !a.starts_with('-') && file_idx == 0 {
            file_idx = i;
        }
    }

    let fd = if file_idx > 0 {
        let path = unsafe { arg(argv, file_idx) };
        let f = open(path, O_RDONLY);
        if f < 0 {
            println!("sort: cannot open '{}'", path);
            return 2;
        }
        f as i32
    } else {
        0 // stdin
    };

    let mut lines: [LineEntry; MAX_LINES] = [const { LineEntry::new() }; MAX_LINES];
    let mut line_count = 0;

    let mut chunk = [0u8; 128];
    let mut cur_line = LineEntry::new();

    loop {
        let n = read(fd, &mut chunk);
        if n <= 0 {
            break;
        }
        for &byte in &chunk[..n as usize] {
            if byte == b'\n' {
                if line_count < MAX_LINES {
                    lines[line_count] = cur_line;
                    line_count += 1;
                }
                cur_line = LineEntry::new();
            } else if cur_line.len < LINE_BUF_SIZE {
                cur_line.buf[cur_line.len] = byte;
                cur_line.len += 1;
            }
        }
    }

    if cur_line.len > 0 && line_count < MAX_LINES {
        lines[line_count] = cur_line;
        line_count += 1;
    }

    if fd > 0 {
        close(fd);
    }

    // Simple bubble sort
    if line_count > 1 {
        for i in 0..line_count - 1 {
            for j in 0..line_count - 1 - i {
                let ord = compare_lines(&lines[j], &lines[j + 1], numeric);
                let should_swap = if reverse {
                    ord == core::cmp::Ordering::Less
                } else {
                    ord == core::cmp::Ordering::Greater
                };
                if should_swap {
                    lines.swap(j, j + 1);
                }
            }
        }
    }

    for line in &lines[..line_count] {
        println!("{}", line.as_str());
    }

    0
}
