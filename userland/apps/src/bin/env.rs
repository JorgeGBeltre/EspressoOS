#![no_std]
#![no_main]

use libc::{arg, close, getcwd_str, open, print, println, read, spawn, waitpid, O_RDONLY, PATH_MAX};

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc > 1 {
        let first = unsafe { arg(argv, 1) };
        if first == "-h" || first == "--help" {
            println!("Usage: env [COMMAND [ARG...]]");
            return 0;
        }
        // Execute command if given
        let mut path_buf = [0u8; 64];
        let cmd_path = if first.contains('/') {
            first
        } else {
            let p = b"/bin/";
            let b = first.as_bytes();
            let tot = (p.len() + b.len()).min(path_buf.len());
            path_buf[..p.len()].copy_from_slice(p);
            path_buf[p.len()..tot].copy_from_slice(&b[..tot - p.len()]);
            core::str::from_utf8(&path_buf[..tot]).unwrap_or(first)
        };
        let pid = spawn(cmd_path, unsafe { argv.offset(1) });
        if pid < 0 {
            println!("env: '{}': No such file or directory", first);
            return 127;
        }
        let mut st = 0;
        let _ = waitpid(pid as i32, &mut st, 0);
        return st;
    }

    let mut cwd_buf = [0u8; PATH_MAX];
    let cwd = getcwd_str(&mut cwd_buf).unwrap_or("/");
    let mut has_pwd = false;

    let fd = open("/run/sh.env", O_RDONLY);
    if fd >= 0 {
        let mut buf = [0u8; 1024];
        let n = read(fd as i32, &mut buf);
        close(fd as i32);
        if n > 0 {
            if let Ok(text) = core::str::from_utf8(&buf[..n as usize]) {
                for line in text.lines() {
                    let t = line.trim();
                    if t.is_empty() || t.starts_with('#') {
                        continue;
                    }
                    if t.starts_with("PWD=") {
                        has_pwd = true;
                        print!("PWD=");
                        println!("{}", cwd);
                    } else {
                        println!("{}", t);
                    }
                }
                if !has_pwd {
                    print!("PWD=");
                    println!("{}", cwd);
                }
                return 0;
            }
        }
    }

    println!("OS=EspressoOS");
    println!("ARCH=xtensa-lx7");
    println!("SHELL=/bin/sh");
    println!("USER=root");
    println!("HOME=/root");
    println!("PATH=/bin");
    println!("TERM=xterm-256color");
    print!("PWD=");
    println!("{}", cwd);
    0
}
