#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read, O_RDONLY};

fn print_dec(mut n: usize) {
    if n == 0 {
        print!("0");
        return;
    }
    let mut buf = [0u8; 16];
    let mut i = 0;
    while n > 0 {
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        i += 1;
    }
    while i > 0 {
        i -= 1;
        let s = core::slice::from_ref(&buf[i]);
        if let Ok(c) = core::str::from_utf8(s) {
            print!("{}", c);
        }
    }
}

fn print_header(title: &str, page: usize) {
    println!("");
    println!("");
    print!("2026-10-14 00:00  ");
    print!("{}", title);
    print!("  Page ");
    print_dec(page);
    println!("");
    println!("");
    println!("");
}

fn print_trailer() {
    println!("");
    println!("");
    println!("");
    println!("");
    println!("");
}

fn process_fd(fd: i32, title: &str, page_len: usize, omit_headers: bool) {
    let mut buf = [0u8; 256];
    let mut line_buf = [0u8; 128];
    let mut line_len = 0;

    let body_len = if page_len > 10 { page_len - 10 } else { page_len };
    let mut page = 1;
    let mut line_count = 0;

    if !omit_headers {
        print_header(title, page);
    }

    loop {
        let n = read(fd, &mut buf);
        if n <= 0 {
            break;
        }

        for &b in &buf[..n as usize] {
            if b == b'\n' {
                if let Ok(s) = core::str::from_utf8(&line_buf[..line_len]) {
                    println!("{}", s);
                }
                line_len = 0;
                line_count += 1;

                if !omit_headers && line_count >= body_len {
                    print_trailer();
                    page += 1;
                    line_count = 0;
                    print_header(title, page);
                }
            } else if line_len < line_buf.len() {
                line_buf[line_len] = b;
                line_len += 1;
            }
        }
    }

    if line_len > 0 {
        if let Ok(s) = core::str::from_utf8(&line_buf[..line_len]) {
            println!("{}", s);
        }
        line_count += 1;
    }

    if !omit_headers && line_count > 0 {
        let rem = body_len.saturating_sub(line_count);
        for _ in 0..rem {
            println!("");
        }
        print_trailer();
    }
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut custom_header = "";
    let mut page_len = 66usize;
    let mut omit_headers = false;
    let mut files: [&str; 8] = [""; 8];
    let mut fcount = 0;

    let mut i = 1;
    while i < argc {
        let a = unsafe { arg(argv, i) };
        if a == "--help" || a == "-h" {
            println!("Usage: pr [OPTION]... [FILE]...");
            println!("Paginate or format files for printing");
            println!("  -h HEADER   use a centered HEADER instead of filename in page header");
            println!("  -l LINES    set page length to LINES (default 66)");
            println!("  -t          omit page headers and trailers");
            return 0;
        } else if a == "-t" {
            omit_headers = true;
        } else if a == "-h" && i + 1 < argc {
            i += 1;
            custom_header = unsafe { arg(argv, i) };
        } else if a == "-l" && i + 1 < argc {
            i += 1;
            let lstr = unsafe { arg(argv, i) };
            let mut val = 0usize;
            for &b in lstr.as_bytes() {
                if b >= b'0' && b <= b'9' {
                    val = val * 10 + (b - b'0') as usize;
                }
            }
            if val > 0 {
                page_len = val;
            }
        } else if !a.starts_with('-') || a == "-" {
            if fcount < 8 {
                files[fcount] = a;
                fcount += 1;
            }
        }
        i += 1;
    }

    if fcount == 0 {
        let title = if !custom_header.is_empty() { custom_header } else { "" };
        process_fd(0, title, page_len, omit_headers);
    } else {
        for idx in 0..fcount {
            let f = files[idx];
            if f == "-" {
                let title = if !custom_header.is_empty() { custom_header } else { "" };
                process_fd(0, title, page_len, omit_headers);
            } else {
                let fd = open(f, O_RDONLY);
                if fd < 0 {
                    println!("pr: cannot open '{}': No such file or directory", f);
                    continue;
                }
                let title = if !custom_header.is_empty() { custom_header } else { f };
                process_fd(fd as i32, title, page_len, omit_headers);
                close(fd as i32);
            }
        }
    }

    0
}
