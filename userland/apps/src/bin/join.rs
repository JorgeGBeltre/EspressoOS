#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read, O_RDONLY};

struct LineReader {
    fd: i32,
    buf: [u8; 256],
    buf_len: usize,
    buf_pos: usize,
    eof: bool,
}

impl LineReader {
    fn new(fd: i32) -> Self {
        Self {
            fd,
            buf: [0; 256],
            buf_len: 0,
            buf_pos: 0,
            eof: false,
        }
    }

    fn read_line<'a>(&mut self, out: &'a mut [u8]) -> Option<&'a str> {
        let mut out_len = 0usize;
        loop {
            if self.buf_pos >= self.buf_len {
                if self.eof {
                    break;
                }
                let n = read(self.fd, &mut self.buf);
                if n <= 0 {
                    self.eof = true;
                    break;
                }
                self.buf_len = n as usize;
                self.buf_pos = 0;
            }

            while self.buf_pos < self.buf_len {
                let b = self.buf[self.buf_pos];
                self.buf_pos += 1;
                if b == b'\n' {
                    if out_len > 0 && out[out_len - 1] == b'\r' {
                        out_len -= 1;
                    }
                    return core::str::from_utf8(&out[..out_len]).ok();
                } else if out_len < out.len() {
                    out[out_len] = b;
                    out_len += 1;
                }
            }
        }

        if out_len > 0 {
            if out[out_len - 1] == b'\r' {
                out_len -= 1;
            }
            core::str::from_utf8(&out[..out_len]).ok()
        } else {
            None
        }
    }
}

fn get_field<'a>(line: &'a str, field_idx: usize, delim: Option<char>) -> &'a str {
    let mut cur = 1usize;
    if let Some(d) = delim {
        for f in line.split(d) {
            if cur == field_idx {
                return f;
            }
            cur += 1;
        }
    } else {
        for f in line.split_ascii_whitespace() {
            if cur == field_idx {
                return f;
            }
            cur += 1;
        }
    }
    ""
}

fn print_joined(
    key: &str,
    line1: &str,
    field1: usize,
    line2: &str,
    field2: usize,
    delim: Option<char>,
) {
    let sep = delim.unwrap_or(' ');
    print!("{}", key);

    // Print other fields of line1
    let mut cur = 1usize;
    if let Some(d) = delim {
        for f in line1.split(d) {
            if cur != field1 {
                print!("{}{}", sep, f);
            }
            cur += 1;
        }
    } else {
        for f in line1.split_ascii_whitespace() {
            if cur != field1 {
                print!("{}{}", sep, f);
            }
            cur += 1;
        }
    }

    // Print other fields of line2
    cur = 1;
    if let Some(d) = delim {
        for f in line2.split(d) {
            if cur != field2 {
                print!("{}{}", sep, f);
            }
            cur += 1;
        }
    } else {
        for f in line2.split_ascii_whitespace() {
            if cur != field2 {
                print!("{}{}", sep, f);
            }
            cur += 1;
        }
    }

    println!();
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut field1 = 1usize;
    let mut field2 = 1usize;
    let mut delim: Option<char> = None;
    let mut unpairable1 = false;
    let mut unpairable2 = false;

    let mut i = 1;
    while i < argc {
        let s = unsafe { arg(argv, i) };
        if s == "-h" || s == "--help" {
            println!("Usage: join [OPTION]... FILE1 FILE2");
            println!("For each pair of input lines with identical join fields, write a line to standard output.");
            println!("  -1 FIELD    Join on this FIELD of file 1");
            println!("  -2 FIELD    Join on this FIELD of file 2");
            println!("  -j FIELD    Equivalent to -1 FIELD -2 FIELD");
            println!("  -t CHAR     Use CHAR as input and output field separator");
            println!("  -v FILENO   Also print unpairable lines from FILENO (1 or 2)");
            return 0;
        } else if s == "-1" {
            if i + 1 < argc {
                if let Ok(v) = unsafe { arg(argv, i + 1) }.parse::<usize>() {
                    field1 = v.max(1);
                }
                i += 2;
            } else {
                return 1;
            }
        } else if s == "-2" {
            if i + 1 < argc {
                if let Ok(v) = unsafe { arg(argv, i + 1) }.parse::<usize>() {
                    field2 = v.max(1);
                }
                i += 2;
            } else {
                return 1;
            }
        } else if s == "-j" {
            if i + 1 < argc {
                if let Ok(v) = unsafe { arg(argv, i + 1) }.parse::<usize>() {
                    field1 = v.max(1);
                    field2 = field1;
                }
                i += 2;
            } else {
                return 1;
            }
        } else if s == "-t" {
            if i + 1 < argc {
                let ts = unsafe { arg(argv, i + 1) };
                delim = ts.chars().next();
                i += 2;
            } else {
                return 1;
            }
        } else if s == "-v" {
            if i + 1 < argc {
                let vs = unsafe { arg(argv, i + 1) };
                if vs == "1" {
                    unpairable1 = true;
                } else if vs == "2" {
                    unpairable2 = true;
                }
                i += 2;
            } else {
                return 1;
            }
        } else if s.starts_with("-") {
            println!("join: unrecognized option");
            return 1;
        } else {
            break;
        }
    }

    if i + 2 > argc {
        println!("join: missing operand");
        return 1;
    }

    let file1_path = unsafe { arg(argv, i) };
    let file2_path = unsafe { arg(argv, i + 1) };

    let fd1 = if file1_path == "-" {
        0
    } else {
        let fd = open(file1_path, O_RDONLY);
        if fd < 0 {
            print!("join: ");
            print!("{}", file1_path);
            println!(": No such file");
            return 1;
        }
        fd as i32
    };

    let fd2 = if file2_path == "-" {
        0
    } else {
        let fd = open(file2_path, O_RDONLY);
        if fd < 0 {
            print!("join: ");
            print!("{}", file2_path);
            println!(": No such file");
            if fd1 > 0 {
                close(fd1);
            }
            return 1;
        }
        fd as i32
    };

    let mut r1 = LineReader::new(fd1);
    let mut r2 = LineReader::new(fd2);

    let mut l1_buf = [0u8; 128];
    let mut l2_buf = [0u8; 128];

    let mut opt_l1: Option<&str> = r1.read_line(&mut l1_buf);
    let mut opt_l2: Option<&str> = r2.read_line(&mut l2_buf);

    while opt_l1.is_some() && opt_l2.is_some() {
        let l1 = opt_l1.unwrap();
        let l2 = opt_l2.unwrap();

        let k1 = get_field(l1, field1, delim);
        let k2 = get_field(l2, field2, delim);

        if k1 == k2 {
            print_joined(k1, l1, field1, l2, field2, delim);
            opt_l1 = r1.read_line(&mut l1_buf);
            opt_l2 = r2.read_line(&mut l2_buf);
        } else if k1 < k2 {
            if unpairable1 {
                println!("{}", l1);
            }
            opt_l1 = r1.read_line(&mut l1_buf);
        } else {
            if unpairable2 {
                println!("{}", l2);
            }
            opt_l2 = r2.read_line(&mut l2_buf);
        }
    }

    if unpairable1 {
        while let Some(l1) = opt_l1 {
            println!("{}", l1);
            opt_l1 = r1.read_line(&mut l1_buf);
        }
    }
    if unpairable2 {
        while let Some(l2) = opt_l2 {
            println!("{}", l2);
            opt_l2 = r2.read_line(&mut l2_buf);
        }
    }

    if fd1 > 0 {
        close(fd1);
    }
    if fd2 > 0 {
        close(fd2);
    }

    0
}
