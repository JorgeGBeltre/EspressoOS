#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read};

const O_RDONLY: u32 = 1;

const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";

fn print_hex_byte(b: u8) {
    let hi = HEX_CHARS[(b >> 4) as usize] as char;
    let lo = HEX_CHARS[(b & 0x0f) as usize] as char;
    print!("{}{}", hi, lo);
}

fn print_offset(off: usize) {
    let mut buf = [b'0'; 8];
    let mut v = off;
    let mut i = 7;
    loop {
        buf[i] = HEX_CHARS[v & 0x0f];
        v >>= 4;
        if i == 0 { break; }
        i -= 1;
    }
    let s = core::str::from_utf8(&buf).unwrap_or("00000000");
    print!("{}  ", s);
}

fn dump_fd(fd: i32, max_bytes: Option<usize>) {
    let mut row = [0u8; 16];
    let mut row_len = 0usize;
    let mut total_offset = 0usize;

    let mut buf = [0u8; 256];
    loop {
        let n = read(fd, &mut buf);
        if n <= 0 { break; }
        for &b in &buf[..n as usize] {
            row[row_len] = b;
            row_len += 1;

            if row_len == 16 {
                print_offset(total_offset);
                for i in 0..16 {
                    print_hex_byte(row[i]);
                    if i == 7 {
                        print!("  ");
                    } else {
                        print!(" ");
                    }
                }
                print!(" |");
                for i in 0..16 {
                    let ch = row[i];
                    if ch >= 32 && ch <= 126 {
                        print!("{}", ch as char);
                    } else {
                        print!(".");
                    }
                }
                println!("|");
                total_offset += 16;
                row_len = 0;

                if let Some(max) = max_bytes {
                    if total_offset >= max {
                        return;
                    }
                }
            }
        }
    }

    if row_len > 0 {
        print_offset(total_offset);
        for i in 0..16 {
            if i < row_len {
                print_hex_byte(row[i]);
            } else {
                print!("  ");
            }
            if i == 7 {
                print!("  ");
            } else {
                print!(" ");
            }
        }
        print!(" |");
        for i in 0..row_len {
            let ch = row[i];
            if ch >= 32 && ch <= 126 {
                print!("{}", ch as char);
            } else {
                print!(".");
            }
        }
        println!("|");
        total_offset += row_len;
    }

    if total_offset > 0 {
        print_offset(total_offset);
        println!("");
    }
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut max_bytes: Option<usize> = None;
    let mut file_path: Option<&str> = None;

    let mut i = 1;
    while i < argc {
        let a = unsafe { arg(argv, i) };
        if a == "-n" && i + 1 < argc {
            i += 1;
            let n_str = unsafe { arg(argv, i) };
            let mut val = 0usize;
            for b in n_str.bytes() {
                if b >= b'0' && b <= b'9' {
                    val = val.saturating_mul(10).saturating_add((b - b'0') as usize);
                }
            }
            if val > 0 { max_bytes = Some(val); }
        } else if a == "-C" || a == "--canonical" {
            // Default format is canonical -C
        } else if a == "-h" || a == "--help" {
            println!("Usage: hexdump [-C] [-n LENGTH] [FILE]");
            println!("Display file or standard input contents in hexadecimal and ASCII.");
            return 0;
        } else if file_path.is_none() && !a.starts_with("-") {
            file_path = Some(a);
        }
        i += 1;
    }

    if let Some(path) = file_path {
        let fd = open(path, O_RDONLY);
        if fd < 0 {
            println!("hexdump: cannot open {}", path);
            return 1;
        }
        dump_fd(fd as i32, max_bytes);
        close(fd as i32);
    } else {
        dump_fd(0, max_bytes);
    }
    0
}
