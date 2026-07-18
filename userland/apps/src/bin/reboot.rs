#![no_std]
#![no_main]

use libc::{close, ioctl, open, println};

const O_RDONLY: u32 = 1;
const POWER_REBOOT: u32 = 2;

/// reboot(1): reinicio software vía `/dev/power` + ioctl (cero syscalls nuevas, D-5). El
/// ioctl no retorna (la placa reinicia).
#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    let fd = open("/dev/power", O_RDONLY);
    if fd < 0 {
        println!("reboot: cannot open /dev/power");
        return 1;
    }
    println!("Rebooting...");
    ioctl(fd as i32, POWER_REBOOT, 0);
    close(fd as i32); // inalcanzable si el reboot ocurre
    1
}
