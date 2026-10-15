#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read, O_RDONLY};

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc > 1 {
        let first = unsafe { arg(argv, 1) };
        if first == "-h" || first == "--help" {
            println!("Usage: lsmod");
            println!("Show the status of modules in the Linux/Unix kernel");
            return 0;
        }
    }

    let fd = open("/proc/modules", O_RDONLY);
    if fd < 0 {
        println!("lsmod: cannot open /proc/modules");
        return 1;
    }
    let fd = fd as i32;

    println!("Module                  Size  Used by");

    let mut buf = [0u8; 1024];
    let mut line_buf = [0u8; 128];
    let mut line_len = 0usize;

    loop {
        let n = read(fd, &mut buf);
        if n <= 0 {
            break;
        }
        for &b in &buf[..n as usize] {
            if b == b'\n' {
                if line_len > 0 {
                    print_module_line(&line_buf[..line_len]);
                    line_len = 0;
                }
            } else if line_len < line_buf.len() {
                line_buf[line_len] = b;
                line_len += 1;
            }
        }
    }

    if line_len > 0 {
        print_module_line(&line_buf[..line_len]);
    }

    close(fd);
    0
}

fn print_module_line(line: &[u8]) {
    let s = match core::str::from_utf8(line) {
        Ok(v) => v,
        Err(_) => return,
    };

    let mut parts = s.split_ascii_whitespace();
    let mod_name = match parts.next() {
        Some(m) => m,
        None => return,
    };
    let mod_size = parts.next().unwrap_or("0");
    let mod_used = parts.next().unwrap_or("0");

    print!("{}", mod_name);
    let mut pad = 24usize.saturating_sub(mod_name.len());
    if pad == 0 {
        pad = 1;
    }
    for _ in 0..pad {
        print!(" ");
    }

    let mut size_pad = 8usize.saturating_sub(mod_size.len());
    for _ in 0..size_pad {
        print!(" ");
    }
    print!("{}", mod_size);
    print!("  ");
    println!("{}", mod_used);
}
