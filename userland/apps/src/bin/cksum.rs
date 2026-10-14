#![no_std]
#![no_main]

use libc::{arg, close, open, println, read, O_RDONLY};

struct PosixCksum {
    crc: u32,
    total_bytes: usize,
}

impl PosixCksum {
    fn new() -> Self {
        Self {
            crc: 0,
            total_bytes: 0,
        }
    }

    fn update(&mut self, data: &[u8]) {
        self.total_bytes += data.len();
        for &b in data {
            let mut c = ((self.crc >> 24) ^ (b as u32)) & 0xFF;
            for _ in 0..8 {
                if (c & 0x80) != 0 {
                    c = (c << 1) ^ 0x04C11DB7;
                } else {
                    c <<= 1;
                }
            }
            self.crc = (self.crc << 8) ^ c;
        }
    }

    fn finish(mut self) -> (u32, usize) {
        let mut len = self.total_bytes;
        while len > 0 {
            let b = (len & 0xFF) as u8;
            let mut c = ((self.crc >> 24) ^ (b as u32)) & 0xFF;
            for _ in 0..8 {
                if (c & 0x80) != 0 {
                    c = (c << 1) ^ 0x04C11DB7;
                } else {
                    c <<= 1;
                }
            }
            self.crc = (self.crc << 8) ^ c;
            len >>= 8;
        }
        (!self.crc, self.total_bytes)
    }
}

fn process_fd(fd: i32) -> (u32, usize) {
    let mut cksum = PosixCksum::new();
    let mut buf = [0u8; 512];
    loop {
        let n = read(fd, &mut buf);
        if n <= 0 {
            break;
        }
        cksum.update(&buf[..n as usize]);
    }
    cksum.finish()
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut files: [&str; 16] = [""; 16];
    let mut file_count = 0;

    let mut i = 1;
    while i < argc {
        let a = unsafe { arg(argv, i) };
        if a == "--help" || a == "-h" {
            println!("Usage: cksum [FILE]...");
            println!("Print CRC checksum and byte counts of each FILE.");
            return 0;
        } else {
            if file_count < 16 {
                files[file_count] = a;
                file_count += 1;
            }
        }
        i += 1;
    }

    if file_count == 0 {
        let (crc, len) = process_fd(0);
        println!("{} {}", crc, len);
        return 0;
    }

    let mut exit_code = 0;
    for &file in &files[..file_count] {
        if file == "-" {
            let (crc, len) = process_fd(0);
            println!("{} {}", crc, len);
        } else {
            let fd = open(file, O_RDONLY);
            if fd < 0 {
                println!("cksum: cannot open '{}'", file);
                exit_code = 1;
                continue;
            }
            let (crc, len) = process_fd(fd as i32);
            let _ = close(fd as i32);
            println!("{} {} {}", crc, len, file);
        }
    }
    exit_code
}
