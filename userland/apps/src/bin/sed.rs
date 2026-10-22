#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read, write, O_RDONLY};

fn parse_subst(expr: &str) -> Option<(&str, &str, bool)> {
    if !expr.starts_with('s') || expr.len() < 4 {
        return None;
    }
    let bytes = expr.as_bytes();
    let delim = bytes[1];
    let rest = &expr[2..];

    // Find next delimiter
    let mut p1 = None;
    for (i, &b) in rest.as_bytes().iter().enumerate() {
        if b == delim {
            p1 = Some(i);
            break;
        }
    }
    let p1 = p1?;
    let find = &rest[..p1];
    let after_find = &rest[p1 + 1..];

    // Find final delimiter
    let mut p2 = None;
    for (i, &b) in after_find.as_bytes().iter().enumerate() {
        if b == delim {
            p2 = Some(i);
            break;
        }
    }
    let p2 = p2?;
    let replace = &after_find[..p2];
    let flags = &after_find[p2 + 1..];
    let global = flags.contains('g');

    Some((find, replace, global))
}

fn apply_line(line: &str, find: &str, replace: &str, global: bool) {
    if find.is_empty() {
        println!("{}", line);
        return;
    }

    let mut cursor = 0;
    let line_bytes = line.as_bytes();
    let find_bytes = find.as_bytes();

    while cursor <= line_bytes.len().saturating_sub(find_bytes.len()) {
        if &line_bytes[cursor..cursor + find_bytes.len()] == find_bytes {
            print!("{}", replace);
            cursor += find_bytes.len();
            if !global {
                if let Ok(rem) = core::str::from_utf8(&line_bytes[cursor..]) {
                    print!("{}", rem);
                }
                println!("");
                return;
            }
        } else {
            print!("{}", line_bytes[cursor] as char);
            cursor += 1;
        }
    }
    if cursor < line_bytes.len() {
        if let Ok(rem) = core::str::from_utf8(&line_bytes[cursor..]) {
            print!("{}", rem);
        }
    }
    println!("");
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        println!("usage: sed 's/pattern/replacement/[g]' [file]");
        return 1;
    }

    let expr = unsafe { arg(argv, 1) };
    if expr == "-h" || expr == "--help" {
        println!("usage: sed 's/pattern/replacement/[g]' [file]");
        println!("Stream editor for filtering and transforming text");
        return 0;
    }

    let (find, replace, global) = match parse_subst(expr) {
        Some(tuple) => tuple,
        None => {
            println!("sed: unsupported expression format '{}'", expr);
            return 1;
        }
    };

    let fd = if argc > 2 {
        let path = unsafe { arg(argv, 2) };
        let f = open(path, O_RDONLY);
        if f < 0 {
            println!("sed: cannot open '{}'", path);
            return 1;
        }
        f as i32
    } else {
        0 // stdin
    };

    let mut buf = [0u8; 512];
    let mut line_buf = [0u8; 512];
    let mut line_len = 0;

    loop {
        let n = read(fd, &mut buf);
        if n <= 0 {
            break;
        }
        for &b in &buf[..n as usize] {
            if b == b'\n' {
                if let Ok(s) = core::str::from_utf8(&line_buf[..line_len]) {
                    apply_line(s, find, replace, global);
                }
                line_len = 0;
            } else if line_len < line_buf.len() {
                line_buf[line_len] = b;
                line_len += 1;
            }
        }
    }

    if line_len > 0 {
        if let Ok(s) = core::str::from_utf8(&line_buf[..line_len]) {
            apply_line(s, find, replace, global);
        }
    }

    if fd != 0 {
        close(fd);
    }

    0
}
