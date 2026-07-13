#![no_std]
#![no_main]

use libc::{println, readdir};

#[no_mangle]
pub extern "C" fn main() -> i32 {
    let mut buf = [0u8; 1024];
    let n = readdir("/", &mut buf);
    if n < 0 {
        println!("ls: error al leer directorio");
        return 1;
    }
    
    let mut pos = 0;
    let limit = n as usize;
    while pos < limit {
        if pos + 11 > limit {
            break;
        }
        let _ino = u64::from_le_bytes([
            buf[pos], buf[pos+1], buf[pos+2], buf[pos+3],
            buf[pos+4], buf[pos+5], buf[pos+6], buf[pos+7]
        ]);
        let _kind = buf[pos+8];
        let name_len = u16::from_le_bytes([buf[pos+9], buf[pos+10]]) as usize;
        pos += 11;
        
        if pos + name_len > limit {
            break;
        }
        if let Ok(name) = core::str::from_utf8(&buf[pos..pos+name_len]) {
            println!("{}", name);
        }
        pos += name_len;
    }
    0
}
