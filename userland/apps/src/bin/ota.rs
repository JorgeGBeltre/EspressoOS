#![no_std]
#![no_main]

use libc::{arg, print, println, read, ota_state, yield_now};

const OTA_STATE_NEW: usize = 0;
const OTA_STATE_PENDING_VERIFY: usize = 1;
const OTA_STATE_VALID: usize = 2;
const OTA_STATE_INVALID: usize = 3;

fn state_str(s: isize) -> &'static str {
    match s as usize {
        OTA_STATE_NEW => "NEW (0)",
        OTA_STATE_PENDING_VERIFY => "PENDING_VERIFY (1)",
        OTA_STATE_VALID => "VALID (2)",
        OTA_STATE_INVALID => "INVALID (3)",
        4 => "ABORTED (4)",
        _ => "UNKNOWN",
    }
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc > 1 {
        let cmd = unsafe { arg(argv, 1) };
        match cmd {
            "status" => {
                let state = ota_state(0, 0);
                println!("Current image state: {}", state_str(state));
                return 0;
            }
            "mark-valid" | "valid" => {
                println!("Marking current image as VALID (stable boot)...");
                let _ = ota_state(1, OTA_STATE_VALID);
                return 0;
            }
            "rollback" | "invalidate" => {
                println!("Marking image as INVALID and forcing reboot (rollback)...");
                let _ = ota_state(1, OTA_STATE_INVALID);
                return 0;
            }
            _ => {
                println!("Usage: ota [status|mark-valid|rollback]");
                return 1;
            }
        }
    }

    println!("--- EspressoOS OTA Control Utility ---");
    println!("1. Get status of the current image");
    println!("2. Mark current image as VALID (stable boot)");
    println!("3. Mark current image as INVALID (Failure/automatic rollback)");
    print!("Select an option: ");
    
    let mut buf = [0u8; 1];
    loop {
        let n = read(0, &mut buf);
        if n > 0 {
            let c = buf[0];
            if c == b'1' {
                println!("1");
                let state = ota_state(0, 0);
                println!("Current image state: {}", state_str(state));
                break;
            } else if c == b'2' {
                println!("2");
                println!("Marking current image as VALID (stable boot)...");
                let _ = ota_state(1, OTA_STATE_VALID);
                break;
            } else if c == b'3' {
                println!("3");
                println!("Marking image as INVALID and forcing reboot (rollback)...");
                let _ = ota_state(1, OTA_STATE_INVALID);
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
