#![no_std]
#![no_main]

use libc::{
    arg, close, open, print, println, read, write, O_CREATE, O_RDONLY, O_TRUNC, O_WRONLY,
};

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

fn format_filename(buf: &mut [u8], prefix: &str, num: usize, digits: usize) -> usize {
    let p_bytes = prefix.as_bytes();
    let mut idx = 0;
    let take = core::cmp::min(p_bytes.len(), buf.len());
    buf[..take].copy_from_slice(&p_bytes[..take]);
    idx += take;

    let mut num_buf = [b'0'; 8];
    let mut d = 0;
    let mut temp = num;
    if temp == 0 {
        num_buf[0] = b'0';
        d = 1;
    } else {
        while temp > 0 && d < 8 {
            num_buf[d] = b'0' + (temp % 10) as u8;
            temp /= 10;
            d += 1;
        }
    }
    while d < digits && d < 8 {
        num_buf[d] = b'0';
        d += 1;
    }
    while d > 0 && idx < buf.len() - 1 {
        d -= 1;
        buf[idx] = num_buf[d];
        idx += 1;
    }
    buf[idx] = 0; // NUL terminator
    idx
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 3 {
        println!("Usage: csplit [-f prefix] [-n digits] FILE ARGS...");
        println!("Split FILE into context pieces by line numbers or patterns");
        println!("  -f PREFIX  Use PREFIX instead of 'xx'");
        println!("  -n DIGITS  Use specified number of digits (default: 2)");
        return 1;
    }

    let mut prefix = "xx";
    let mut digits = 2usize;
    let mut arg_idx = 1;

    while arg_idx < argc {
        let s = unsafe { arg(argv, arg_idx) };
        if s == "-h" || s == "--help" {
            println!("Usage: csplit [-f prefix] [-n digits] FILE ARGS...");
            println!("Split FILE into context pieces by line numbers or patterns");
            return 0;
        } else if s == "-f" && arg_idx + 1 < argc {
            prefix = unsafe { arg(argv, arg_idx + 1) };
            arg_idx += 2;
        } else if s == "-n" && arg_idx + 1 < argc {
            if let Some(d) = parse_u32(unsafe { arg(argv, arg_idx + 1) }) {
                digits = core::cmp::min(core::cmp::max(d, 1), 6);
            }
            arg_idx += 2;
        } else {
            break;
        }
    }

    if arg_idx >= argc {
        println!("csplit: missing file operand");
        return 1;
    }

    let filepath = unsafe { arg(argv, arg_idx) };
    arg_idx += 1;

    let in_fd = open(filepath, O_RDONLY);
    if in_fd < 0 {
        print!("csplit: cannot open ");
        println!("{}", filepath);
        return 1;
    }

    // Split targets: collect up to 8 split points (either line number or pattern)
    let spec = if arg_idx < argc {
        unsafe { arg(argv, arg_idx) }
    } else {
        ""
    };

    let target_line = if !spec.is_empty() && !spec.starts_with('/') {
        parse_u32(spec)
    } else {
        None
    };

    let target_pattern = if spec.starts_with('/') && spec.ends_with('/') && spec.len() >= 2 {
        &spec[1..spec.len() - 1]
    } else {
        ""
    };

    let mut part_num = 0usize;
    let mut name_buf = [0u8; 32];
    let name_len = format_filename(&mut name_buf, prefix, part_num, digits);
    let name_str = core::str::from_utf8(&name_buf[..name_len]).unwrap_or("xx00");

    let mut out_fd = open(name_str, O_WRONLY | O_CREATE | O_TRUNC);
    if out_fd < 0 {
        println!("csplit: cannot create output file");
        close(in_fd as i32);
        return 1;
    }

    let mut read_buf = [0u8; 512];
    let mut line_buf = [0u8; 256];
    let mut line_len = 0usize;
    let mut cur_line_num = 1usize;
    let mut bytes_written_cur = 0usize;
    let mut split_occurred = false;

    loop {
        let n = read(in_fd as i32, &mut read_buf);
        if n <= 0 {
            break;
        }
        for &b in &read_buf[..n as usize] {
            if line_len < line_buf.len() {
                line_buf[line_len] = b;
                line_len += 1;
            }
            if b == b'\n' {
                let is_split_point = if !split_occurred {
                    if let Some(t_line) = target_line {
                        cur_line_num == t_line
                    } else if !target_pattern.is_empty() {
                        let l_str = core::str::from_utf8(&line_buf[..line_len]).unwrap_or("");
                        l_str.contains(target_pattern)
                    } else {
                        false
                    }
                } else {
                    false
                };

                if is_split_point && cur_line_num > 1 {
                    // Close current piece
                    close(out_fd as i32);
                    print_dec(bytes_written_cur);
                    println!("");

                    // Open next piece
                    part_num += 1;
                    bytes_written_cur = 0;
                    let nl = format_filename(&mut name_buf, prefix, part_num, digits);
                    let ns = core::str::from_utf8(&name_buf[..nl]).unwrap_or("xx01");
                    out_fd = open(ns, O_WRONLY | O_CREATE | O_TRUNC);
                    if out_fd < 0 {
                        close(in_fd as i32);
                        return 1;
                    }
                    split_occurred = true;
                }

                if out_fd >= 0 {
                    let w = write(out_fd as i32, &line_buf[..line_len]);
                    if w > 0 {
                        bytes_written_cur += w as usize;
                    }
                }

                line_len = 0;
                cur_line_num += 1;
            }
        }
    }

    if line_len > 0 && out_fd >= 0 {
        let w = write(out_fd as i32, &line_buf[..line_len]);
        if w > 0 {
            bytes_written_cur += w as usize;
        }
    }

    if out_fd >= 0 {
        close(out_fd as i32);
        print_dec(bytes_written_cur);
        println!("");
    }

    close(in_fd as i32);
    0
}
