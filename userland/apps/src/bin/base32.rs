#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read, write, O_RDONLY};

const STDOUT: i32 = 1;
const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

fn decode_char(c: u8) -> Option<u8> {
    match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a'),
        b'2'..=b'7' => Some(c - b'2' + 26),
        _ => None,
    }
}

fn encode_stream(fd: i32) {
    let mut in_buf = [0u8; 5];
    let mut in_len = 0usize;
    let mut raw_buf = [0u8; 512];

    loop {
        let n = read(fd, &mut raw_buf);
        if n <= 0 {
            break;
        }
        for &b in &raw_buf[..n as usize] {
            in_buf[in_len] = b;
            in_len += 1;
            if in_len == 5 {
                let mut out = [b'='; 8];
                out[0] = ALPHABET[(in_buf[0] >> 3) as usize];
                out[1] = ALPHABET[(((in_buf[0] & 0x07) << 2) | (in_buf[1] >> 6)) as usize];
                out[2] = ALPHABET[((in_buf[1] >> 1) & 0x1f) as usize];
                out[3] = ALPHABET[(((in_buf[1] & 0x01) << 4) | (in_buf[2] >> 4)) as usize];
                out[4] = ALPHABET[(((in_buf[2] & 0x0f) << 1) | (in_buf[3] >> 7)) as usize];
                out[5] = ALPHABET[((in_buf[3] >> 2) & 0x1f) as usize];
                out[6] = ALPHABET[(((in_buf[3] & 0x03) << 3) | (in_buf[4] >> 5)) as usize];
                out[7] = ALPHABET[(in_buf[4] & 0x1f) as usize];
                let _ = write(STDOUT, &out);
                in_len = 0;
            }
        }
    }

    if in_len > 0 {
        let mut out = [b'='; 8];
        out[0] = ALPHABET[(in_buf[0] >> 3) as usize];
        if in_len == 1 {
            out[1] = ALPHABET[((in_buf[0] & 0x07) << 2) as usize];
        } else if in_len == 2 {
            out[1] = ALPHABET[(((in_buf[0] & 0x07) << 2) | (in_buf[1] >> 6)) as usize];
            out[2] = ALPHABET[((in_buf[1] >> 1) & 0x1f) as usize];
            out[3] = ALPHABET[((in_buf[1] & 0x01) << 4) as usize];
        } else if in_len == 3 {
            out[1] = ALPHABET[(((in_buf[0] & 0x07) << 2) | (in_buf[1] >> 6)) as usize];
            out[2] = ALPHABET[((in_buf[1] >> 1) & 0x1f) as usize];
            out[3] = ALPHABET[(((in_buf[1] & 0x01) << 4) | (in_buf[2] >> 4)) as usize];
            out[4] = ALPHABET[((in_buf[2] & 0x0f) << 1) as usize];
        } else if in_len == 4 {
            out[1] = ALPHABET[(((in_buf[0] & 0x07) << 2) | (in_buf[1] >> 6)) as usize];
            out[2] = ALPHABET[((in_buf[1] >> 1) & 0x1f) as usize];
            out[3] = ALPHABET[(((in_buf[1] & 0x01) << 4) | (in_buf[2] >> 4)) as usize];
            out[4] = ALPHABET[(((in_buf[2] & 0x0f) << 1) | (in_buf[3] >> 7)) as usize];
            out[5] = ALPHABET[((in_buf[3] >> 2) & 0x1f) as usize];
            out[6] = ALPHABET[((in_buf[3] & 0x03) << 3) as usize];
        }
        let _ = write(STDOUT, &out);
    }
    let _ = write(STDOUT, b"\n");
}

fn decode_stream(fd: i32) {
    let mut in_chars = [0u8; 8];
    let mut in_count = 0usize;
    let mut raw_buf = [0u8; 512];

    loop {
        let n = read(fd, &mut raw_buf);
        if n <= 0 {
            break;
        }
        for &b in &raw_buf[..n as usize] {
            if b == b'=' || decode_char(b).is_some() {
                in_chars[in_count] = b;
                in_count += 1;
                if in_count == 8 {
                    let mut vals = [0u8; 8];
                    let mut pad_count = 0usize;
                    for i in 0..8 {
                        if in_chars[i] == b'=' {
                            pad_count += 1;
                            vals[i] = 0;
                        } else {
                            vals[i] = decode_char(in_chars[i]).unwrap_or(0);
                        }
                    }

                    let mut out = [0u8; 5];
                    out[0] = (vals[0] << 3) | (vals[1] >> 2);
                    out[1] = ((vals[1] & 0x03) << 6) | (vals[2] << 1) | (vals[3] >> 4);
                    out[2] = ((vals[3] & 0x0f) << 4) | (vals[4] >> 1);
                    out[3] = ((vals[4] & 0x01) << 7) | (vals[5] << 2) | (vals[6] >> 3);
                    out[4] = ((vals[6] & 0x07) << 5) | vals[7];

                    let emit_len = match pad_count {
                        0 => 5,
                        1 => 4,
                        3 => 3,
                        4 => 2,
                        6 => 1,
                        _ => 5,
                    };
                    let _ = write(STDOUT, &out[..emit_len]);
                    in_count = 0;
                }
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut decode = false;
    let mut file_idx = 1;

    for i in 1..argc {
        let s = unsafe { arg(argv, i) };
        if s == "-h" || s == "--help" {
            println!("Usage: base32 [-d] [FILE]");
            println!("Base32 encode or decode FILE, or standard input, to standard output");
            println!("  -d, --decode  Decode data");
            return 0;
        } else if s == "-d" || s == "--decode" {
            decode = true;
            file_idx = i + 1;
        } else if !s.starts_with('-') {
            file_idx = i;
            break;
        }
    }

    let mut fd = 0i32;
    if file_idx < argc {
        let path = unsafe { arg(argv, file_idx) };
        if path != "-" {
            let opened = open(path, O_RDONLY);
            if opened < 0 {
                print!("base32: cannot open ");
                println!("{}", path);
                return 1;
            }
            fd = opened as i32;
        }
    }

    if decode {
        decode_stream(fd);
    } else {
        encode_stream(fd);
    }

    if fd > 0 {
        close(fd);
    }
    0
}
