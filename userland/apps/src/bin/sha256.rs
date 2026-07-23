#![no_std]
#![no_main]

use libc::{arg, close, ioctl, open, print, println, read};

const O_RDONLY: u32 = 1;
const SHA256_CMD: u32 = 0;

/// Espejo del struct del kernel (drivers::crypto::ShaReq, D-1).
#[repr(C)]
struct ShaReq {
    in_ptr: usize,
    in_len: usize,
    out_ptr: usize,
}

/// sha256(1): hash SHA-256 por hardware, vía `/dev/sha0` + ioctl. `sha256 TEXT` hashea el
/// texto (formato igual al builtin: `SHA256("TEXT") = ...`); sin argumento lee stdin (útil en
/// pipeline). El diferencial `sha256 hello` == hash público conocido prueba el camino de datos
/// de crypto de verdad (no como el `00 00 00` de un bus vacío).
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut input = [0u8; 4096];
    let mut ilen = 0usize;
    let text = if argc >= 2 {
        let t = unsafe { arg(argv, 1) };
        let b = t.as_bytes();
        ilen = core::cmp::min(b.len(), input.len());
        input[..ilen].copy_from_slice(&b[..ilen]);
        Some(t)
    } else {
        // stdin (hasta 512 bytes).
        while ilen < input.len() {
            let n = read(0, &mut input[ilen..]);
            if n <= 0 {
                break;
            }
            ilen += n as usize;
        }
        None
    };

    let fd = open("/dev/sha0", O_RDONLY);
    if fd < 0 {
        println!("sha256: cannot open /dev/sha0");
        return 1;
    }
    let mut out = [0u8; 32];
    let req = ShaReq {
        in_ptr: input.as_ptr() as usize,
        in_len: ilen,
        out_ptr: out.as_ptr() as usize,
    };
    let rc = ioctl(fd as i32, SHA256_CMD, &req as *const ShaReq as usize);
    close(fd as i32);
    if rc < 0 {
        println!("sha256: failed ({})", rc);
        return 1;
    }

    if let Some(t) = text {
        print!("SHA256(\"{}\") = ", t);
    }
    for b in &out {
        print!("{:02x}", b);
    }
    println!("");
    0
}
