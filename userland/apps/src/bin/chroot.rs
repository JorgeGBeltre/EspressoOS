#![no_std]
#![no_main]

use libc::{arg, chdir, println, spawn, waitpid};

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

    if chdir(newroot) < 0 {
        println!("chroot: cannot change root directory to '{}': No such file or directory", newroot);
        return 125;
    }

    let (cmd, child_argv) = if argc >= 3 {
        let c = unsafe { arg(argv, 2) };
        (c, unsafe { argv.offset(2) })
    } else {
        ("/bin/sh", core::ptr::null())
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
