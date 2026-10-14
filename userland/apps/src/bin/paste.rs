#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read, O_RDONLY};

const MAX_BUF: usize = 2048;

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 3 {
        println!("usage: paste [-d delim] <file1> <file2>");
        println!("       Merge lines of files");
        return 1;
    }

    let mut delim = "\t";
    let mut file1_opt: Option<&str> = None;
    let mut file2_opt: Option<&str> = None;

    let mut i = 1;
    while i < argc {
        let a = unsafe { arg(argv, i) };
        if a == "-d" {
            i += 1;
            if i < argc {
                delim = unsafe { arg(argv, i) };
            }
        } else if a == "-h" || a == "--help" {
            println!("usage: paste [-d delim] <file1> <file2>");
            return 0;
        } else if !a.starts_with('-') {
            if file1_opt.is_none() {
                file1_opt = Some(a);
            } else if file2_opt.is_none() {
                file2_opt = Some(a);
            }
        }
        i += 1;
    }

    let p1 = match file1_opt {
        Some(p) => p,
        None => return 1,
    };
    let p2 = match file2_opt {
        Some(p) => p,
        None => return 1,
    };

    let fd1 = open(p1, O_RDONLY);
    if fd1 < 0 {
        println!("paste: cannot open {}", p1);
        return 1;
    }
    let fd2 = open(p2, O_RDONLY);
    if fd2 < 0 {
        println!("paste: cannot open {}", p2);
        close(fd1 as i32);
        return 1;
    }

    let mut buf1 = [0u8; MAX_BUF];
    let mut buf2 = [0u8; MAX_BUF];

    let n1 = read(fd1 as i32, &mut buf1);
    let n2 = read(fd2 as i32, &mut buf2);
    close(fd1 as i32);
    close(fd2 as i32);

    let s1 = match core::str::from_utf8(&buf1[..n1.max(0) as usize]) {
        Ok(s) => s,
        Err(_) => "",
    };
    let s2 = match core::str::from_utf8(&buf2[..n2.max(0) as usize]) {
        Ok(s) => s,
        Err(_) => "",
    };

    let mut it1 = s1.lines();
    let mut it2 = s2.lines();

    loop {
        let l1 = it1.next();
        let l2 = it2.next();

        if l1.is_none() && l2.is_none() {
            break;
        }

        let str1 = l1.unwrap_or("");
        let str2 = l2.unwrap_or("");

        print!("{}", str1);
        print!("{}", delim);
        println!("{}", str2);
    }

    0
}
