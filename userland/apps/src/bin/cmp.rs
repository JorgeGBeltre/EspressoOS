#![no_std]
#![no_main]

use libc::{arg, close, open, println, read};

const O_RDONLY: u32 = 1;

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 3 {
        println!("usage: cmp FILE1 FILE2");
        return 2;
    }

    let path1 = unsafe { arg(argv, 1) };
    let path2 = unsafe { arg(argv, 2) };

    if path1 == "-h" || path1 == "--help" {
        println!("usage: cmp FILE1 FILE2");
        println!("Compare two files byte by byte");
        return 0;
    }

    let fd1 = open(path1, O_RDONLY);
    if fd1 < 0 {
        println!("cmp: cannot open '{}'", path1);
        return 2;
    }

    let fd2 = open(path2, O_RDONLY);
    if fd2 < 0 {
        println!("cmp: cannot open '{}'", path2);
        close(fd1 as i32);
        return 2;
    }

    let mut buf1 = [0u8; 256];
    let mut buf2 = [0u8; 256];
    let mut byte_count: usize = 0;
    let mut line_count: usize = 1;

    let mut exit_code = 0;

    loop {
        let n1 = read(fd1 as i32, &mut buf1);
        let n2 = read(fd2 as i32, &mut buf2);

        if n1 < 0 || n2 < 0 {
            println!("cmp: read error");
            exit_code = 2;
            break;
        }

        let l1 = n1 as usize;
        let l2 = n2 as usize;

        if l1 == 0 && l2 == 0 {
            break; // Files are identical
        }

        let min_l = l1.min(l2);
        for i in 0..min_l {
            byte_count += 1;
            if buf1[i] != buf2[i] {
                println!("{}: byte {}, line {}", path1, byte_count, line_count);
                exit_code = 1;
                break;
            }
            if buf1[i] == b'\n' {
                line_count += 1;
            }
        }

        if exit_code != 0 {
            break;
        }

        if l1 != l2 {
            let shorter = if l1 < l2 { path1 } else { path2 };
            println!("cmp: EOF on {}", shorter);
            exit_code = 1;
            break;
        }
    }

    close(fd1 as i32);
    close(fd2 as i32);
    exit_code
}
