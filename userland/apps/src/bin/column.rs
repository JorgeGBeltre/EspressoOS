#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read, O_RDONLY};

const MAX_LINES: usize = 32;
const MAX_LINE_LEN: usize = 128;
const MAX_COLS: usize = 8;

struct TableBatch {
    lines: [[u8; MAX_LINE_LEN]; MAX_LINES],
    line_lens: [usize; MAX_LINES],
    count: usize,
    delim: u8,
    use_custom_delim: bool,
}

impl TableBatch {
    fn new(delim: u8, use_custom: bool) -> Self {
        Self {
            lines: [[0u8; MAX_LINE_LEN]; MAX_LINES],
            line_lens: [0usize; MAX_LINES],
            count: 0,
            delim,
            use_custom_delim: use_custom,
        }
    }

    fn add_line(&mut self, data: &[u8]) {
        if self.count >= MAX_LINES {
            self.flush();
        }
        let len = core::cmp::min(data.len(), MAX_LINE_LEN);
        self.lines[self.count][..len].copy_from_slice(&data[..len]);
        self.line_lens[self.count] = len;
        self.count += 1;
    }

    fn flush(&mut self) {
        if self.count == 0 {
            return;
        }

        let mut col_widths = [0usize; MAX_COLS];

        // First pass: measure column widths
        for i in 0..self.count {
            let line_bytes = &self.lines[i][..self.line_lens[i]];
            let mut col_idx = 0usize;
            let mut col_start = 0usize;
            let mut idx = 0usize;

            while idx <= line_bytes.len() {
                let is_end = idx == line_bytes.len();
                let is_sep = if is_end {
                    false
                } else if self.use_custom_delim {
                    line_bytes[idx] == self.delim
                } else {
                    line_bytes[idx] == b' ' || line_bytes[idx] == b'\t'
                };

                if is_end || is_sep {
                    if idx > col_start || self.use_custom_delim {
                        let w = idx - col_start;
                        if col_idx < MAX_COLS {
                            col_widths[col_idx] = core::cmp::max(col_widths[col_idx], w);
                            col_idx += 1;
                        }
                    }
                    col_start = idx + 1;
                }
                idx += 1;
            }
        }

        // Second pass: print aligned table
        for i in 0..self.count {
            let line_bytes = &self.lines[i][..self.line_lens[i]];
            let mut col_idx = 0usize;
            let mut col_start = 0usize;
            let mut idx = 0usize;

            while idx <= line_bytes.len() {
                let is_end = idx == line_bytes.len();
                let is_sep = if is_end {
                    false
                } else if self.use_custom_delim {
                    line_bytes[idx] == self.delim
                } else {
                    line_bytes[idx] == b' ' || line_bytes[idx] == b'\t'
                };

                if is_end || is_sep {
                    if idx > col_start || self.use_custom_delim {
                        let field_bytes = &line_bytes[col_start..idx];
                        if let Ok(s) = core::str::from_utf8(field_bytes) {
                            print!("{}", s);
                        }
                        if !is_end && col_idx < MAX_COLS {
                            let width = col_widths[col_idx];
                            let pad = (width + 2).saturating_sub(field_bytes.len());
                            for _ in 0..pad {
                                print!(" ");
                            }
                        }
                        col_idx += 1;
                    }
                    col_start = idx + 1;
                }
                idx += 1;
            }
            println!("");
        }

        self.count = 0;
    }
}

fn process_stream(fd: i32, batch: &mut TableBatch) {
    let mut buf = [0u8; 256];
    let mut line_buf = [0u8; MAX_LINE_LEN];
    let mut line_len = 0usize;

    loop {
        let n = read(fd, &mut buf);
        if n <= 0 {
            break;
        }
        for &b in &buf[..n as usize] {
            if b == b'\n' {
                batch.add_line(&line_buf[..line_len]);
                line_len = 0;
            } else if b != b'\r' && line_len < line_buf.len() {
                line_buf[line_len] = b;
                line_len += 1;
            }
        }
    }
    if line_len > 0 {
        batch.add_line(&line_buf[..line_len]);
    }
    batch.flush();
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut delim = b' ';
    let mut use_custom_delim = false;
    let mut _table_mode = false;
    let mut arg_idx = 1;

    while arg_idx < argc {
        let s = unsafe { arg(argv, arg_idx) };
        if s == "-h" || s == "--help" {
            println!("Usage: column [-t] [-s DELIM] [FILE...]");
            println!("Columnate lists or format files into aligned tables");
            println!("  -t         Create a table with aligned columns");
            println!("  -s DELIM   Specify delimiter characters for table splitting");
            return 0;
        } else if s == "-t" {
            _table_mode = true;
            arg_idx += 1;
        } else if s == "-s" {
            if arg_idx + 1 < argc {
                let d = unsafe { arg(argv, arg_idx + 1) };
                if !d.is_empty() {
                    delim = d.as_bytes()[0];
                    use_custom_delim = true;
                }
                arg_idx += 2;
            } else {
                println!("column: option '-s' requires an argument");
                return 1;
            }
        } else if s.starts_with("-s") {
            let d = &s[2..];
            if !d.is_empty() {
                delim = d.as_bytes()[0];
                use_custom_delim = true;
            }
            arg_idx += 1;
        } else {
            break;
        }
    }

    let mut batch = TableBatch::new(delim, use_custom_delim);

    if arg_idx >= argc {
        process_stream(0, &mut batch);
    } else {
        for i in arg_idx..argc {
            let path = unsafe { arg(argv, i) };
            if path == "-" {
                process_stream(0, &mut batch);
            } else {
                let fd = open(path, O_RDONLY);
                if fd < 0 {
                    print!("column: cannot open '");
                    print!("{}", path);
                    println!("'");
                } else {
                    process_stream(fd as i32, &mut batch);
                    close(fd as i32);
                }
            }
        }
    }

    0
}
