#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read};

const O_RDONLY: u32 = 1;
const PAGE_LINES: usize = 20;

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let (fd, is_stdin) = if argc > 1 {
        let path = unsafe { arg(argv, 1) };
        if path == "-" {
            (0i32, true)
        } else {
            let f = open(path, O_RDONLY);
            if f < 0 {
                println!("less: cannot open {}", path);
                return 1;
            }
            (f as i32, false)
        }
    } else {
        (0i32, true)
    };

    let mut buf = [0u8; 512];
    let mut line_count = 0usize;

    loop {
        let n = read(fd, &mut buf);
        if n <= 0 {
            break;
        }
        let chunk = &buf[..n as usize];
        for &b in chunk {
            let out_b = [b];
            let _ = libc::write(1, &out_b);
            if b == b'\n' {
                line_count += 1;
                if line_count >= PAGE_LINES && !is_stdin {
                    print!("--More--");
                    let mut cmd = [0u8; 1];
                    let r = read(0, &mut cmd);
                    print!("\r        \r");
                    if r > 0 && (cmd[0] == b'q' || cmd[0] == b'Q') {
                        if !is_stdin {
                            close(fd);
                        }
                        return 0;
                    } else if r > 0 && (cmd[0] == b'\n' || cmd[0] == b'\r') {
                        line_count = PAGE_LINES - 1;
                    } else {
                        line_count = 0;
                    }
                }
            }
        }
    }

    if !is_stdin {
        close(fd);
    }
    0
}
