#![no_std]
#![no_main]

use libc::{arg, println, read, write};

fn is_in_set(b: u8, set: &str) -> bool {
    let bytes = set.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 2 < bytes.len() && bytes[i + 1] == b'-' {
            let start = bytes[i];
            let end = bytes[i + 2];
            if b >= start && b <= end {
                return true;
            }
            i += 3;
        } else {
            let ch = if bytes[i] == b'\\' && i + 1 < bytes.len() {
                i += 1;
                match bytes[i] {
                    b'r' => b'\r',
                    b'n' => b'\n',
                    b't' => b'\t',
                    b'0' => b'\0',
                    other => other,
                }
            } else {
                bytes[i]
            };
            if b == ch {
                return true;
            }
            i += 1;
        }
    }
    false
}

fn translate_byte(b: u8, set1: &str, set2: &str) -> u8 {
    if set1 == "a-z" && set2 == "A-Z" {
        if b >= b'a' && b <= b'z' {
            return b - b'a' + b'A';
        }
    } else if set1 == "A-Z" && set2 == "a-z" {
        if b >= b'A' && b <= b'Z' {
            return b - b'A' + b'a';
        }
    }
    b
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        println!("Usage: tr [-d] SET1 [SET2]");
        println!("Translate or delete characters from standard input.");
        return 1;
    }

    let first = unsafe { arg(argv, 1) };
    if first == "-h" || first == "--help" {
        println!("Usage: tr [-d] SET1 [SET2]");
        println!("Translate or delete characters from standard input.");
        println!("Examples:");
        println!("  tr -d '\\r'         Delete carriage returns (CRLF to LF)");
        println!("  tr a-z A-Z         Convert lowercase to uppercase");
        return 0;
    }

    let mut delete_mode = false;
    let mut set1 = "";
    let mut set2 = "";

    if first == "-d" {
        delete_mode = true;
        if argc >= 3 {
            set1 = unsafe { arg(argv, 2) };
        } else {
            println!("tr: missing operand after '-d'");
            return 1;
        }
    } else {
        set1 = first;
        if argc >= 3 {
            set2 = unsafe { arg(argv, 2) };
        }
    }

    let mut in_buf = [0u8; 512];
    let mut out_buf = [0u8; 512];

    loop {
        let n = read(0, &mut in_buf);
        if n <= 0 { break; }
        let mut out_pos = 0usize;

        for &b in &in_buf[..n as usize] {
            if delete_mode {
                if !is_in_set(b, set1) {
                    out_buf[out_pos] = b;
                    out_pos += 1;
                }
            } else {
                let mapped = translate_byte(b, set1, set2);
                out_buf[out_pos] = mapped;
                out_pos += 1;
            }
        }

        if out_pos > 0 {
            let _ = write(1, &out_buf[..out_pos]);
        }
    }
    0
}
