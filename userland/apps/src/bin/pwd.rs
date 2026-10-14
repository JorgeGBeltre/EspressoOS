#![no_std]
#![no_main]

use libc::{getcwd_str, println, PATH_MAX};

#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    let mut buf = [0u8; PATH_MAX];
    match getcwd_str(&mut buf) {
        Ok(cwd) => {
            println!("{}", cwd);
            0
        }
        Err(e) => {
            println!("pwd: error {}", e);
            1
        }
    }
}
