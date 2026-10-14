#![no_std]
#![no_main]

use libc::{arg, println, spawn, wait};

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let target_user = if argc > 1 {
        unsafe { arg(argv, 1) }
    } else {
        "root"
    };

    println!("[su] Switching session to user '{}'...", target_user);

    let mut user_buf = [0u8; 32];
    let ulen = target_user.len().min(31);
    user_buf[..ulen].copy_from_slice(&target_user.as_bytes()[..ulen]);
    user_buf[ulen] = 0;

    let child_argv = [
        b"sh\0".as_ptr(),
        b"--user\0".as_ptr(),
        user_buf.as_ptr(),
        core::ptr::null(),
    ];

    let pid = spawn("/bin/sh", child_argv.as_ptr());
    if pid >= 0 {
        let mut status = 0;
        let _ = wait(&mut status);
        status
    } else {
        println!("su: failed to spawn shell: code {}", pid);
        1
    }
}
