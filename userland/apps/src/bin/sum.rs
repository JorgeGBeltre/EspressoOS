#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read, O_RDONLY};

fn sum_stream(fd: i32, sysv: bool) -> (u32, usize) {
    let mut buf = [0u8; 512];
    let mut total_bytes = 0usize;
    let mut checksum: u32 = 0;

    loop {
        let n = read(fd, &mut buf);
        if n <= 0 {
            break;
        }
        let cnt = n as usize;
        total_bytes += cnt;

        if sysv {
            for &b in &buf[..cnt] {
                checksum = checksum.wrapping_add(b as u32);
            }
        } else {
            // BSD 16-bit right-rotation checksum
            for &b in &buf[..cnt] {
                checksum = (checksum >> 1) + ((checksum & 1) << 15);
                checksum = (checksum + (b as u32)) & 0xffff;
            }
        }
    }

    if sysv {
        let r = (checksum & 0xffff) + ((checksum >> 16) & 0xffff);
        let c = (r & 0xffff) + (r >> 16);
        checksum = c & 0xffff;
    }

    let blocks = (total_bytes + 511) / 512;
    (checksum, blocks)
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut sysv = false;
    let mut arg_idx = 1;

    while arg_idx < argc {
        let s = unsafe { arg(argv, arg_idx) };
        if s == "-h" || s == "--help" {
            println!("Usage: sum [-r | -s] [FILE...]");
            println!("Checksum and count the blocks in a file");
            println!("  -r  Use the BSD sum algorithm (default)");
            println!("  -s  Use the System V sum algorithm");
            return 0;
        } else if s == "-s" || s == "--sysv" {
            sysv = true;
            arg_idx += 1;
        } else if s == "-r" {
            sysv = false;
            arg_idx += 1;
        } else {
            break;
        }
    }

    if arg_idx >= argc {
        let (cksum, blocks) = sum_stream(0, sysv);
        print!("{}", cksum);
        print!(" ");
        println!("{}", blocks);
        return 0;
    }

    let mut all_ok = true;
    for i in arg_idx..argc {
        let file = unsafe { arg(argv, i) };
        let fd = open(file, O_RDONLY);
        if fd < 0 {
            print!("sum: cannot open '");
            print!("{}", file);
            println!("'");
            all_ok = false;
            continue;
        }
        let (cksum, blocks) = sum_stream(fd as i32, sysv);
        close(fd as i32);

        print!("{}", cksum);
        print!(" ");
        print!("{}", blocks);
        print!(" ");
        println!("{}", file);
    }

    if all_ok { 0 } else { 1 }
}
