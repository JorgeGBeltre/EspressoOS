#![no_std]
#![no_main]

use libc::{arg, print, println, spawn, uptime_ms, waitpid};

fn print_dec(mut n: usize) {
    if n == 0 {
        print!("0");
        return;
    }
    let mut buf = [0u8; 16];
    let mut i = 0;
    while n > 0 {
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        i += 1;
    }
    while i > 0 {
        i -= 1;
        let s = core::slice::from_ref(&buf[i]);
        if let Ok(c) = core::str::from_utf8(s) {
            print!("{}", c);
        }
    }
}

fn print_ms(ms: usize) {
    let sec = ms / 1000;
    let rem = ms % 1000;
    print_dec(sec);
    print!(".");
    if rem < 100 {
        print!("0");
    }
    if rem < 10 {
        print!("0");
    }
    print_dec(rem);
    print!("s\n");
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        println!("Usage: time COMMAND [ARG...]");
        println!("Run COMMAND and report real execution time in seconds");
        return 1;
    }

    let cmd = unsafe { arg(argv, 1) };
    if cmd == "--help" || cmd == "-h" {
        println!("Usage: time COMMAND [ARG...]");
        println!("Run COMMAND and report real execution time in seconds");
        return 0;
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
    let t0 = uptime_ms();
    let mut pid = spawn(path, child_argv);
    if pid < 0 {
        pid = spawn(cmd, child_argv);
    }
    if pid < 0 {
        println!("time: cannot run '{}': command not found", cmd);
        return 127;
    }

    let mut status = 0i32;
    let _ = waitpid(pid as i32, &mut status, 0);
    let t1 = uptime_ms();
    let dt = t1.wrapping_sub(t0);

    print!("\nreal\t");
    print_ms(dt);
    println!("user\t0.000s\nsys\t0.000s");

    status
}
