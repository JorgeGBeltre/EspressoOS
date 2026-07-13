#![no_std]
#![no_main]

use libc::println;

#[no_mangle]
pub extern "C" fn main() -> i32 {
    println!("Hola desde EspressoOS Userland!");
    0
}
