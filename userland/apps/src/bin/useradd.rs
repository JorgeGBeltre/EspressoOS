#![no_std]
#![no_main]

use libc::{arg, close, mkdir, open, println, write, O_CREATE, O_WRONLY};

const O_WRONLY_APPEND: u32 = 0x200 | 2; // CREATE | APPEND | WRONLY

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        println!("usage: useradd [-u UID] [-d HOME] [-s SHELL] USERNAME");
        return 1;
    }

    let mut username = "";
    let mut uid = "1001";
    let mut home = "";
    let mut shell = "/bin/sh";

    let mut i = 1;
    while i < argc {
        let a = unsafe { arg(argv, i) };
        if a == "-h" || a == "--help" {
            println!("usage: useradd [-u UID] [-d HOME] [-s SHELL] USERNAME");
            println!("Create a new user account in /etc/passwd and /etc/group");
            return 0;
        } else if a == "-u" && i + 1 < argc {
            i += 1;
            uid = unsafe { arg(argv, i) };
        } else if a == "-d" && i + 1 < argc {
            i += 1;
            home = unsafe { arg(argv, i) };
        } else if a == "-s" && i + 1 < argc {
            i += 1;
            shell = unsafe { arg(argv, i) };
        } else if !a.starts_with('-') {
            username = a;
        }
        i += 1;
    }

    if username.is_empty() {
        println!("useradd: missing username operand");
        return 1;
    }

    let mut home_buf = [0u8; 64];
    let default_home = if home.is_empty() {
        let prefix = b"/home/";
        let mut idx = 0;
        for &b in prefix { home_buf[idx] = b; idx += 1; }
        for &b in username.as_bytes() { if idx < 63 { home_buf[idx] = b; idx += 1; } }
        core::str::from_utf8(&home_buf[..idx]).unwrap_or("/home")
    } else {
        home
    };

    // Append to /etc/passwd
    let fd_passwd = open("/etc/passwd", O_WRONLY_APPEND);
    if fd_passwd >= 0 {
        let _ = write(fd_passwd as i32, username.as_bytes());
        let _ = write(fd_passwd as i32, b":x:");
        let _ = write(fd_passwd as i32, uid.as_bytes());
        let _ = write(fd_passwd as i32, b":");
        let _ = write(fd_passwd as i32, uid.as_bytes());
        let _ = write(fd_passwd as i32, b"::");
        let _ = write(fd_passwd as i32, default_home.as_bytes());
        let _ = write(fd_passwd as i32, b":");
        let _ = write(fd_passwd as i32, shell.as_bytes());
        let _ = write(fd_passwd as i32, b"\n");
        close(fd_passwd as i32);
    } else {
        println!("useradd: failed to open /etc/passwd");
        return 1;
    }

    // Append to /etc/group
    let fd_group = open("/etc/group", O_WRONLY_APPEND);
    if fd_group >= 0 {
        let _ = write(fd_group as i32, username.as_bytes());
        let _ = write(fd_group as i32, b":x:");
        let _ = write(fd_group as i32, uid.as_bytes());
        let _ = write(fd_group as i32, b":\n");
        close(fd_group as i32);
    }

    // Create home directory
    let _ = mkdir(default_home);

    println!("useradd: user '{}' added with UID {} and home '{}'", username, uid, default_home);
    0
}
