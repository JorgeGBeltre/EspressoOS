#![no_std]
#![no_main]

use libc::{arg, print, println};

fn is_portable_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'.' || c == b'_' || c == b'-'
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        println!("Usage: pathchk [-p] [-P] PATH...");
        println!("Check whether file names are valid or portable");
        println!("  -p  Check for most POSIX systems (max path 256, max name 14, portable chars)");
        println!("  -P  Check for empty names and leading '-'");
        return 1;
    }

    let mut posix_portable = false;
    let mut check_empty_leading = false;
    let mut arg_idx = 1;

    while arg_idx < argc {
        let s = unsafe { arg(argv, arg_idx) };
        if s == "-h" || s == "--help" {
            println!("Usage: pathchk [-p] [-P] PATH...");
            println!("Check whether file names are valid or portable");
            println!("  -p  Check for most POSIX systems");
            println!("  -P  Check for empty names and leading '-'");
            return 0;
        } else if s == "-p" {
            posix_portable = true;
            arg_idx += 1;
        } else if s == "-P" {
            check_empty_leading = true;
            arg_idx += 1;
        } else if s == "-pP" || s == "-Pp" {
            posix_portable = true;
            check_empty_leading = true;
            arg_idx += 1;
        } else {
            break;
        }
    }

    if arg_idx >= argc {
        println!("pathchk: missing operand");
        return 1;
    }

    let max_path_len = 256usize;
    let max_comp_len = if posix_portable { 14usize } else { 64usize };

    let mut all_ok = true;

    for i in arg_idx..argc {
        let path = unsafe { arg(argv, i) };
        if path.is_empty() {
            println!("pathchk: '': empty file name");
            all_ok = false;
            continue;
        }

        if path.len() > max_path_len {
            print!("pathchk: '");
            print!("{}", path);
            println!("': path length exceeds limit");
            all_ok = false;
            continue;
        }

        let mut comp_start = 0usize;
        let bytes = path.as_bytes();
        let mut idx = 0usize;

        while idx <= bytes.len() {
            if idx == bytes.len() || bytes[idx] == b'/' {
                let comp_len = idx - comp_start;
                if comp_len > 0 {
                    let comp = &bytes[comp_start..idx];
                    if comp_len > max_comp_len {
                        print!("pathchk: '");
                        print!("{}", path);
                        println!("': component length exceeds limit");
                        all_ok = false;
                    }
                    if check_empty_leading && comp[0] == b'-' {
                        print!("pathchk: '");
                        print!("{}", path);
                        println!("': leading hyphen in component");
                        all_ok = false;
                    }
                    if posix_portable {
                        for &b in comp {
                            if !is_portable_char(b) {
                                print!("pathchk: '");
                                print!("{}", path);
                                println!("': nonportable character in component");
                                all_ok = false;
                                break;
                            }
                        }
                    }
                } else if check_empty_leading && idx > 0 && idx < bytes.len() {
                    print!("pathchk: '");
                    print!("{}", path);
                    println!("': empty component");
                    all_ok = false;
                }
                comp_start = idx + 1;
            }
            idx += 1;
        }
    }

    if all_ok { 0 } else { 1 }
}
