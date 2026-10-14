#![no_std]
#![no_main]

use libc::{
    arg, close, open, print, println, read, unlink, write, O_CREATE, O_RDONLY, O_TRUNC, O_WRONLY,
};

const STDOUT: i32 = 1;

fn convert_stream(in_fd: i32, out_fd: i32) -> bool {
    let mut in_buf = [0u8; 512];
    let mut out_buf = [0u8; 512];
    let mut out_len = 0usize;
    let mut prev_cr = false;

    loop {
        let n = read(in_fd, &mut in_buf);
        if n <= 0 {
            break;
        }
        for &b in &in_buf[..n as usize] {
            if b == b'\n' {
                if !prev_cr {
                    if out_len < out_buf.len() {
                        out_buf[out_len] = b'\r';
                        out_len += 1;
                    }
                    if out_len == out_buf.len() {
                        let _ = write(out_fd, &out_buf);
                        out_len = 0;
                    }
                }
                if out_len < out_buf.len() {
                    out_buf[out_len] = b'\n';
                    out_len += 1;
                }
                prev_cr = false;
            } else {
                if b == b'\r' {
                    prev_cr = true;
                } else {
                    prev_cr = false;
                }
                if out_len < out_buf.len() {
                    out_buf[out_len] = b;
                    out_len += 1;
                }
            }

            if out_len >= out_buf.len() {
                let _ = write(out_fd, &out_buf);
                out_len = 0;
            }
        }
    }

    if out_len > 0 {
        let _ = write(out_fd, &out_buf[..out_len]);
    }

    true
}

fn convert_file(path: &str, quiet: bool) -> bool {
    let in_fd = open(path, O_RDONLY);
    if in_fd < 0 {
        if !quiet {
            print!("unix2dos: cannot open ");
            println!("{}", path);
        }
        return false;
    }

    let tmp_path = "/tmp/u2d.tmp";
    let tmp_fd = open(tmp_path, O_WRONLY | O_CREATE | O_TRUNC);
    if tmp_fd < 0 {
        close(in_fd as i32);
        if !quiet {
            println!("unix2dos: cannot create temporary file");
        }
        return false;
    }

    convert_stream(in_fd as i32, tmp_fd as i32);
    close(in_fd as i32);
    close(tmp_fd as i32);

    // Copy temp file back to original file
    let copy_in = open(tmp_path, O_RDONLY);
    if copy_in < 0 {
        return false;
    }
    let copy_out = open(path, O_WRONLY | O_TRUNC);
    if copy_out < 0 {
        close(copy_in as i32);
        return false;
    }

    let mut buf = [0u8; 512];
    loop {
        let n = read(copy_in as i32, &mut buf);
        if n <= 0 {
            break;
        }
        let _ = write(copy_out as i32, &buf[..n as usize]);
    }

    close(copy_in as i32);
    close(copy_out as i32);
    let _ = unlink(tmp_path);

    if !quiet {
        print!("unix2dos: converted ");
        println!("{}", path);
    }
    true
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut quiet = false;
    let mut start_idx = 1;

    for i in 1..argc {
        let s = unsafe { arg(argv, i) };
        if s == "-h" || s == "--help" {
            println!("Usage: unix2dos [-q] [FILE...]");
            println!("Convert Unix line breaks (LF) to DOS/Windows line breaks (CRLF)");
            println!("  -q, --quiet  Quiet mode, suppress all warnings and messages");
            return 0;
        } else if s == "-q" || s == "--quiet" {
            quiet = true;
            start_idx = i + 1;
        } else {
            start_idx = i;
            break;
        }
    }

    if start_idx >= argc || (start_idx == argc - 1 && unsafe { arg(argv, start_idx) } == "-") {
        convert_stream(0, STDOUT);
        return 0;
    }

    let mut all_ok = true;
    for i in start_idx..argc {
        let path = unsafe { arg(argv, i) };
        if path == "-" {
            convert_stream(0, STDOUT);
        } else if !convert_file(path, quiet) {
            all_ok = false;
        }
    }

    if all_ok {
        0
    } else {
        1
    }
}
