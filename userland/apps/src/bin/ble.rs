#![no_std]
#![no_main]

use libc::{arg, close, ioctl, open, print, println, read};

const O_RDONLY: u32 = 1;
const BLE_ADVERTISE: u32 = 0;
const BLE_ADVERTISE_SYNC_DIAG: u32 = 0xD1A6; // DIAGNÓSTICO temporal (experimento de pila)

/// ble(1): `ble status` (lee `/dev/ble0`, D-3) o `ble advertise` (ioctl → anuncia como
/// 'EspressoOS'). Las acciones por ioctl, el estado por read.
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        println!("usage: ble status | advertise");
        return 1;
    }
    let fd = open("/dev/ble0", O_RDONLY);
    if fd < 0 {
        println!("ble: cannot open /dev/ble0");
        return 1;
    }
    let fd = fd as i32;
    let r = match unsafe { arg(argv, 1) } {
        "status" => {
            let mut buf = [0u8; 128];
            let n = read(fd, &mut buf);
            if n > 0 {
                if let Ok(s) = core::str::from_utf8(&buf[..n as usize]) {
                    print!("{}", s);
                }
            }
            0
        }
        "advertise" => {
            // El ioctl ENCOLA (D-4); el net_task ejecuta las escrituras HCI. No bloquea.
            let rc = ioctl(fd, BLE_ADVERTISE, 0);
            if rc < 0 {
                println!("ble advertise: failed ({})", rc);
                1
            } else {
                println!("BLE: advertise requested (check with 'ble status')");
                0
            }
        }
        "advertise-sync" => {
            // DIAGNÓSTICO: fuerza el camino SÍNCRONO (el que colgaba) en la pila de este
            // proceso userland (16K) para medir el pico. QUITAR tras cerrar el experimento.
            let rc = ioctl(fd, BLE_ADVERTISE_SYNC_DIAG, 0);
            println!("ble advertise-sync: rc={}", rc);
            if rc < 0 {
                1
            } else {
                0
            }
        }
        _ => {
            println!("usage: ble status | advertise");
            1
        }
    };
    close(fd);
    r
}
