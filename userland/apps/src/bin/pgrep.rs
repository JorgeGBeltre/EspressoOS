#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read, readdir, O_RDONLY};

fn parse_pid(s: &str) -> Option<u32> {
    if s.is_empty() {
        return None;
    }
    let mut val = 0u32;
    for &b in s.as_bytes() {
        if b < b'0' || b > b'9' {
            return None;
        }
        val = val.saturating_mul(10).saturating_add((b - b'0') as u32);
    }
    Some(val)
}

fn get_process_cmd(pid_str: &str, out_buf: &mut [u8]) -> usize {
    let mut path = [0u8; 40];
    let p_pre = b"/proc/";
    let p_post = b"/cmdline";
    path[..p_pre.len()].copy_from_slice(p_pre);
    let mut idx = p_pre.len();
    for &b in pid_str.as_bytes() {
        if idx < path.len() - p_post.len() {
            path[idx] = b;
            idx += 1;
        }
    }
    path[idx..idx + p_post.len()].copy_from_slice(p_post);
    idx += p_post.len();

    let path_str = unsafe { core::str::from_utf8_unchecked(&path[..idx]) };
    let fd = open(path_str, O_RDONLY) as i32;
    if fd < 0 {
        return 0;
    }
    let n = read(fd, out_buf);
    close(fd);
    if n <= 0 {
        return 0;
    }
    let mut len = n as usize;
    while len > 0 && (out_buf[len - 1] == 0 || out_buf[len - 1] == b'\n' || out_buf[len - 1] == b' ') {
        len -= 1;
    }
    len
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut list_name = false;
    let mut exact = false;
    let mut pattern: Option<&str> = None;

    for i in 1..argc {
        let arg_str = unsafe { arg(argv, i) };
        if arg_str == "-l" {
            list_name = true;
        } else if arg_str == "-x" {
            exact = true;
        } else if arg_str == "-h" || arg_str == "--help" {
            println!("Usage: pgrep [-l] [-x] PATTERN");
            println!("Look up process IDs based on name");
            println!("  -l    list the process name as well as the process ID");
            println!("  -x    match only processes whose names exactly match the pattern");
            return 0;
        } else if !arg_str.starts_with('-') && pattern.is_none() {
            pattern = Some(arg_str);
        }
    }

    let pat = match pattern {
        Some(p) => p,
        None => {
            println!("pgrep: no pattern specified");
            println!("Usage: pgrep [-l] [-x] PATTERN");
            return 2;
        }
    };

    let mut buf = [0u8; 1024];
    let n = readdir("/proc", &mut buf);
    if n < 0 {
        println!("pgrep: cannot read /proc");
        return 2;
    }

    let limit = n as usize;
    let mut pos = 0;
    let mut found_count = 0;

    while pos + 11 <= limit {
        let kind = buf[pos + 8];
        let name_len = u16::from_le_bytes([buf[pos + 9], buf[pos + 10]]) as usize;
        pos += 11;

        if pos + name_len > limit {
            break;
        }

        if let Ok(entry_name) = core::str::from_utf8(&buf[pos..pos + name_len]) {
            if kind == 2 {
                if let Some(_pid) = parse_pid(entry_name) {
                    let mut cmd_buf = [0u8; 128];
                    let cmd_len = get_process_cmd(entry_name, &mut cmd_buf);
                    let cmd_str = if cmd_len > 0 {
                        unsafe { core::str::from_utf8_unchecked(&cmd_buf[..cmd_len]) }
                    } else {
                        entry_name
                    };

                    let base_cmd = match cmd_str.rfind('/') {
                        Some(idx) => &cmd_str[idx + 1..],
                        None => cmd_str,
                    };

                    let matches = if exact {
                        base_cmd == pat || cmd_str == pat
                    } else {
                        base_cmd.contains(pat) || cmd_str.contains(pat)
                    };

                    if matches {
                        found_count += 1;
                        print!("{}", entry_name);
                        if list_name {
                            print!(" {}", base_cmd);
                        }
                        println!();
                    }
                }
            }
        }

        pos += name_len;
    }

    if found_count > 0 {
        0
    } else {
        1
    }
}
