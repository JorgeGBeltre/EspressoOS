#![no_std]
#![no_main]

use libc::{arg, println, spawn, wait};

/// Lanza /bin/wifi con `argv` (NULL-terminado) y espera. El shim de nmcli mapea la
/// sintaxis de nmcli a los mismos ioctls, reusando /bin/wifi en vez de duplicarlos.
fn run_wifi(argv: &[*const u8]) -> i32 {
    let pid = spawn("/bin/wifi", argv.as_ptr());
    if pid < 0 {
        println!("nmcli: cannot run /bin/wifi");
        return 1;
    }
    let mut st = 0;
    let _ = wait(&mut st);
    0
}

/// nmcli-compatible shim (README §4). Formas soportadas:
///   nmcli device status
///   nmcli device wifi list
///   nmcli device wifi connect "SSID" [password "PASS"]
///   nmcli radio wifi on|off   (no-op: la radio siempre está)
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let a1 = if argc > 1 { unsafe { arg(argv, 1) } } else { "" };
    let a2 = if argc > 2 { unsafe { arg(argv, 2) } } else { "" };
    let a3 = if argc > 3 { unsafe { arg(argv, 3) } } else { "" };
    let null = core::ptr::null();

    match (a1, a2, a3) {
        ("device", "status", _) => {
            run_wifi(&[b"wifi\0".as_ptr(), b"status\0".as_ptr(), null])
        }
        ("device", "wifi", "list") => {
            run_wifi(&[b"wifi\0".as_ptr(), b"scan\0".as_ptr(), null])
        }
        ("device", "wifi", "connect") => {
            if argc < 5 {
                println!("usage: nmcli device wifi connect \"SSID\" [password \"PASS\"]");
                return 1;
            }
            let ssid = unsafe { arg(argv, 4) };
            // arg(5)="password", arg(6)=PASS (opcional).
            let pass = if argc >= 7 { unsafe { arg(argv, 6) } } else { "" };
            if pass.is_empty() {
                run_wifi(&[b"wifi\0".as_ptr(), b"connect\0".as_ptr(), ssid.as_ptr(), null])
            } else {
                run_wifi(&[
                    b"wifi\0".as_ptr(),
                    b"connect\0".as_ptr(),
                    ssid.as_ptr(),
                    pass.as_ptr(),
                    null,
                ])
            }
        }
        ("radio", "wifi", _) => 0, // on/off no-op: la radio siempre está encendida
        _ => {
            println!(
                "nmcli: unsupported (device status | device wifi list | device wifi connect \"SSID\" password \"PASS\")"
            );
            1
        }
    }
}
