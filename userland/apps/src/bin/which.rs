#![no_std]
#![no_main]

use libc::{arg, close, open, println};

const O_RDONLY: u32 = 1;

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        println!("usage: which <program>");
        return 1;
    }
    let prog = unsafe { arg(argv, 1) };
    let mut path_buf = [0u8; 64];
    let prefix = b"/bin/";
    path_buf[..prefix.len()].copy_from_slice(prefix);
    let pbytes = prog.as_bytes();
    if prefix.len() + pbytes.len() >= path_buf.len() {
        return 1;
    }
    path_buf[prefix.len()..prefix.len() + pbytes.len()].copy_from_slice(pbytes);
    let full = match core::str::from_utf8(&path_buf[..prefix.len() + pbytes.len()]) {
        Ok(s) => s,
        Err(_) => return 1,
    };

    let fd = open(full, O_RDONLY);
    if fd >= 0 {
        close(fd as i32);
        println!("{}", full);
        0
    } else {
        println!("which: no {} in (/bin)", prog);
        1
    }
}
