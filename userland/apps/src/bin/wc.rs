#![no_std]
#![no_main]

use libc::{arg, close, open, println, read};

const O_RDONLY: u32 = 1;
const STDIN: i32 = 0;

struct Counts {
    lines: usize,
    words: usize,
    bytes: usize,
}

fn count_fd(fd: i32) -> Result<Counts, ()> {
    let mut buf = [0u8; 128];
    let mut lines = 0;
    let mut words = 0;
    let mut bytes = 0;
    let mut in_word = false;

    loop {
        let n = read(fd, &mut buf);
        if n < 0 {
            return Err(());
        }
        if n == 0 {
            break;
        }
        let len = n as usize;
        bytes += len;
        for i in 0..len {
            let c = buf[i];
            if c == b'\n' {
                lines += 1;
            }
            if c == b' ' || c == b'\t' || c == b'\n' || c == b'\r' {
                in_word = false;
            } else if !in_word {
                in_word = true;
                words += 1;
            }
        }
    }
    Ok(Counts { lines, words, bytes })
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc <= 1 {
        if let Ok(c) = count_fd(STDIN) {
            println!("{:>8} {:>8} {:>8}", c.lines, c.words, c.bytes);
            return 0;
        } else {
            println!("wc: read error on stdin");
            return 1;
        }
    }

    let mut tot = Counts { lines: 0, words: 0, bytes: 0 };
    let mut status = 0;
    for i in 1..argc {
        let path = unsafe { arg(argv, i) };
        let fd = open(path, O_RDONLY);
        if fd < 0 {
            println!("wc: cannot open '{}'", path);
            status = 1;
            continue;
        }
        match count_fd(fd as i32) {
            Ok(c) => {
                println!("{:>8} {:>8} {:>8} {}", c.lines, c.words, c.bytes, path);
                tot.lines += c.lines;
                tot.words += c.words;
                tot.bytes += c.bytes;
            }
            Err(_) => {
                println!("wc: error reading '{}'", path);
                status = 1;
            }
        }
        let _ = close(fd as i32);
    }

    if argc > 2 {
        println!("{:>8} {:>8} {:>8} total", tot.lines, tot.words, tot.bytes);
    }
    status
}
