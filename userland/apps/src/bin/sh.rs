#![no_std]
#![no_main]

use libc::{println, print, read, spawn, wait, yield_now};

#[no_mangle]
pub extern "C" fn main() -> i32 {
    println!("--- EspressoOS Shell (Userland) ---");
    let mut buf = [0u8; 64];
    loop {
        print!("$ ");
        let mut len = 0;
        loop {
            let mut char_buf = [0u8; 1];
            let n = read(0, &mut char_buf);
            if n > 0 {
                let c = char_buf[0];
                if c == b'\n' || c == b'\r' {
                    println!("");
                    break;
                }
                let _ = libc::write(1, &char_buf);
                if len < buf.len() - 1 {
                    buf[len] = c;
                    len += 1;
                }
            } else {
                yield_now();
            }
        }
        
        if len == 0 {
            continue;
        }
        
        if let Ok(cmd) = core::str::from_utf8(&buf[0..len]) {
            let cmd = cmd.trim();
            if cmd == "exit" {
                println!("Saliendo de la shell...");
                break;
            }
            
            let mut path_buf = [0u8; 64];
            let path = if cmd.starts_with('/') {
                cmd
            } else {
                let prefix = b"/bin/";
                let cmd_bytes = cmd.as_bytes();
                if prefix.len() + cmd_bytes.len() < path_buf.len() {
                    path_buf[..prefix.len()].copy_from_slice(prefix);
                    path_buf[prefix.len()..prefix.len() + cmd_bytes.len()].copy_from_slice(cmd_bytes);
                    core::str::from_utf8(&path_buf[..prefix.len() + cmd_bytes.len()]).unwrap_or("")
                } else {
                    ""
                }
            };
            
            if !path.is_empty() {
                let pid = spawn(path, 0, 0, 0, 0);
                if pid >= 0 {
                    let mut status = 0;
                    let _ = wait(&mut status);
                } else {
                    println!("Error al ejecutar: {}", cmd);
                }
            }
        }
    }
    0
}
