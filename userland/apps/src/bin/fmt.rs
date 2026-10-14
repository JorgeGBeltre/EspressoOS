#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read, O_RDONLY};

const STDOUT: i32 = 1;

fn parse_u32(s: &str) -> Option<usize> {
    if s.is_empty() {
        return None;
    }
    let mut n = 0usize;
    for &b in s.as_bytes() {
        if b < b'0' || b > b'9' {
            return None;
        }
        n = n.saturating_mul(10).saturating_add((b - b'0') as usize);
    }
    Some(n)
}

fn write_stdout(bytes: &[u8]) {
    let _ = libc::write(STDOUT, bytes);
}

struct Formatter {
    width: usize,
    split_only: bool,
    cur_line: [u8; 256],
    cur_len: usize,
}

impl Formatter {
    fn new(width: usize, split_only: bool) -> Self {
        Self {
            width: if width == 0 { 75 } else { width },
            split_only,
            cur_line: [0u8; 256],
            cur_len: 0,
        }
    }

    fn flush_line(&mut self) {
        if self.cur_len > 0 {
            write_stdout(&self.cur_line[..self.cur_len]);
            write_stdout(b"\n");
            self.cur_len = 0;
        }
    }

    fn add_word(&mut self, word: &[u8]) {
        if word.is_empty() {
            return;
        }
        if self.cur_len == 0 {
            let take = core::cmp::min(word.len(), self.cur_line.len());
            self.cur_line[..take].copy_from_slice(&word[..take]);
            self.cur_len = take;
        } else if self.cur_len + 1 + word.len() <= self.width && self.cur_len + 1 + word.len() <= self.cur_line.len() {
            self.cur_line[self.cur_len] = b' ';
            self.cur_len += 1;
            self.cur_line[self.cur_len..self.cur_len + word.len()].copy_from_slice(word);
            self.cur_len += word.len();
        } else {
            self.flush_line();
            let take = core::cmp::min(word.len(), self.cur_line.len());
            self.cur_line[..take].copy_from_slice(&word[..take]);
            self.cur_len = take;
        }
    }

    fn process_line(&mut self, line: &[u8]) {
        let is_blank = line.iter().all(|&b| b == b' ' || b == b'\t' || b == b'\r');
        if is_blank {
            self.flush_line();
            write_stdout(b"\n");
            return;
        }

        if self.split_only {
            if line.len() <= self.width {
                write_stdout(line);
                write_stdout(b"\n");
                return;
            }
            // Split line at words
            let mut i = 0;
            while i < line.len() {
                while i < line.len() && (line[i] == b' ' || line[i] == b'\t') {
                    i += 1;
                }
                if i >= line.len() {
                    break;
                }
                let start = i;
                while i < line.len() && line[i] != b' ' && line[i] != b'\t' {
                    i += 1;
                }
                self.add_word(&line[start..i]);
            }
            self.flush_line();
        } else {
            let mut i = 0;
            while i < line.len() {
                while i < line.len() && (line[i] == b' ' || line[i] == b'\t') {
                    i += 1;
                }
                if i >= line.len() {
                    break;
                }
                let start = i;
                while i < line.len() && line[i] != b' ' && line[i] != b'\t' {
                    i += 1;
                }
                self.add_word(&line[start..i]);
            }
        }
    }

    fn finish(&mut self) {
        self.flush_line();
    }
}

fn format_fd(fd: i32, fmt: &mut Formatter) {
    let mut buf = [0u8; 512];
    let mut line_buf = [0u8; 256];
    let mut line_len = 0usize;

    loop {
        let n = read(fd, &mut buf);
        if n <= 0 {
            break;
        }
        for &b in &buf[..n as usize] {
            if b == b'\n' {
                fmt.process_line(&line_buf[..line_len]);
                line_len = 0;
            } else if b != b'\r' {
                if line_len < line_buf.len() {
                    line_buf[line_len] = b;
                    line_len += 1;
                }
            }
        }
    }
    if line_len > 0 {
        fmt.process_line(&line_buf[..line_len]);
    }
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut width = 75usize;
    let mut split_only = false;
    let mut file_start = 1i32;

    let mut i = 1;
    while i < argc {
        let s = unsafe { arg(argv, i) };
        if s == "-h" || s == "--help" {
            println!("Usage: fmt [-w width] [-s] [file...]");
            println!("Reformat text paragraphs to specified column width");
            println!("  -w, -WIDTH  Maximum line width (default: 75)");
            println!("  -s          Split lines only, do not join");
            return 0;
        } else if s == "-s" {
            split_only = true;
            file_start = i + 1;
        } else if s == "-w" {
            if i + 1 < argc {
                let w_str = unsafe { arg(argv, i + 1) };
                if let Some(w) = parse_u32(w_str) {
                    width = w;
                    i += 1;
                    file_start = i + 1;
                }
            }
        } else if s.starts_with("-w") && s.len() > 2 {
            if let Some(w) = parse_u32(&s[2..]) {
                width = w;
                file_start = i + 1;
            }
        } else if s.starts_with('-') && s.len() > 1 && s.as_bytes()[1] >= b'0' && s.as_bytes()[1] <= b'9' {
            if let Some(w) = parse_u32(&s[1..]) {
                width = w;
                file_start = i + 1;
            }
        } else if !s.starts_with('-') {
            file_start = i;
            break;
        }
        i += 1;
    }

    let mut fmt = Formatter::new(width, split_only);

    if file_start >= argc || (file_start == argc - 1 && unsafe { arg(argv, file_start) } == "-") {
        format_fd(0, &mut fmt);
    } else {
        for f in file_start..argc {
            let filename = unsafe { arg(argv, f) };
            if filename == "-" {
                format_fd(0, &mut fmt);
                continue;
            }
            let fd = open(filename, O_RDONLY);
            if fd < 0 {
                print!("fmt: cannot open ");
                println!("{}", filename);
                continue;
            }
            format_fd(fd as i32, &mut fmt);
            close(fd as i32);
        }
    }
    fmt.finish();
    0
}
