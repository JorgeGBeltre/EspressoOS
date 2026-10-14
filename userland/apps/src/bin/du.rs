#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, readdir, seek};

const O_RDONLY: u32 = 1;

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

fn print_size(bytes: usize, human: bool) {
    if !human {
        let kb = (bytes + 1023) / 1024;
        print_dec(kb);
        print!("\t");
    } else if bytes < 1024 {
        print_dec(bytes);
        print!("B\t");
    } else if bytes < 1024 * 1024 {
        print_dec(bytes / 1024);
        print!("K\t");
    } else {
        print_dec(bytes / (1024 * 1024));
        print!("M\t");
    }
}

fn file_size(path: &str) -> usize {
    let fd = open(path, O_RDONLY);
    if fd < 0 {
        return 0;
    }
    let s = seek(fd as i32, 0, 2);
    close(fd as i32);
    if s > 0 {
        s as usize
    } else {
        0
    }
}

fn dir_size(dir_path: &str, human: bool, summary: bool) -> usize {
    let mut total = 0usize;
    let mut buf = [0u8; 1024];
    let n = readdir(dir_path, &mut buf);
    if n <= 0 {
        return file_size(dir_path);
    }

    let limit = n as usize;
    let mut pos = 0;
    while pos + 11 <= limit {
        let kind = buf[pos + 8];
        let name_len = u16::from_le_bytes([buf[pos + 9], buf[pos + 10]]) as usize;
        pos += 11;

        if pos + name_len > limit {
            break;
        }

        if let Ok(name) = core::str::from_utf8(&buf[pos..pos + name_len]) {
            if name != "." && name != ".." {
                let mut path_buf = [0u8; 64];
                let mut o = 0;
                let cbytes = dir_path.as_bytes();
                if o + cbytes.len() < path_buf.len() {
                    path_buf[o..o + cbytes.len()].copy_from_slice(cbytes);
                    o += cbytes.len();
                }
                if o > 0 && path_buf[o - 1] != b'/' && o < path_buf.len() {
                    path_buf[o] = b'/';
                    o += 1;
                }
                let nbytes = name.as_bytes();
                if o + nbytes.len() < path_buf.len() {
                    path_buf[o..o + nbytes.len()].copy_from_slice(nbytes);
                    o += nbytes.len();
                }

                if let Ok(full) = core::str::from_utf8(&path_buf[..o]) {
                    if kind == 2 {
                        let sub = dir_size(full, human, summary);
                        total += sub;
                        if !summary {
                            print_size(sub, human);
                            println!("{}", full);
                        }
                    } else {
                        let s = file_size(full);
                        total += s;
                    }
                }
            }
        }
        pos += name_len;
    }
    total
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut human = false;
    let mut summary = false;
    let mut target = ".";
    let mut i = 1;

    while i < argc {
        let a = unsafe { arg(argv, i) };
        if a == "-h" {
            human = true;
        } else if a == "-s" {
            summary = true;
        } else if !a.starts_with('-') {
            target = a;
        }
        i += 1;
    }

    let total = dir_size(target, human, summary);
    print_size(total, human);
    println!("{}", target);
    0
}
