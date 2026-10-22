#![no_std]
#![no_main]

use libc::{arg, close, open, println, spawn, unlink, wait, O_CREATE, O_EXCL, O_RDWR};

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 3 {
        println!("usage: flock <lockfile> <command> [args...]");
        println!("Manage locks from shell scripts");
        return 1;
    }

    let lockfile = unsafe { arg(argv, 1) };
    if lockfile == "-h" || lockfile == "--help" {
        println!("usage: flock <lockfile> <command> [args...]");
        println!("Manage locks from shell scripts");
        return 0;
    }

    let cmd = unsafe { arg(argv, 2) };

    // Acquire lock (exclusive create)
    let fd = open(lockfile, O_CREATE | O_EXCL | O_RDWR);
    if fd < 0 {
        println!("flock: cannot acquire lock on '{}'", lockfile);
        return 1;
    }

    // Build args for child command
    // Prepare argv array up to 8 args
    let mut child_argv = [core::ptr::null(); 9];
    let mut arg_count = 0;
    let mut i = 2;
    while i < argc && arg_count < 8 {
        let a = unsafe { arg(argv, i) };
        child_argv[arg_count] = a.as_ptr();
        arg_count += 1;
        i += 1;
    }
    child_argv[arg_count] = core::ptr::null();

    let mut cmd_buf = [0u8; 64];
    let cmd_path = if cmd.starts_with('/') {
        cmd
    } else {
        let prefix = b"/bin/";
        let mut idx = 0;
        for &b in prefix { cmd_buf[idx] = b; idx += 1; }
        for &b in cmd.as_bytes() { if idx < 63 { cmd_buf[idx] = b; idx += 1; } }
        core::str::from_utf8(&cmd_buf[..idx]).unwrap_or(cmd)
    };

    let pid = spawn(cmd_path, child_argv.as_ptr());
    let exit_code = if pid >= 0 {
        let mut status = 0i32;
        wait(&mut status);
        status
    } else {
        println!("flock: failed to execute '{}'", cmd_path);
        1
    };

    close(fd as i32);
    let _ = unlink(lockfile);

    exit_code
}
