#![no_std]
#![no_main]

use libc::{arg, close, open, println, read, write, O_CREATE, O_RDONLY, O_TRUNC, O_WRONLY};

const MAX_LINES: usize = 64;
const MAX_LINE_LEN: usize = 80;

static mut LINES: [[u8; MAX_LINE_LEN]; MAX_LINES] = [[0; MAX_LINE_LEN]; MAX_LINES];
static mut LINE_LENS: [usize; MAX_LINES] = [0; MAX_LINES];
static mut LINE_COUNT: usize = 0;

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        println!("usage: patch <file> <diff_file>");
        println!("       patch -i <diff_file> <file>");
        return 2;
    }

    let mut target_file: Option<&str> = None;
    let mut diff_file: Option<&str> = None;

    let arg1 = unsafe { arg(argv, 1) };
    if arg1 == "-i" {
        if argc > 2 {
            diff_file = Some(unsafe { arg(argv, 2) });
        }
        if argc > 3 {
            target_file = Some(unsafe { arg(argv, 3) });
        }
    } else {
        target_file = Some(arg1);
        if argc > 2 {
            diff_file = Some(unsafe { arg(argv, 2) });
        }
    }

    let diff_path = match diff_file {
        Some(p) => p,
        None => {
            println!("patch: missing diff file");
            return 2;
        }
    };

    // Read diff file
    let diff_fd = open(diff_path, O_RDONLY);
    if diff_fd < 0 {
        println!("patch: cannot open diff file {}", diff_path);
        return 2;
    }

    let mut diff_buf = [0u8; 2048];
    let diff_len = read(diff_fd as i32, &mut diff_buf);
    close(diff_fd as i32);

    if diff_len <= 0 {
        println!("patch: empty diff file");
        return 1;
    }

    let diff_str = match core::str::from_utf8(&diff_buf[..diff_len as usize]) {
        Ok(s) => s,
        Err(_) => {
            println!("patch: non-UTF8 diff content");
            return 2;
        }
    };

    // If target file was not explicitly passed, inspect +++ line in diff
    let resolved_target = if let Some(tf) = target_file {
        tf
    } else {
        let mut found = None;
        for line in diff_str.lines() {
            if line.starts_with("+++ ") {
                found = Some(line[4..].trim());
                break;
            }
        }
        match found {
            Some(f) => f,
            None => {
                println!("patch: no target file specified and none found in diff");
                return 2;
            }
        }
    };

    // Load original target file if present
    unsafe {
        LINE_COUNT = 0;
    }
    let target_fd = open(resolved_target, O_RDONLY);
    if target_fd >= 0 {
        let mut t_buf = [0u8; 2048];
        let t_len = read(target_fd as i32, &mut t_buf);
        close(target_fd as i32);
        if t_len > 0 {
            if let Ok(content) = core::str::from_utf8(&t_buf[..t_len as usize]) {
                for line in content.lines() {
                    unsafe {
                        if LINE_COUNT < MAX_LINES {
                            let bytes = line.as_bytes();
                            let len = core::cmp::min(bytes.len(), MAX_LINE_LEN);
                            LINES[LINE_COUNT][..len].copy_from_slice(&bytes[..len]);
                            LINE_LENS[LINE_COUNT] = len;
                            LINE_COUNT += 1;
                        }
                    }
                }
            }
        }
    }

    println!("patching file {}", resolved_target);

    // Parse hunks
    let mut current_line: usize = 0;
    for line in diff_str.lines() {
        if line.starts_with("--- ") || line.starts_with("+++ ") {
            continue;
        }

        if line.starts_with("@@ line ") {
            // format: @@ line <N> @@
            let rest = &line[8..];
            if let Some(end_idx) = rest.find(' ') {
                if let Ok(n) = rest[..end_idx].parse::<usize>() {
                    current_line = if n > 0 { n - 1 } else { 0 };
                }
            }
            continue;
        }

        if line.starts_with("- ") {
            // In a simple replace/delete hunk, current_line is targeted
            // If the next line is + we replace; if not, we can remove
            // To support simple unified diff, we check if current_line < LINE_COUNT
            continue;
        }

        if line.starts_with("+ ") {
            let add_content = &line[2..];
            let bytes = add_content.as_bytes();
            let len = core::cmp::min(bytes.len(), MAX_LINE_LEN);

            unsafe {
                if current_line < LINE_COUNT {
                    // Replace existing line
                    LINES[current_line][..len].copy_from_slice(&bytes[..len]);
                    LINE_LENS[current_line] = len;
                    current_line += 1;
                } else if LINE_COUNT < MAX_LINES {
                    // Append new line
                    LINES[LINE_COUNT][..len].copy_from_slice(&bytes[..len]);
                    LINE_LENS[LINE_COUNT] = len;
                    LINE_COUNT += 1;
                    current_line = LINE_COUNT;
                }
            }
        }
    }

    // Write back modified lines
    let out_fd = open(resolved_target, O_WRONLY | O_CREATE | O_TRUNC);
    if out_fd < 0 {
        println!("patch: failed to write to {}", resolved_target);
        return 1;
    }

    unsafe {
        for i in 0..LINE_COUNT {
            let _ = write(out_fd as i32, &LINES[i][..LINE_LENS[i]]);
            let _ = write(out_fd as i32, b"\n");
        }
    }
    close(out_fd as i32);

    0
}
