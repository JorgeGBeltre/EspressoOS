#![no_std]
#![no_main]

use libc::{println, print, read, ota_state, yield_now};

#[no_mangle]
pub extern "C" fn main() -> i32 {
    println!("--- EspressoOS OTA Control Utility ---");
    println!("1. Obtener estado de la imagen actual");
    println!("2. Marcar imagen actual como INVALIDA (Falla/Rollback automatico)");
    print!("Seleccione una opcion: ");
    
    let mut buf = [0u8; 1];
    loop {
        let n = read(0, &mut buf);
        if n > 0 {
            let c = buf[0];
            if c == b'1' {
                println!("1");
                let state = ota_state(0, 0);
                println!("Estado actual de la imagen (otadata.ota_state): {}", state);
                break;
            } else if c == b'2' {
                println!("2");
                println!("Marcando imagen como INVALIDA y forzando reinicio (rollback)...");
                let _ = ota_state(1, 3); // OtaImgState::Invalid = 3
                break;
            } else if c == b'\n' || c == b'\r' {
                // ignorar
            } else {
                println!("Opcion invalida");
                break;
            }
        } else {
            yield_now();
        }
    }
    0
}
