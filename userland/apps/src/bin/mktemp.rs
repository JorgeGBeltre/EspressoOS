#![no_std]
#![no_main]

use libc::{arg, close, mkdir, open, println, read, uptime_ms, O_CREATE, O_RDONLY, O_TRUNC, O_WRONLY};

const BASE36: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut is_dir = false;
    let mut template = "/tmp/tmp.XXXXXX";

    let mut i = 1;
    while i < argc {
        let a = unsafe { arg(argv, i) };
        if a == "-d" {
            is_dir = true;
        } else if !a.starts_with('-') {
            template = a;
        }
        i += 1;
    }

    let mut rand_bytes = [0u8; 6];
    let ufd = open("/dev/urandom", O_RDONLY);
    let mut got_random = false;
    if ufd >= 0 {
        if read(ufd as i32, &mut rand_bytes) == 6 {
            got_random = true;
        }
        close(ufd as i32);
    }

    if !got_random {
        // Fallback LCG using uptime ticks
        let mut seed = uptime_ms();
        for b in &mut rand_bytes {
            seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
            *b = (seed >> 16) as u8;
        }
    }

    let mut path_buf = [0u8; 128];
    let t_bytes = template.as_bytes();
    let t_len = t_bytes.len().min(120);
    path_buf[..t_len].copy_from_slice(&t_bytes[..t_len]);

    // Replace trailing X's with random chars
    let mut rand_idx = 0;
    for idx in (0..t_len).rev() {
        if path_buf[idx] == b'X' && rand_idx < rand_bytes.len() {
            let choice = (rand_bytes[rand_idx] as usize) % BASE36.len();
            path_buf[idx] = BASE36[choice];
            rand_idx += 1;
        }
    }
    path_buf[t_len] = 0;

    let path_str = match core::str::from_utf8(&path_buf[..t_len]) {
        Ok(s) => s,
        Err(_) => {
            println!("mktemp: invalid path string encoding");
            return 1;
        }
    };

    if is_dir {
        let ret = mkdir(path_str);
        if ret < 0 {
            println!("mktemp: failed to create directory {}", path_str);
            return 1;
        }
    } else {
        let fd = open(path_str, O_CREATE | O_WRONLY | O_TRUNC);
        if fd < 0 {
            println!("mktemp: failed to create file {}", path_str);
            return 1;
        }
        close(fd as i32);
    }

    println!("{}", path_str);
    0
}
