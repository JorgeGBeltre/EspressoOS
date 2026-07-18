#![no_std]
#![no_main]

use libc::{arg, close, ioctl, open, println};

const O_RDONLY: u32 = 1;
const POWER_SLEEP: u32 = 0;
const POWER_DEEP_SLEEP: u32 = 1;

fn parse_dec(s: &str) -> Option<u64> {
    let mut v = 0u64;
    let mut any = false;
    for b in s.bytes() {
        if b.is_ascii_digit() {
            v = v * 10 + (b - b'0') as u64;
            any = true;
        } else {
            return None;
        }
    }
    if any {
        Some(v)
    } else {
        None
    }
}

/// power(1): `power sleep [SECONDS]` (light sleep, vuelve) o `power deep-sleep [SECONDS]`
/// (rebootea al despertar). Vía `/dev/power` + ioctl (cero syscalls nuevas, D-5).
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        println!("usage: power sleep [SECONDS] | deep-sleep [SECONDS]");
        return 1;
    }
    let cmd = match unsafe { arg(argv, 1) } {
        "sleep" => POWER_SLEEP,
        "deep-sleep" => POWER_DEEP_SLEEP,
        _ => {
            println!("usage: power sleep [SECONDS] | deep-sleep [SECONDS]");
            return 1;
        }
    };
    let secs = if argc >= 3 {
        parse_dec(unsafe { arg(argv, 2) }).unwrap_or(5)
    } else {
        5
    };

    let fd = open("/dev/power", O_RDONLY);
    if fd < 0 {
        println!("power: cannot open /dev/power");
        return 1;
    }
    // deep-sleep no retorna (la placa reinicia); light sleep vuelve tras SECONDS.
    let rc = ioctl(fd as i32, cmd, secs as usize);
    close(fd as i32);
    if rc < 0 {
        println!("power: failed ({})", rc);
        return 1;
    }
    0
}
