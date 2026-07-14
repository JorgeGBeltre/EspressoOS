#![no_std]
#![no_main]

use libc::{println, print, read, spawn, wait, yield_now, pipe, dup2, close};

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
        
        if let Ok(cmd_str) = core::str::from_utf8(&buf[0..len]) {
            let cmd_str = cmd_str.trim();
            if cmd_str == "exit" {
                println!("Saliendo de la shell...");
                break;
            }
            
            if let Some(pipe_idx) = cmd_str.find('|') {
                // Comando con tubería
                let left_cmd = cmd_str[..pipe_idx].trim();
                let right_cmd = cmd_str[pipe_idx + 1..].trim();
                
                let mut p = [0i32; 2];
                if pipe(&mut p) < 0 {
                    println!("Error al crear pipe");
                    continue;
                }
                
                let saved_stdout = dup2(1, 10);
                let saved_stdin = dup2(0, 11);
                
                let mut path_buf1 = [0u8; 64];
                let path1 = resolve_path(left_cmd, &mut path_buf1);
                
                let mut path_buf2 = [0u8; 64];
                let path2 = resolve_path(right_cmd, &mut path_buf2);
                
                if path1.is_empty() || path2.is_empty() {
                    println!("Ruta de comando vacía o inválida");
                    close(p[0]);
                    close(p[1]);
                    close(10);
                    close(11);
                    continue;
                }
                
                // Ejecutar izquierdo (redireccionando su stdout a la escritura del pipe)
                dup2(p[1], 1);
                let pid1 = spawn(path1, 0, 0, 0, 0);
                
                // Ejecutar derecho (redireccionando su stdin a la lectura del pipe y restaurando stdout)
                dup2(p[0], 0);
                dup2(saved_stdout as i32, 1);
                let pid2 = spawn(path2, 0, 0, 0, 0);
                
                // Restaurar stdin
                dup2(saved_stdin as i32, 0);
                
                close(p[0]);
                close(p[1]);
                close(10);
                close(11);
                
                if pid1 >= 0 && pid2 >= 0 {
                    let mut status = 0;
                    let _ = wait(&mut status);
                    let _ = wait(&mut status);
                } else {
                    println!("Error al spawnear procesos en la tubería");
                }
            } else {
                // Comando simple
                let mut path_buf = [0u8; 64];
                let path = resolve_path(cmd_str, &mut path_buf);
                if !path.is_empty() {
                    let pid = spawn(path, 0, 0, 0, 0);
                    if pid >= 0 {
                        let mut status = 0;
                        let _ = wait(&mut status);
                    } else {
                        println!("Error al ejecutar: {}", cmd_str);
                    }
                }
            }
        }
    }
    0
}

fn resolve_path<'a>(cmd: &str, out_buf: &'a mut [u8]) -> &'a str {
    let cmd = cmd.trim();
    if cmd.starts_with('/') {
        let cmd_bytes = cmd.as_bytes();
        if cmd_bytes.len() < out_buf.len() {
            out_buf[..cmd_bytes.len()].copy_from_slice(cmd_bytes);
            core::str::from_utf8(&out_buf[..cmd_bytes.len()]).unwrap_or("")
        } else {
            ""
        }
    } else {
        let prefix = b"/bin/";
        let cmd_bytes = cmd.as_bytes();
        if prefix.len() + cmd_bytes.len() < out_buf.len() {
            out_buf[..prefix.len()].copy_from_slice(prefix);
            out_buf[prefix.len()..prefix.len() + cmd_bytes.len()].copy_from_slice(cmd_bytes);
            core::str::from_utf8(&out_buf[..prefix.len() + cmd_bytes.len()]).unwrap_or("")
        } else {
            ""
        }
    }
}
