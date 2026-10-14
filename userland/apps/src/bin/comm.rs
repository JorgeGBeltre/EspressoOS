#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read, O_RDONLY};

const MAX_BUF: usize = 2048;

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 3 {
        println!("usage: comm [-123] <file1> <file2>");
        println!("       Compare two sorted files line by line");
        return 2;
    }

    let mut sup1 = false;
    let mut sup2 = false;
    let mut sup3 = false;
    let mut f1_opt: Option<&str> = None;
    let mut f2_opt: Option<&str> = None;

    for i in 1..argc {
        let a = unsafe { arg(argv, i) };
        if a.starts_with('-') && a.len() > 1 && a.as_bytes()[1] >= b'1' && a.as_bytes()[1] <= b'3' {
            for &b in a.as_bytes()[1..].iter() {
                if b == b'1' {
                    sup1 = true;
                } else if b == b'2' {
                    sup2 = true;
                } else if b == b'3' {
                    sup3 = true;
                }
            }
        } else if !a.starts_with('-') {
            if f1_opt.is_none() {
                f1_opt = Some(a);
            } else if f2_opt.is_none() {
                f2_opt = Some(a);
            }
        }
    }

    let path1 = match f1_opt {
        Some(p) => p,
        None => {
            println!("comm: missing file1 operand");
            return 2;
        }
    };

    let path2 = match f2_opt {
        Some(p) => p,
        None => {
            println!("comm: missing file2 operand");
            return 2;
        }
    };

    let fd1 = open(path1, O_RDONLY);
    if fd1 < 0 {
        println!("comm: cannot open {}", path1);
        return 2;
    }

    let fd2 = open(path2, O_RDONLY);
    if fd2 < 0 {
        println!("comm: cannot open {}", path2);
        close(fd1 as i32);
        return 2;
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

    let mut it1 = s1.lines().peekable();
    let mut it2 = s2.lines().peekable();

    loop {
        let p1 = it1.peek().copied();
        let p2 = it2.peek().copied();

        match (p1, p2) {
            (Some(l1), Some(l2)) => {
                if l1 < l2 {
                    if !sup1 {
                        println!("{}", l1);
                    }
                    it1.next();
                } else if l1 > l2 {
                    if !sup2 {
                        if !sup1 {
                            print!("\t");
                        }
                        println!("{}", l2);
                    }
                    it2.next();
                } else {
                    if !sup3 {
                        if !sup1 {
                            print!("\t");
                        }
                        if !sup2 {
                            print!("\t");
                        }
                        println!("{}", l1);
                    }
                    it1.next();
                    it2.next();
                }
            }
            (Some(l1), None) => {
                if !sup1 {
                    println!("{}", l1);
                }
                it1.next();
            }
            (None, Some(l2)) => {
                if !sup2 {
                    if !sup1 {
                        print!("\t");
                    }
                    println!("{}", l2);
                }
                it2.next();
            }
            (None, None) => break,
        }
    }

    0
}
