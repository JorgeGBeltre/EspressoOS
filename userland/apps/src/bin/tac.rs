#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read, O_RDONLY};

const MAX_BUF: usize = 8192;
const MAX_LINES: usize = 128;

fn process_fd(fd: i32) {
    let mut buf = [0u8; MAX_BUF];
    let mut total = 0;

    loop {
        if total >= MAX_BUF {
            break;
        }
        let n = read(fd, &mut buf[total..]);
        if n <= 0 {
            break;
        }
        total += n as usize;
    }

    if total == 0 {
        return;
    }

    let mut line_starts = [0usize; MAX_LINES];
    let mut line_ends = [0usize; MAX_LINES];
    let mut num_lines = 0;

    let mut cur_start = 0;
    for i in 0..total {
        if buf[i] == b'\n' {
            if num_lines < MAX_LINES {
                line_starts[num_lines] = cur_start;
                line_ends[num_lines] = i + 1;
                num_lines += 1;
            }
            cur_start = i + 1;
        }
    }

    if cur_start < total && num_lines < MAX_LINES {
        line_starts[num_lines] = cur_start;
        line_ends[num_lines] = total;
        num_lines += 1;
    }

    for i in (0..num_lines).rev() {
        let st = line_starts[i];
        let en = line_ends[i];
        let line = &buf[st..en];

        let s = unsafe { core::str::from_utf8_unchecked(line) };
        print!("{}", s);
        if !line.ends_with(b"\n") {
            println!();
        }
    }
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut files = [core::ptr::null::<u8>(); 16];
    let mut num_files = 0;

    for i in 1..argc {
        let arg_str = unsafe { arg(argv, i) };
        if arg_str == "-h" || arg_str == "--help" {
            println!("Usage: tac [FILE]...");
            println!("Concatenate and print files in reverse (last line first)");
            return 0;
        } else if !arg_str.starts_with('-') || arg_str == "-" {
            if num_files < files.len() {
                unsafe {
                    files[num_files] = *argv.offset(i as isize);
                    num_files += 1;
                }
            }
        }
    }

    if num_files == 0 {
        process_fd(0);
        return 0;
    }

    let mut exit_code = 0;
    for i in 0..num_files {
        let path_ptr = files[i];
        let path_str = unsafe {
            let mut len = 0;
            while *path_ptr.add(len) != 0 {
                len += 1;
            }
            core::str::from_utf8_unchecked(core::slice::from_raw_parts(path_ptr, len))
        };

        if path_str == "-" {
            process_fd(0);
        } else {
            let fd = open(path_str, O_RDONLY) as i32;
            if fd < 0 {
                print!("tac: failed to open '");
                print!("{}", path_str);
                println!("': No such file or directory");
                exit_code = 1;
                continue;
            }
            process_fd(fd);
            close(fd);
        }
    }

    exit_code
}
