#![no_std]
#![no_main]

use libc::{println, print, read, ota_state, yield_now};

#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    println!("--- EspressoOS OTA Control Utility ---");
    println!("1. Get status of the current image");
    println!("2. Mark current image as INVALID (Failure/automatic rollback)");
    print!("Select an option: ");
    
    let mut buf = [0u8; 1];
    loop {
        let n = read(0, &mut buf);
        if n > 0 {
            let c = buf[0];
            if c == b'1' {
                println!("1");
                let state = ota_state(0, 0);
                println!("Current image state (otadata.ota_state): {}", state);
                break;
            } else if c == b'2' {
                println!("2");
                println!("Marking image as INVALID and forcing reboot (rollback)...");
                let _ = ota_state(1, 3);
                break;
            } else if c == b'\n' || c == b'\r' {

            } else {
                println!("Invalid option");
                break;
            }
        } else {
            yield_now();
        }
    }
    0
}
