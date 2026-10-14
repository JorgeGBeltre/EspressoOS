#![no_std]
#![no_main]

use libc::yield_now;

#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    // Los sistemas de archivos VFS / EspFs escriben síncronamente en flash;
    // yield asegura que cualquier tarea de fondo drene sus buffers pendientes.
    for _ in 0..10 {
        yield_now();
    }
    0
}
