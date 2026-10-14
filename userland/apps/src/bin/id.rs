#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read, O_RDONLY};

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut print_u_only = false;
    let mut print_g_only = false;
    let mut print_name_only = false;

    if argc > 1 {
        let flag = unsafe { arg(argv, 1) };
        if flag == "-u" {
            print_u_only = true;
        } else if flag == "-g" {
            print_g_only = true;
        } else if flag == "-un" {
            print_u_only = true;
            print_name_only = true;
        } else if flag == "-gn" {
            print_g_only = true;
            print_name_only = true;
        }
    }

    let mut user_name = "youareme";
    let mut name_buf = [0u8; 32];
    let mut has_custom_user = false;

    let fd = open("/etc/passwd", O_RDONLY);
    if fd >= 0 {
        let mut buf = [0u8; 128];
        let n = read(fd as i32, &mut buf);
        close(fd as i32);
        if n > 0 {
            if let Ok(s) = core::str::from_utf8(&buf[..n as usize]) {
                if let Some(first_line) = s.split('\n').next() {
                    if let Some(u) = first_line.split(':').next() {
                        let trimmed = u.trim();
                        if !trimmed.is_empty() {
                            let len = trimmed.len().min(31);
                            name_buf[..len].copy_from_slice(&trimmed.as_bytes()[..len]);
                            name_buf[len] = 0;
                            has_custom_user = true;
                        }
                    }
                }
            }
        }
    }

    if has_custom_user {
        let len = name_buf.iter().position(|&c| c == 0).unwrap_or(name_buf.len());
        if let Ok(s) = core::str::from_utf8(&name_buf[..len]) {
            user_name = s;
        }
    }

    let is_root = user_name == "root";
    let uid = if is_root { 0 } else { 1000 };
    let gid = if is_root { 0 } else { 1000 };
    let grp_name = if is_root { "root" } else { "users" };

    if print_u_only {
        if print_name_only {
            println!("{}", user_name);
        } else {
            println!("{}", uid);
        }
        return 0;
    }

    if print_g_only {
        if print_name_only {
            println!("{}", grp_name);
        } else {
            println!("{}", gid);
        }
        return 0;
    }

    println!(
        "uid={}({}) gid={}({}) groups={}({})",
        uid, user_name, gid, grp_name, gid, grp_name
    );
    0
}
