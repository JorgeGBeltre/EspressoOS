#![no_std]
#![no_main]

use libc::{arg, close, open, println, read, uptime_ms, O_RDONLY};

const MAX_LINES: usize = 64;
const BUF_SIZE: usize = 1024;

struct Rng {
    state: u64,
}

impl Rng {
    fn new() -> Self {
        let mut seed = (uptime_ms() as u64) ^ 0x9E3779B97F4A7C15;
        let fd = open("/dev/urandom", O_RDONLY);
        if fd >= 0 {
            let mut buf = [0u8; 8];
            if read(fd as i32, &mut buf) == 8 {
                seed ^= u64::from_le_bytes(buf);
            }
            let _ = close(fd as i32);
        }
        if seed == 0 {
            seed = 0x517cc1b727220a95;
        }
        Self { state: seed }
    }

    fn next_u32(&mut self) -> u32 {
        self.state = self
            .state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.state >> 18) ^ (self.state >> 42)) as u32
    }

    fn gen_range(&mut self, upper: usize) -> usize {
        if upper <= 1 {
            0
        } else {
            (self.next_u32() as usize) % upper
        }
    }
}

fn parse_usize(s: &str) -> Option<usize> {
    let mut val: usize = 0;
    if s.is_empty() {
        return None;
    }
    for b in s.bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        val = val.checked_mul(10)?.checked_add((b - b'0') as usize)?;
    }
    Some(val)
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut echo_mode = false;
    let mut range_lo: Option<usize> = None;
    let mut range_hi: Option<usize> = None;
    let mut head_count: Option<usize> = None;
    let mut positional: [&str; MAX_LINES] = [""; MAX_LINES];
    let mut pos_count = 0;

    let mut i = 1;
    while i < argc {
        let a = unsafe { arg(argv, i) };
        if a == "--help" || a == "-h" {
            println!("Usage: shuf [OPTION]... [FILE]");
            println!("  or:  shuf -e [OPTION]... [ARG]...");
            println!("  or:  shuf -i LO-HI [OPTION]...");
            println!("Write a random permutation of the input lines to standard output.");
            println!("  -e           treat each ARG as an input line");
            println!("  -i LO-HI     treat each number LO through HI as an input line");
            println!("  -n COUNT     output at most COUNT lines");
            return 0;
        } else if a == "-e" {
            echo_mode = true;
        } else if a == "-n" {
            i += 1;
            if i < argc {
                head_count = parse_usize(unsafe { arg(argv, i) });
            }
        } else if a.starts_with("-n") && a.len() > 2 {
            head_count = parse_usize(&a[2..]);
        } else if a == "-i" {
            i += 1;
            if i < argc {
                let range_str = unsafe { arg(argv, i) };
                if let Some((lo, hi)) = range_str.split_once('-') {
                    range_lo = parse_usize(lo);
                    range_hi = parse_usize(hi);
                }
            }
        } else if a.starts_with("-i") && a.len() > 2 {
            if let Some((lo, hi)) = a[2..].split_once('-') {
                range_lo = parse_usize(lo);
                range_hi = parse_usize(hi);
            }
        } else {
            if pos_count < MAX_LINES {
                positional[pos_count] = a;
                pos_count += 1;
            }
        }
        i += 1;
    }

    let mut rng = Rng::new();

    if let (Some(lo), Some(hi)) = (range_lo, range_hi) {
        if lo > hi {
            println!("shuf: invalid input range: '{}-{}'", lo, hi);
            return 1;
        }
        let total = hi - lo + 1;
        let mut numbers: [usize; MAX_LINES] = [0; MAX_LINES];
        let count = core::cmp::min(total, MAX_LINES);
        for idx in 0..count {
            numbers[idx] = lo + idx;
        }
        for idx in (1..count).rev() {
            let j = rng.gen_range(idx + 1);
            numbers.swap(idx, j);
        }
        let limit = head_count.unwrap_or(count).min(count);
        for idx in 0..limit {
            println!("{}", numbers[idx]);
        }
        return 0;
    }

    if echo_mode {
        let mut lines = positional;
        let count = pos_count;
        for idx in (1..count).rev() {
            let j = rng.gen_range(idx + 1);
            lines.swap(idx, j);
        }
        let limit = head_count.unwrap_or(count).min(count);
        for idx in 0..limit {
            println!("{}", lines[idx]);
        }
        return 0;
    }

    // Read from file or stdin
    let file = if pos_count > 0 { positional[0] } else { "-" };
    let fd = if file == "-" {
        0
    } else {
        let opened = open(file, O_RDONLY);
        if opened < 0 {
            println!("shuf: cannot open '{}'", file);
            return 1;
        }
        opened as i32
    };

    let mut raw_buf = [0u8; BUF_SIZE];
    let mut total_read = 0;
    while total_read < BUF_SIZE {
        let n = read(fd, &mut raw_buf[total_read..]);
        if n <= 0 {
            break;
        }
        total_read += n as usize;
    }
    if fd != 0 {
        let _ = close(fd);
    }

    let mut line_ptrs: [&str; MAX_LINES] = [""; MAX_LINES];
    let mut line_count = 0;
    let s = core::str::from_utf8(&raw_buf[..total_read]).unwrap_or("");
    for line in s.split('\n') {
        let trimmed = line.trim_end_matches('\r');
        if !trimmed.is_empty() && line_count < MAX_LINES {
            line_ptrs[line_count] = trimmed;
            line_count += 1;
        }
    }

    if line_count == 0 {
        return 0;
    }

    for idx in (1..line_count).rev() {
        let j = rng.gen_range(idx + 1);
        line_ptrs.swap(idx, j);
    }

    let limit = head_count.unwrap_or(line_count).min(line_count);
    for idx in 0..limit {
        println!("{}", line_ptrs[idx]);
    }
    0
}
