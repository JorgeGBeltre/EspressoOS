#![no_std]
#![no_main]

use libc::{arg, close, ioctl, open, println};

const O_RDONLY: u32 = 1;
const SET_PASSWORD_CMD: u32 = 0;

// Debe coincidir con config::DEV_USER del kernel (drivers/ssh/config.rs).
const DEFAULT_USER: &str = "youareme";

/// Espejo del struct del kernel (drivers::passwd::PasswdReq).
#[repr(C)]
struct PasswdReq {
    user_ptr: usize,
    user_len: usize,
    pass_ptr: usize,
    pass_len: usize,
}

/// passwd(1). Fija la credencial SSH vía `/dev/passwd` + ioctl. El kernel sala y hashea
/// (SHA-256+salt, `$s5$...`) ANTES de escribir `/etc/passwd` -- el password nunca llega a
/// flash en texto plano (SP2 R6). `drivers/ssh/auth.rs` consulta `/etc/passwd` ANTES que el
/// DEV_USER/DEV_PASSWORD compilado, así que esto cambia el login SIN recompilar y PERSISTE
/// en EspFs (sobrevive reinicios y reflasheo). `rm /etc/passwd` revierte a la credencial
/// compilada.
///
///   passwd NEWPASS         -> user "youareme"
///   passwd USER NEWPASS
///
/// Nota: el password sigue viajando en argv (visible en la línea de comandos y en el
/// historial del shell) -- eso es un límite de cómo se invoca, no del almacenamiento.
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

    let fd = open("/dev/passwd", O_RDONLY);
    if fd < 0 {
        println!("passwd: cannot open /dev/passwd");
        return 1;
    }
    let fd = fd as i32;

    let req = PasswdReq {
        user_ptr: user.as_ptr() as usize,
        user_len: user.len(),
        pass_ptr: pass.as_ptr() as usize,
        pass_len: pass.len(),
    };
    let rc = ioctl(fd, SET_PASSWORD_CMD, &req as *const PasswdReq as usize);
    let _ = close(fd);

    if rc >= 0 {
        println!("passwd: SSH password updated for '{}' (salted+hashed in /etc/passwd)", user);
        0
    } else {
        println!("passwd: failed ({})", rc);
        1
    }
}
