#![no_std]
#![no_main]

use libc::{arg, close, open, println};

// WRONLY|CREATE (sin TRUNC): crea si falta, no destruye el contenido si ya existe.
const O_WRONLY_CREATE: u32 = 0x0002 | 0x0100;

/// touch(1). Crea ficheros vacíos (o los deja intactos si ya existen). Paridad con el
/// builtin `touch FILE...`.
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc <= 1 {
        println!("usage: touch FILE...");
        return 1;
    }
    let mut status = 0;
    for i in 1..argc {
        let path = unsafe { arg(argv, i) };
        let fd = open(path, O_WRONLY_CREATE);
        if fd < 0 {
            println!("touch: {}: cannot create", path);
            status = 1;
        } else {
            let _ = close(fd as i32);
        }
    }
    status
}
