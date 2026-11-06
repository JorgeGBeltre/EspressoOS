#![no_std]
#![no_main]

use libc::{arg, chdir, chroot, println, spawn, waitpid};

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        println!("Usage: chroot NEWROOT [COMMAND [ARG...]]");
        println!("Run COMMAND with root directory set to NEWROOT");
        return 125;
    }

    let newroot = unsafe { arg(argv, 1) };
    if newroot == "--help" || newroot == "-h" {
        println!("Usage: chroot NEWROOT [COMMAND [ARG...]]");
        println!("Run COMMAND with root directory set to NEWROOT");
        return 0;
    }

    let rc = chroot(newroot);
    if rc < 0 {
        let reason = match rc {
            -1 => "Operation not permitted",
            -20 => "Not a directory",
            _ => "No such file or directory",
        };
        println!("chroot: cannot change root directory to '{}': {}", newroot, reason);
        return 125;
    }
    let _ = chdir("/");

    let default_argv = [b"sh\0".as_ptr(), core::ptr::null()];
    let (cmd, child_argv) = if argc >= 3 {
        let c = unsafe { arg(argv, 2) };
        (c, unsafe { argv.offset(2) })
    } else {
        ("/bin/sh", default_argv.as_ptr())
    };

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

    let mut pid = spawn(path, child_argv);
    if pid < 0 {
        pid = spawn(cmd, child_argv);
    }
    if pid < 0 {
        println!("chroot: failed to run command '{}': No such file or directory", cmd);
        return 127;
    }

    let mut status = 0i32;
    let _ = waitpid(pid as i32, &mut status, 0);
    status
}
