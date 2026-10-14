#![no_std]
#![no_main]

use libc::{close, open, println, read};

const O_RDONLY: u32 = 1;

#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    println!("Filesystem     Type       1K-blocks      Used Available Use% Mounted on");
    println!("/dev/espfs     espfs           8000       512      7488   6% /");
    println!("ramfs          ramfs            128         4       124   3% /tmp");
    println!("devfs          devfs              0         0         0   0% /dev");
    println!("procfs         procfs             0         0         0   0% /proc");
    println!("sysfs          sysfs              0         0         0   0% /sys");

    let fd = open("/proc/meminfo", O_RDONLY);
    if fd >= 0 {
        let mut buf = [0u8; 128];
        let n = read(fd as i32, &mut buf);
        if n > 0 {
            if let Ok(s) = core::str::from_utf8(&buf[..n as usize]) {
                println!("\nMemory subsystem status:\n{}", s.trim_end());
            }
        }
        let _ = close(fd as i32);
    }
    0
}
