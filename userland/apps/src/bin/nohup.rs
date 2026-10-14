#![no_std]
#![no_main]

use libc::{arg, close, dup2, open, println, signal, spawn, wait, O_APPEND, O_CREATE, O_WRONLY};

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        println!("Usage: nohup COMMAND [ARG...]");
        println!("Run COMMAND immune to hangups, with output to nohup.out");
        return 127;
    }

    let cmd = unsafe { arg(argv, 1) };
    if cmd == "--help" || cmd == "-h" {
        println!("Usage: nohup COMMAND [ARG...]");
        println!("Run COMMAND immune to hangups, with output to nohup.out");
        return 0;
    }

    // Ignore SIGHUP (signal 1)
    let _ = signal(1, 1, 0);

    // Redirect stdout/stderr to nohup.out
    let out_fd = open("nohup.out", O_CREATE | O_WRONLY | O_APPEND);
    if out_fd >= 0 {
        let _ = dup2(out_fd as i32, 1);
        let _ = dup2(out_fd as i32, 2);
        if out_fd != 1 && out_fd != 2 {
            let _ = close(out_fd as i32);
        }
    }

    let mut path_buf = [0u8; 64];
    let path = if cmd.starts_with('/') {
        cmd
    } else {
        let prefix = b"/bin/";
        let cmd_bytes = cmd.as_bytes();
        let total = prefix.len() + cmd_bytes.len();
        if total < path_buf.len() {
            path_buf[..prefix.len()].copy_from_slice(prefix);
            path_buf[prefix.len()..total].copy_from_slice(cmd_bytes);
            core::str::from_utf8(&path_buf[..total]).unwrap_or(cmd)
        } else {
            cmd
        }
    };

    let child_argv = unsafe { argv.offset(1) };
    let pid = spawn(path, child_argv);
    if pid < 0 {
        let pid2 = spawn(cmd, child_argv);
        if pid2 < 0 {
            println!("nohup: cannot run command '{}'", cmd);
            return 127;
        }
        let mut status = 0i32;
        let _ = wait(&mut status);
        return status;
    }

    let mut status = 0i32;
    let _ = wait(&mut status);
    status
}
