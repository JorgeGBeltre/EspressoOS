#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read, write};

const O_RDONLY: u32 = 1;

const B64_TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn b64_val(c: u8) -> Option<u8> {
    match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

fn encode_fd(fd: i32) {
    let mut in_buf = [0u8; 3];
    let mut in_len = 0usize;
    let mut out_buf = [0u8; 68];
    let mut out_len = 0usize;

    let mut raw_buf = [0u8; 256];
    loop {
        let n = read(fd, &mut raw_buf);
        if n <= 0 { break; }
        for &b in &raw_buf[..n as usize] {
            in_buf[in_len] = b;
            in_len += 1;
            if in_len == 3 {
                let b0 = in_buf[0];
                let b1 = in_buf[1];
                let b2 = in_buf[2];
                out_buf[out_len] = B64_TABLE[(b0 >> 2) as usize];
                out_buf[out_len + 1] = B64_TABLE[(((b0 & 3) << 4) | (b1 >> 4)) as usize];
                out_buf[out_len + 2] = B64_TABLE[(((b1 & 15) << 2) | (b2 >> 6)) as usize];
                out_buf[out_len + 3] = B64_TABLE[(b2 & 63) as usize];
                out_len += 4;
                in_len = 0;

                if out_len >= 64 {
                    out_buf[out_len] = b'\n';
                    out_len += 1;
                    let _ = write(1, &out_buf[..out_len]);
                    out_len = 0;
                }
            }
        }
    }

    if in_len == 1 {
        let b0 = in_buf[0];
        out_buf[out_len] = B64_TABLE[(b0 >> 2) as usize];
        out_buf[out_len + 1] = B64_TABLE[((b0 & 3) << 4) as usize];
        out_buf[out_len + 2] = b'=';
        out_buf[out_len + 3] = b'=';
        out_len += 4;
    } else if in_len == 2 {
        let b0 = in_buf[0];
        let b1 = in_buf[1];
        out_buf[out_len] = B64_TABLE[(b0 >> 2) as usize];
        out_buf[out_len + 1] = B64_TABLE[(((b0 & 3) << 4) | (b1 >> 4)) as usize];
        out_buf[out_len + 2] = B64_TABLE[((b1 & 15) << 2) as usize];
        out_buf[out_len + 3] = b'=';
        out_len += 4;
    }

    if out_len > 0 {
        out_buf[out_len] = b'\n';
        out_len += 1;
        let _ = write(1, &out_buf[..out_len]);
    }
}

fn decode_fd(fd: i32) {
    let mut quad = [0u8; 4];
    let mut quad_len = 0usize;
    let mut out_buf = [0u8; 256];
    let mut out_len = 0usize;

    let mut raw_buf = [0u8; 256];
    loop {
        let n = read(fd, &mut raw_buf);
        if n <= 0 { break; }
        for &b in &raw_buf[..n as usize] {
            if b == b'\r' || b == b'\n' || b == b' ' || b == b'\t' {
                continue;
            }
            quad[quad_len] = b;
            quad_len += 1;
            if quad_len == 4 {
                let v0 = b64_val(quad[0]).unwrap_or(0);
                let v1 = b64_val(quad[1]).unwrap_or(0);
                let v2 = b64_val(quad[2]).unwrap_or(0);
                let v3 = b64_val(quad[3]).unwrap_or(0);

                out_buf[out_len] = (v0 << 2) | (v1 >> 4);
                out_len += 1;
                if quad[2] != b'=' {
                    out_buf[out_len] = ((v1 & 15) << 4) | (v2 >> 2);
                    out_len += 1;
                }
                if quad[3] != b'=' {
                    out_buf[out_len] = ((v2 & 3) << 6) | v3;
                    out_len += 1;
                }
                quad_len = 0;

                if out_len >= 192 {
                    let _ = write(1, &out_buf[..out_len]);
                    out_len = 0;
                }
            }
        }
    }
    if out_len > 0 {
        let _ = write(1, &out_buf[..out_len]);
    }
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut decode = false;
    let mut file_path: Option<&str> = None;

    let mut i = 1;
    while i < argc {
        let a = unsafe { arg(argv, i) };
        if a == "-d" || a == "--decode" {
            decode = true;
        } else if a == "-h" || a == "--help" {
            println!("Usage: base64 [-d] [FILE]");
            println!("Base64 encode or decode FILE, or standard input, to standard output.");
            return 0;
        } else if file_path.is_none() && !a.starts_with("-") {
            file_path = Some(a);
        }
        i += 1;
    }

    if let Some(path) = file_path {
        let fd = open(path, O_RDONLY);
        if fd < 0 {
            println!("base64: cannot open {}", path);
            return 1;
        }
        if decode {
            decode_fd(fd as i32);
        } else {
            encode_fd(fd as i32);
        }
        close(fd as i32);
    } else {
        if decode {
            decode_fd(0);
        } else {
            encode_fd(0);
        }
    }
    0
}
