#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read};

const O_RDONLY: u32 = 1;

/// pms(1): estado de la protección de memoria (PMS), leyendo `/sys/pms` (D-8). La ACCIÓN
/// `pms world1` (aplicar la política W^X, feature-gated) sigue en el shell del kernel hasta
/// SP4; aquí solo se lee el estado.
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc > 1 && unsafe { arg(argv, 1) } == "world1" {
        println!("pms: la accion 'world1' esta en el shell del kernel (feature-gated); aqui solo estado");
    }
    let fd = open("/sys/pms", O_RDONLY);
    if fd < 0 {
        println!("pms: cannot open /sys/pms");
        return 1;
    }
    let mut buf = [0u8; 256];
    loop {
        let n = read(fd as i32, &mut buf);
        if n <= 0 {
            break;
        }
        if let Ok(s) = core::str::from_utf8(&buf[..n as usize]) {
            print!("{}", s);
        }
    }
    close(fd as i32);
    0
}
