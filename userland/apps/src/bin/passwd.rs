#![no_std]
#![no_main]

use libc::{arg, close, open, println, write};

// WRONLY|CREATE|TRUNC (mismo patrón que /bin/write): reescribe el fichero entero.
const O_WRONLY_CREATE_TRUNC: u32 = 0x0002 | 0x0100 | 0x0400;

// Debe coincidir con config::DEV_USER del kernel (drivers/ssh/config.rs).
const DEFAULT_USER: &str = "youareme";

/// passwd(1). Fija la credencial SSH escribiendo `/etc/passwd` (`user:pass`, texto
/// plano). `drivers/ssh/auth.rs` consulta `/etc/passwd` ANTES que el DEV_USER/DEV_PASSWORD
/// compilado, así que esto cambia el login SIN recompilar y PERSISTE en EspFs (sobrevive
/// reinicios y reflasheo). `rm /etc/passwd` revierte a la credencial compilada.
///
///   passwd NEWPASS         -> "youareme:NEWPASS"
///   passwd USER NEWPASS    -> "USER:NEWPASS"
///
/// Nota: reescribe el fichero con UNA sola línea (un usuario). El password va en argv
/// (visible en la línea de comandos) y se guarda en plano — consistente con el auth
/// actual, apto para placa de desarrollo, no para producción.
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let (user, pass) = match argc {
        2 => (DEFAULT_USER, unsafe { arg(argv, 1) }),
        3 => (unsafe { arg(argv, 1) }, unsafe { arg(argv, 2) }),
        _ => {
            println!("usage: passwd [USER] NEWPASSWORD");
            return 1;
        }
    };
    if pass.is_empty() || pass.contains(':') || pass.contains('\n') {
        println!("passwd: password must be non-empty and contain no ':' or newline");
        return 1;
    }
    if user.is_empty() || user.contains(':') || user.contains('\n') {
        println!("passwd: invalid user name");
        return 1;
    }

    let fd = open("/etc/passwd", O_WRONLY_CREATE_TRUNC);
    if fd < 0 {
        println!("passwd: cannot open /etc/passwd");
        return 1;
    }
    let fd = fd as i32;

    let ok = write(fd, user.as_bytes()) >= 0
        && write(fd, b":") >= 0
        && write(fd, pass.as_bytes()) >= 0
        && write(fd, b"\n") >= 0;
    let _ = close(fd);

    if ok {
        println!("passwd: SSH password updated for '{}' (plaintext in /etc/passwd)", user);
        0
    } else {
        println!("passwd: write error");
        1
    }
}
