#![no_std]
#![no_main]

use libc::{close, open, print, read};

const O_RDONLY: u32 = 1;

/// ps(1): lee `/proc/tasks` (D-8) y lo imprime. A diferencia del builtin del kernel (que
/// solo mostraba la task actual), enumera TODAS las tasks: tid/name/state/used/size/free.
#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    let fd = open("/proc/tasks", O_RDONLY);
    if fd < 0 {
        libc::println!("ps: cannot open /proc/tasks");
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
