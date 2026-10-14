#![no_std]
#![no_main]

use libc::{
    arg, close, open, println, read, write, O_CREATE, O_RDONLY, O_TRUNC, O_WRONLY,
};

const MAX_BUF: usize = 2048;

fn make_suffix(idx: usize) -> (u8, u8) {
    let first = b'a' + ((idx / 26) % 26) as u8;
    let second = b'a' + (idx % 26) as u8;
    (first, second)
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut byte_split: Option<usize> = None;
    let mut line_split: Option<usize> = None;
    let mut file_opt: Option<&str> = None;
    let mut prefix = "x";

    let mut i = 1;
    while i < argc {
        let a = unsafe { arg(argv, i) };
        if a == "-b" {
            i += 1;
            if i < argc {
                if let Ok(b) = unsafe { arg(argv, i) }.parse::<usize>() {
                    byte_split = Some(b);
                }
            }
        } else if a == "-l" {
            i += 1;
            if i < argc {
                if let Ok(l) = unsafe { arg(argv, i) }.parse::<usize>() {
                    line_split = Some(l);
                }
            }
        } else if a == "-h" || a == "--help" {
            println!("usage: split [-b bytes] [-l lines] [file [prefix]]");
            println!("       Split a file into pieces");
            return 0;
        } else if !a.starts_with('-') {
            if file_opt.is_none() {
                file_opt = Some(a);
            } else {
                prefix = a;
            }
        }
        i += 1;
    }

    let input_path = match file_opt {
        Some(p) => p,
        None => {
            println!("split: missing input file");
            return 1;
        }
    };

    let fd = open(input_path, O_RDONLY);
    if fd < 0 {
        println!("split: cannot open {}", input_path);
        return 1;
    }

    let mut buf = [0u8; MAX_BUF];
    let n = read(fd as i32, &mut buf);
    close(fd as i32);

    if n <= 0 {
        return 0;
    }

    let data = &buf[..n as usize];
    let mut chunk_idx = 0;

    let chunk_size = byte_split.unwrap_or(if line_split.is_some() { 0 } else { 512 });

    if chunk_size > 0 {
        // Split by byte count
        let mut offset = 0;
        while offset < data.len() {
            let end = core::cmp::min(offset + chunk_size, data.len());
            let (s1, s2) = make_suffix(chunk_idx);

            let mut out_path = [0u8; 64];
            let p_bytes = prefix.as_bytes();
            let mut p_len = 0;
            for &b in p_bytes {
                if p_len < 60 {
                    out_path[p_len] = b;
                    p_len += 1;
                }
            }
            out_path[p_len] = s1;
            out_path[p_len + 1] = s2;
            p_len += 2;

            if let Ok(out_name) = core::str::from_utf8(&out_path[..p_len]) {
                let out_fd = open(out_name, O_WRONLY | O_CREATE | O_TRUNC);
                if out_fd >= 0 {
                    let _ = write(out_fd as i32, &data[offset..end]);
                    close(out_fd as i32);
                }
            }

            offset = end;
            chunk_idx += 1;
        }
    } else {
        // Split by lines
        let lines_per_chunk = line_split.unwrap_or(10);
        if let Ok(content) = core::str::from_utf8(data) {
            let mut current_lines = 0;
            let (s1, s2) = make_suffix(chunk_idx);
            let mut out_path = [0u8; 64];
            let p_bytes = prefix.as_bytes();
            let mut p_len = 0;
            for &b in p_bytes {
                if p_len < 60 {
                    out_path[p_len] = b;
                    p_len += 1;
                }
            }
            out_path[p_len] = s1;
            out_path[p_len + 1] = s2;
            p_len += 2;

            let mut out_fd = if let Ok(out_name) = core::str::from_utf8(&out_path[..p_len]) {
                open(out_name, O_WRONLY | O_CREATE | O_TRUNC)
            } else {
                -1
            };

            for line in content.lines() {
                if current_lines >= lines_per_chunk {
                    if out_fd >= 0 {
                        close(out_fd as i32);
                    }
                    chunk_idx += 1;
                    let (ns1, ns2) = make_suffix(chunk_idx);
                    out_path[p_len - 2] = ns1;
                    out_path[p_len - 1] = ns2;
                    out_fd = if let Ok(out_name) = core::str::from_utf8(&out_path[..p_len]) {
                        open(out_name, O_WRONLY | O_CREATE | O_TRUNC)
                    } else {
                        -1
                    };
                    current_lines = 0;
                }

                if out_fd >= 0 {
                    let _ = write(out_fd as i32, line.as_bytes());
                    let _ = write(out_fd as i32, b"\n");
                }
                current_lines += 1;
            }

            if out_fd >= 0 {
                close(out_fd as i32);
            }
        }
    }

    println!("split: processed {} bytes into {} pieces", data.len(), chunk_idx + 1);
    0
}
