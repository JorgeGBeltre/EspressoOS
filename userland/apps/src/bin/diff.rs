#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read, O_RDONLY};

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 3 {
        println!("usage: diff <file1> <file2>");
        return 2;
    }

    let p1 = unsafe { arg(argv, 1) };
    let p2 = unsafe { arg(argv, 2) };

    let fd1 = open(p1, O_RDONLY);
    if fd1 < 0 {
        println!("diff: cannot open {}", p1);
        return 2;
    }

    let fd2 = open(p2, O_RDONLY);
    if fd2 < 0 {
        println!("diff: cannot open {}", p2);
        close(fd1 as i32);
        return 2;
    }

    let mut buf1 = [0u8; 1024];
    let mut buf2 = [0u8; 1024];

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

    let mut lines1 = s1.lines();
    let mut lines2 = s2.lines();

    let mut diff_found = false;
    let mut line_no = 1;

    loop {
        let l1 = lines1.next();
        let l2 = lines2.next();

        if l1.is_none() && l2.is_none() {
            break;
        }

        match (l1, l2) {
            (Some(a), Some(b)) => {
                if a != b {
                    if !diff_found {
                        println!("--- {}", p1);
                        println!("+++ {}", p2);
                        diff_found = true;
                    }
                    println!("@@ line {} @@", line_no);
                    println!("- {}", a);
                    println!("+ {}", b);
                }
            }
            (Some(a), None) => {
                if !diff_found {
                    println!("--- {}", p1);
                    println!("+++ {}", p2);
                    diff_found = true;
                }
                println!("@@ line {} @@", line_no);
                println!("- {}", a);
            }
            (None, Some(b)) => {
                if !diff_found {
                    println!("--- {}", p1);
                    println!("+++ {}", p2);
                    diff_found = true;
                }
                println!("@@ line {} @@", line_no);
                println!("+ {}", b);
            }
            (None, None) => break,
        }
        line_no += 1;
    }

    if diff_found {
        1
    } else {
        0
    }
}
