#![no_std]
#![no_main]

use libc::{arg, close, open, println, write};

// WRONLY|CREATE|TRUNC: igual que el builtin del kernel (trunca al escribir).
const O_WRONLY_CREATE_TRUNC: u32 = 0x0002 | 0x0100 | 0x0400;

/// write(1). Escribe TEXT... (unido por espacios, con `\n` final) en FILE, truncando.
/// Paridad byte a byte con el builtin `write FILE TEXT...` del kernel — importante para
/// editar `/etc/rc.local` desde la consola de serie (cierra media trampa circular).
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 3 {
        println!("usage: write FILE TEXT...");
        return 1;
    }
    let path = unsafe { arg(argv, 1) };
    let fd = open(path, O_WRONLY_CREATE_TRUNC);
    if fd < 0 {
        println!("write: {}: cannot open", path);
        return 1;
    }
    let fd = fd as i32;

    let mut ok = true;
    for i in 2..argc {
        if i > 2 && write(fd, b" ") < 0 {
            ok = false;
            break;
        }
        let word = unsafe { arg(argv, i) };
        if write(fd, word.as_bytes()) < 0 {
            ok = false;
            break;
        }
    }
    if ok && write(fd, b"\n") < 0 {
        ok = false;
    }
    let _ = close(fd);

    if ok {
        0
    } else {
        println!("write: {}: write error", path);
        1
    }
}
