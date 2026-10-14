#![no_std]
#![no_main]

use libc::{getcwd_str, println, PATH_MAX};

#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    let mut cwd_buf = [0u8; PATH_MAX];
    let cwd = getcwd_str(&mut cwd_buf).unwrap_or("/");

    println!("OS=EspressoOS");
    println!("VERSION=0.2.0-dev");
    println!("ARCH=xtensa-lx7");
    println!("BOARD=ESP32-S3-WROOM-1-N16R8");
    println!("SHELL=/bin/sh");
    println!("USER=youareme");
    print_var("PWD", cwd);
    println!("PATH=/bin");
    println!("TERM=xterm-256color");
    0
}

fn print_var(k: &str, v: &str) {
    libc::print!("{}", k);
    libc::print!("=");
    libc::println!("{}", v);
}
