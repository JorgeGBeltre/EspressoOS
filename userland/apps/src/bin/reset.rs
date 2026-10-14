#![no_std]
#![no_main]

use libc::{arg, println, write};

const STDOUT: i32 = 1;
// ANSI sequence: soft reset, show cursor, default attributes, clear screen, cursor home
const RESET_SEQ: &[u8] = b"\x1b[!p\x1b[?25h\x1b[0m\x1b[2J\x1b[H";

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc > 1 {
        let a = unsafe { arg(argv, 1) };
        if a == "-h" || a == "--help" {
            println!("Usage: reset");
            println!("Reset terminal mode and clear screen attributes");
            return 0;
        }
    }

    let _ = write(STDOUT, RESET_SEQ);
    0
}
