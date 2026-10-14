#![no_std]
#![no_main]

use libc::{arg, close, open, println, read, write};

const O_RDONLY: u32 = 1;
const STDIN: i32 = 0;
const STDOUT: i32 = 1;
const MAX_LINES: usize = 32;
const LINE_BUF: usize = 128;

fn run_tail(fd: i32, n_lines: usize) {
    let mut ring: [[u8; LINE_BUF]; MAX_LINES] = [[0; LINE_BUF]; MAX_LINES];
    let mut lens: [usize; MAX_LINES] = [0; MAX_LINES];
    let mut count = 0usize;

    let mut buf = [0u8; 128];
    let mut cur = [0u8; LINE_BUF];
    let mut cur_len = 0usize;

    loop {
        let n = read(fd, &mut buf);
        if n <= 0 {
            break;
        }
        for &b in &buf[..n as usize] {
            if b == b'\n' {
                let idx = count % MAX_LINES;
                ring[idx][..cur_len].copy_from_slice(&cur[..cur_len]);
                lens[idx] = cur_len;
                count += 1;
                cur_len = 0;
            } else if cur_len < LINE_BUF {
                cur[cur_len] = b;
                cur_len += 1;
            }
        }
    }
    if cur_len > 0 {
        let idx = count % MAX_LINES;
        ring[idx][..cur_len].copy_from_slice(&cur[..cur_len]);
        lens[idx] = cur_len;
        count += 1;
    }

    let take = n_lines.min(count).min(MAX_LINES);
    let start = count.saturating_sub(take);
    for i in start..count {
        let idx = i % MAX_LINES;
        let l = lens[idx];
        let _ = write(STDOUT, &ring[idx][..l]);
        let _ = write(STDOUT, b"\n");
    }
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut max_lines = 10usize;
    let mut file_start = 1;

    if argc > 1 {
        let first = unsafe { arg(argv, 1) };
        if first.starts_with("-n") {
            if first.len() > 2 {
                if let Ok(n) = parse_usize(&first[2..]) {
                    max_lines = n;
                    file_start = 2;
                }
            } else if argc > 2 {
                let second = unsafe { arg(argv, 2) };
                if let Ok(n) = parse_usize(second) {
                    max_lines = n;
                    file_start = 3;
                }
            }
        }
    }

    if file_start >= argc {
        run_tail(STDIN, max_lines);
        return 0;
    }

    for i in file_start..argc {
        let path = unsafe { arg(argv, i) };
        let fd = open(path, O_RDONLY);
        if fd < 0 {
            println!("tail: cannot open '{}'", path);
            continue;
        }
        run_tail(fd as i32, max_lines);
        let _ = close(fd as i32);
    }
    0
}

fn parse_usize(s: &str) -> Result<usize, ()> {
    let mut val = 0usize;
    for b in s.bytes() {
        if b >= b'0' && b <= b'9' {
            val = val * 10 + (b - b'0') as usize;
        } else {
            return Err(());
        }
    }
    Ok(val)
}
