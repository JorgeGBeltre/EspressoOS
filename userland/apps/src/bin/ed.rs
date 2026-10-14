#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read, write, O_CREATE, O_RDONLY, O_TRUNC, O_WRONLY};

const MAX_LINES: usize = 32;
const MAX_LINE_LEN: usize = 64;

static mut BUFFER: [[u8; MAX_LINE_LEN]; MAX_LINES] = [[0; MAX_LINE_LEN]; MAX_LINES];
static mut LINE_LENS: [usize; MAX_LINES] = [0; MAX_LINES];
static mut LINE_COUNT: usize = 0;

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let initial_file = if argc > 1 {
        Some(unsafe { arg(argv, 1) })
    } else {
        None
    };

    if let Some(path) = initial_file {
        load_file(path);
    }

    println!("EspressoOS ed(1) text editor. Enter 'h' for help.");
    let mut cmd_buf = [0u8; 80];

    loop {
        print!(": ");
        let n = read_line(&mut cmd_buf);
        if n == 0 {
            break;
        }

        let cmd_str = match core::str::from_utf8(&cmd_buf[..n]) {
            Ok(s) => s.trim(),
            Err(_) => continue,
        };

        if cmd_str.is_empty() {
            continue;
        }

        let first_char = cmd_str.chars().next().unwrap_or(' ');
        match first_char {
            'q' => break,
            'p' => print_buffer(),
            'a' => append_mode(),
            'd' => {
                let parts: &str = cmd_str[1..].trim();
                if let Ok(idx) = parts.parse::<usize>() {
                    delete_line(idx);
                } else {
                    println!("? usage: d <line_number>");
                }
            }
            'w' => {
                let target = cmd_str[1..].trim();
                let path = if !target.is_empty() {
                    target
                } else if let Some(p) = initial_file {
                    p
                } else {
                    println!("? no filename specified");
                    continue;
                };
                save_file(path);
            }
            'r' => {
                let target = cmd_str[1..].trim();
                if !target.is_empty() {
                    load_file(target);
                } else {
                    println!("? usage: r <filename>");
                }
            }
            'h' => {
                println!("ed commands:");
                println!("  p         - print all lines");
                println!("  a         - append lines (enter '.' on its own line to finish)");
                println!("  d <N>     - delete line N (1-based)");
                println!("  w [file]  - write buffer to file");
                println!("  r <file>  - read file into buffer");
                println!("  q         - quit editor");
            }
            _ => println!("?"),
        }
    }

    0
}

fn read_line(buf: &mut [u8]) -> usize {
    let mut pos = 0;
    while pos < buf.len() - 1 {
        let mut b = [0u8; 1];
        let n = read(0, &mut b);
        if n <= 0 {
            break;
        }
        if b[0] == b'\n' {
            break;
        }
        if b[0] == b'\r' {
            continue;
        }
        buf[pos] = b[0];
        pos += 1;
    }
    pos
}

fn print_buffer() {
    unsafe {
        if LINE_COUNT == 0 {
            println!("(empty buffer)");
            return;
        }
        for i in 0..LINE_COUNT {
            let len = LINE_LENS[i];
            print!("{}: ", i + 1);
            for j in 0..len {
                print!("{}", BUFFER[i][j] as char);
            }
            println!("");
        }
    }
}

fn append_mode() {
    println!("Entering append mode. Enter '.' alone to stop.");
    let mut line_buf = [0u8; MAX_LINE_LEN];
    loop {
        let n = read_line(&mut line_buf);
        let s = match core::str::from_utf8(&line_buf[..n]) {
            Ok(val) => val.trim(),
            Err(_) => "",
        };
        if s == "." {
            break;
        }
        unsafe {
            if LINE_COUNT >= MAX_LINES {
                println!("? buffer full (max {} lines)", MAX_LINES);
                break;
            }
            let len = n.min(MAX_LINE_LEN);
            BUFFER[LINE_COUNT][..len].copy_from_slice(&line_buf[..len]);
            LINE_LENS[LINE_COUNT] = len;
            LINE_COUNT += 1;
        }
    }
}

fn delete_line(idx: usize) {
    unsafe {
        if idx == 0 || idx > LINE_COUNT {
            println!("? invalid line index");
            return;
        }
        let target = idx - 1;
        for i in target..(LINE_COUNT - 1) {
            BUFFER[i] = BUFFER[i + 1];
            LINE_LENS[i] = LINE_LENS[i + 1];
        }
        LINE_COUNT -= 1;
        println!("deleted line {}", idx);
    }
}

fn save_file(path: &str) {
    let fd = open(path, O_CREATE | O_WRONLY | O_TRUNC);
    if fd < 0 {
        println!("? cannot write to {}", path);
        return;
    }
    unsafe {
        let mut total_written = 0;
        for i in 0..LINE_COUNT {
            let len = LINE_LENS[i];
            let _ = write(fd as i32, &BUFFER[i][..len]);
            let _ = write(fd as i32, b"\n");
            total_written += len + 1;
        }
        close(fd as i32);
        println!("{} bytes written to {}", total_written, path);
    }
}

fn load_file(path: &str) {
    let fd = open(path, O_RDONLY);
    if fd < 0 {
        println!("? cannot open {}", path);
        return;
    }
    let mut read_buf = [0u8; 1024];
    let n = read(fd as i32, &mut read_buf);
    close(fd as i32);

    if n <= 0 {
        return;
    }

    unsafe {
        LINE_COUNT = 0;
        let mut start = 0;
        for i in 0..n as usize {
            if read_buf[i] == b'\n' || i == (n as usize - 1) {
                let end = if read_buf[i] == b'\n' { i } else { i + 1 };
                let len = (end - start).min(MAX_LINE_LEN);
                if LINE_COUNT < MAX_LINES {
                    BUFFER[LINE_COUNT][..len].copy_from_slice(&read_buf[start..start + len]);
                    LINE_LENS[LINE_COUNT] = len;
                    LINE_COUNT += 1;
                }
                start = i + 1;
            }
        }
        println!("{} lines read from {}", LINE_COUNT, path);
    }
}
