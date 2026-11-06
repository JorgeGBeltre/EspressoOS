#![no_std]
#![no_main]

use libc::{
    arg, close, flock, open, println, spawn, waitpid, LOCK_EX, LOCK_NB, LOCK_SH, LOCK_UN,
    O_CREATE, O_RDWR,
};

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut shared = false;
    let mut nonblock = false;
    let mut unlock = false;
    let mut idx = 1;

    while idx < argc {
        let a = unsafe { arg(argv, idx) };
        if a == "-h" || a == "--help" {
            println!("Usage: flock [-s|-x|-u] [-n] <lockfile> <command> [args...]");
            return 0;
        } else if a == "-s" || a == "--shared" {
            shared = true;
            idx += 1;
        } else if a == "-x" || a == "--exclusive" {
            shared = false;
            idx += 1;
        } else if a == "-n" || a == "--nonblock" {
            nonblock = true;
            idx += 1;
        } else if a == "-u" || a == "--unlock" {
            unlock = true;
            idx += 1;
        } else {
            break;
        }
    }

    if idx >= argc {
        println!("Usage: flock [-s|-x|-u] [-n] <lockfile> <command> [args...]");
        return 1;
    }

    let lockfile = unsafe { arg(argv, idx) };
    idx += 1;

    let fd = open(lockfile, O_CREATE | O_RDWR);
    if fd < 0 {
        println!("flock: cannot open '{}'", lockfile);
        return 1;
    }

    let mut op = if unlock {
        LOCK_UN
    } else if shared {
        LOCK_SH
    } else {
        LOCK_EX
    };
    if nonblock && !unlock {
        op |= LOCK_NB;
    }

    if flock(fd as i32, op) < 0 {
        close(fd as i32);
        println!("flock: failed to acquire lock on '{}'", lockfile);
        return 1;
    }

    if idx >= argc {
        close(fd as i32);
        return 0;
    }

    let cmd = unsafe { arg(argv, idx) };
    let mut child_argv = [core::ptr::null(); 12];
    let mut arg_count = 0;
    while idx < argc && arg_count < 11 {
        child_argv[arg_count] = unsafe { arg(argv, idx) }.as_ptr();
        arg_count += 1;
        idx += 1;
    }
    child_argv[arg_count] = core::ptr::null();

    let mut cmd_buf = [0u8; 64];
    let cmd_path = if cmd.contains('/') {
        cmd
    } else {
        let prefix = b"/bin/";
        let mut k = 0;
        for &b in prefix {
            cmd_buf[k] = b;
            k += 1;
        }
        for &b in cmd.as_bytes() {
            if k < 63 {
                cmd_buf[k] = b;
                k += 1;
            }
        }
        core::str::from_utf8(&cmd_buf[..k]).unwrap_or(cmd)
    };

    let pid = spawn(cmd_path, child_argv.as_ptr());
    let exit_code = if pid >= 0 {
        let mut status = 0i32;
        let _ = waitpid(pid as i32, &mut status, 0);
        status
    } else {
        println!("flock: failed to execute '{}'", cmd_path);
        127
    };

    let _ = flock(fd as i32, LOCK_UN);
    close(fd as i32);
    exit_code
}
