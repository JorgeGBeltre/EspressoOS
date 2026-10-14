#![no_std]
#![no_main]

use libc::{println, uptime_ms};

#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    let ms = uptime_ms();
    let sec = ms / 1000;
    let min = sec / 60;
    let hours = min / 60;
    let days = hours / 24;
    println!("Uptime: {}d {:02}:{:02}:{:02} ({} ms total)", days, hours % 24, min % 60, sec % 60, ms);
    0
}
