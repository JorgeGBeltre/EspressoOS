#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read, write, O_CREATE, O_RDWR, O_TRUNC, O_WRONLY};

fn parse_size(s: &str) -> Option<usize> {
    if s.is_empty() {
        return None;
    }
    let bytes = s.as_bytes();
    let last = bytes[bytes.len() - 1];
    let (num_str, mult) = match last {
        b'k' | b'K' => (&s[..s.len() - 1], 1024usize),
        b'm' | b'M' => (&s[..s.len() - 1], 1024 * 1024usize),
        b'0'..=b'9' => (s, 1usize),
        _ => return None,
    };
    let val = num_str.parse::<usize>().ok()?;
    val.checked_mul(mult)
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        println!("Usage: truncate -s SIZE FILE...");
        println!("Shrink or extend the size of each FILE to the specified size");
        println!("  -s, --size SIZE    Target size in bytes (accepts k, M suffixes)");
        println!("  -c, --no-create    Do not create any files");
        return 1;
    }

    let mut target_size: Option<usize> = None;
    let mut no_create = false;
    let mut arg_idx = 1;

    while arg_idx < argc {
        let s = unsafe { arg(argv, arg_idx) };
        if s == "-h" || s == "--help" {
            println!("Usage: truncate -s SIZE FILE...");
            println!("Shrink or extend the size of each FILE to the specified size");
            println!("  -s, --size SIZE    Target size in bytes (accepts k, M suffixes)");
            println!("  -c, --no-create    Do not create any files");
            return 0;
        } else if s == "-c" || s == "--no-create" {
            no_create = true;
            arg_idx += 1;
        } else if s == "-s" || s == "--size" {
            if arg_idx + 1 < argc {
                let size_str = unsafe { arg(argv, arg_idx + 1) };
                target_size = parse_size(size_str);
                if target_size.is_none() {
                    print!("truncate: invalid size: ");
                    println!("{}", size_str);
                    return 1;
                }
                arg_idx += 2;
            } else {
                println!("truncate: option '-s' requires an argument");
                return 1;
            }
        } else if s.starts_with("-s") {
            target_size = parse_size(&s[2..]);
            if target_size.is_none() {
                print!("truncate: invalid size: ");
                println!("{}", &s[2..]);
                return 1;
            }
            arg_idx += 1;
        } else {
            break;
        }
    }

    let target_len = match target_size {
        Some(sz) => sz,
        None => {
            println!("truncate: must specify a size with '-s'");
            return 1;
        }
    };

    if arg_idx >= argc {
        println!("truncate: missing operand");
        return 1;
    }

    let zeros = [0u8; 128];
    let mut all_ok = true;

    for i in arg_idx..argc {
        let file = unsafe { arg(argv, i) };
        if target_len == 0 {
            let flags = if no_create {
                O_WRONLY | O_TRUNC
            } else {
                O_WRONLY | O_CREATE | O_TRUNC
            };
            let fd = open(file, flags);
            if fd < 0 {
                print!("truncate: cannot truncate '");
                print!("{}", file);
                println!("'");
                all_ok = false;
            } else {
                close(fd as i32);
            }
        } else {
            // Target length > 0
            let flags = if no_create {
                O_RDWR
            } else {
                O_RDWR | O_CREATE
            };
            let fd = open(file, flags);
            if fd < 0 {
                print!("truncate: cannot open '");
                print!("{}", file);
                println!("'");
                all_ok = false;
                continue;
            }

            // Determine existing size by reading
            let mut cur_size = 0usize;
            let mut check_buf = [0u8; 256];
            loop {
                let n = read(fd as i32, &mut check_buf);
                if n <= 0 {
                    break;
                }
                cur_size += n as usize;
            }

            if cur_size < target_len {
                let mut to_write = target_len - cur_size;
                while to_write > 0 {
                    let chunk = core::cmp::min(to_write, zeros.len());
                    let written = write(fd as i32, &zeros[..chunk]);
                    if written <= 0 {
                        break;
                    }
                    to_write -= written as usize;
                }
            }
            close(fd as i32);
        }
    }

    if all_ok { 0 } else { 1 }
}
