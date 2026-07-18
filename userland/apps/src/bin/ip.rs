#![no_std]
#![no_main]

use libc::{close, open, println, read};

const O_RDONLY: u32 = 1;

/// Valor del campo `prefix:` en el snapshot de /dev/wlan0 (recortado), o "".
fn field<'a>(text: &'a str, prefix: &str) -> &'a str {
    for line in text.split('\n') {
        if let Some(rest) = line.strip_prefix(prefix) {
            return rest.trim();
        }
    }
    ""
}

/// ip(1) mínimo: muestra la dirección de wlan0, SSID y estado, leyendo el snapshot de
/// /dev/wlan0 (D-3: estado por read, no por ioctl).
#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    let fd = open("/dev/wlan0", O_RDONLY);
    if fd < 0 {
        println!("ip: cannot open /dev/wlan0");
        return 1;
    }
    let mut buf = [0u8; 256];
    let mut total = 0usize;
    while total < buf.len() {
        let n = read(fd as i32, &mut buf[total..]);
        if n <= 0 {
            break;
        }
        total += n as usize;
    }
    close(fd as i32);

    let text = core::str::from_utf8(&buf[..total]).unwrap_or("");
    println!(
        "wlan0: {}  ssid \"{}\"  state {}",
        field(text, "ip:"),
        field(text, "ssid:"),
        field(text, "state:")
    );
    0
}
