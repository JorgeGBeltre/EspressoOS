#![no_std]
#![no_main]

use libc::{close, open, print, println, read};

const O_RDONLY: u32 = 1;

/// smp(1): estado de multicore, leyendo `/sys/smp` (D-8). Las acciones (arrancar el
/// APP_CPU) son feature-gated y quedan en el kernel hasta SP4.
#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    let fd = open("/sys/smp", O_RDONLY);
    if fd < 0 {
        println!("smp: cannot open /sys/smp");
        return 1;
    }
    let mut buf = [0u8; 256];
    loop {
        let n = read(fd as i32, &mut buf);
        if n <= 0 {
            break;
        }
        if let Ok(s) = core::str::from_utf8(&buf[..n as usize]) {
            print!("{}", s);
        }
    }
    close(fd as i32);
    0
}
