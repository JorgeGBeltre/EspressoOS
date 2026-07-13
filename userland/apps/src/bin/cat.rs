#![no_std]
#![no_main]

use libc::{open, read, write, close, println};

#[no_mangle]
pub extern "C" fn main() -> i32 {
    let fd = open("/etc/hosts", 1); // RDONLY = 1
    if fd < 0 {
        println!("cat: no se pudo abrir /etc/hosts");
        return 1;
    }
    
    let mut buf = [0u8; 128];
    loop {
        let n = read(fd as i32, &mut buf);
        if n < 0 {
            println!("cat: error al leer");
            let _ = close(fd as i32);
            return 1;
        }
        if n == 0 {
            break;
        }
        let _ = write(1, &buf[0..n as usize]);
    }
    let _ = close(fd as i32);
    0
}
