#![no_std]
#![no_main]

use libc::{arg, close, getcwd_str, open, println, read, O_RDONLY, PATH_MAX};

const DEFAULT_ENV: &[(&str, &str)] = &[
    ("PATH", "/bin"),
    ("USER", "root"),
    ("HOME", "/root"),
    ("SHELL", "/bin/sh"),
    ("TERM", "xterm-256color"),
    ("OS", "EspressoOS"),
    ("ARCH", "xtensa-lx7"),
];

fn lookup_file<'a>(key: &str, text: &'a str) -> Option<&'a str> {
    for line in text.lines() {
        let t = line.trim();
        if let Some(eq) = t.find('=') {
            if &t[..eq] == key {
                return Some(&t[eq + 1..]);
            }
        }
    }
    None
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut cwd_buf = [0u8; PATH_MAX];
    let cwd = getcwd_str(&mut cwd_buf).unwrap_or("/");

    let mut ebuf = [0u8; 1024];
    let mut elen = 0usize;
    let fd = open("/run/sh.env", O_RDONLY);
    if fd >= 0 {
        let n = read(fd as i32, &mut ebuf);
        close(fd as i32);
        if n > 0 {
            elen = n as usize;
        }
    }
    let etext = core::str::from_utf8(&ebuf[..elen]).unwrap_or("");

    if argc <= 1 {
        if !etext.is_empty() {
            for line in etext.lines() {
                let t = line.trim();
                if !t.is_empty() && !t.starts_with("PWD=") {
                    println!("{}", t);
                }
            }
        } else {
            for &(k, v) in DEFAULT_ENV {
                println!("{}={}", k, v);
            }
        }
        println!("PWD={}", cwd);
        return 0;
    }

    let mut found_all = true;
    for i in 1..argc {
        let var_name = unsafe { arg(argv, i) };
        if var_name == "PWD" {
            println!("{}", cwd);
            continue;
        }
        if let Some(val) = lookup_file(var_name, etext) {
            println!("{}", val);
            continue;
        }
        let mut match_found = false;
        if etext.is_empty() {
            for &(k, v) in DEFAULT_ENV {
                if k == var_name {
                    println!("{}", v);
                    match_found = true;
                    break;
                }
            }
        }
        if !match_found {
            found_all = false;
        }
    }

    if found_all { 0 } else { 1 }
}
