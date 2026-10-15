#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read, O_RDONLY};

fn starts_with_ignore_case(s: &str, prefix: &str) -> bool {
    if s.len() < prefix.len() {
        return false;
    }
    let sb = s.as_bytes();
    let pb = prefix.as_bytes();
    for i in 0..prefix.len() {
        let sc = sb[i].to_ascii_lowercase();
        let pc = pb[i].to_ascii_lowercase();
        if sc != pc {
            return false;
        }
    }
    true
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        println!("Usage: look [-f] string [file]");
        println!("Display lines beginning with a given string");
        println!("  -f, --ignore-case  Ignore case when comparing");
        println!("  -h, --help         Display this help");
        return 1;
    }

    let mut ignore_case = false;
    let mut string_target: Option<&str> = None;
    let mut file_target: Option<&str> = None;

    let mut i = 1;
    while i < argc {
        let s = unsafe { arg(argv, i) };
        if s == "-h" || s == "--help" {
            println!("Usage: look [-f] string [file]");
            println!("Display lines beginning with a given string");
            println!("  -f, --ignore-case  Ignore case when comparing");
            return 0;
        } else if s == "-f" || s == "--ignore-case" {
            ignore_case = true;
            i += 1;
        } else if string_target.is_none() {
            string_target = Some(s);
            i += 1;
        } else if file_target.is_none() {
            file_target = Some(s);
            i += 1;
        } else {
            i += 1;
        }
    }

    let prefix = match string_target {
        Some(p) => p,
        None => {
            println!("look: missing search string");
            return 1;
        }
    };

    let filepath = file_target.unwrap_or("/etc/passwd");
    let fd = open(filepath, O_RDONLY);
    if fd < 0 {
        print!("look: cannot open ");
        println!("{}", filepath);
        return 2;
    }
    let fd = fd as i32;

    let mut buf = [0u8; 512];
    let mut line_buf = [0u8; 128];
    let mut line_len = 0usize;
    let mut matched = false;

    loop {
        let n = read(fd, &mut buf);
        if n <= 0 {
            break;
        }
        for &b in &buf[..n as usize] {
            if b == b'\n' {
                if line_len > 0 {
                    if let Ok(line) = core::str::from_utf8(&line_buf[..line_len]) {
                        let ok = if ignore_case {
                            starts_with_ignore_case(line, prefix)
                        } else {
                            line.starts_with(prefix)
                        };
                        if ok {
                            println!("{}", line);
                            matched = true;
                        }
                    }
                    line_len = 0;
                }
            } else if line_len < line_buf.len() {
                line_buf[line_len] = b;
                line_len += 1;
            }
        }
    }

    if line_len > 0 {
        if let Ok(line) = core::str::from_utf8(&line_buf[..line_len]) {
            let ok = if ignore_case {
                starts_with_ignore_case(line, prefix)
            } else {
                line.starts_with(prefix)
            };
            if ok {
                println!("{}", line);
                matched = true;
            }
        }
    }

    close(fd);
    if matched {
        0
    } else {
        1
    }
}
