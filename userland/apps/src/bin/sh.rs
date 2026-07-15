#![no_std]
#![no_main]

use libc::{println, print, read, spawn, wait, yield_now, pipe, dup2, close, readdir};

#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    println!("--- EspressoOS Shell (Userland) ---");
    let mut buf = [0u8; 64];
    loop {
        print!("EspressoOS:~$ ");
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
                println!("Exiting the shell...");
                break;
            }

            if cmd_str == "help" {
                print_help();
                continue;
            }

            if let Some(pipe_idx) = cmd_str.find('|') {

                let left_cmd = cmd_str[..pipe_idx].trim();
                let right_cmd = cmd_str[pipe_idx + 1..].trim();
                
                let mut p = [0i32; 2];
                if pipe(&mut p) < 0 {
                    println!("Error creating pipe");
                    continue;
                }
                
                let saved_stdout = dup2(1, 10);
                let saved_stdin = dup2(0, 11);
                
                let mut path_buf1 = [0u8; 64];
                let path1 = resolve_path(left_cmd, &mut path_buf1);
                
                let mut path_buf2 = [0u8; 64];
                let path2 = resolve_path(right_cmd, &mut path_buf2);
                
                if path1.is_empty() || path2.is_empty() {
                    println!("Empty or invalid command path");
                    close(p[0]);
                    close(p[1]);
                    close(10);
                    close(11);
                    continue;
                }
                

                dup2(p[1], 1);
                let pid1 = spawn(path1, 0, 0, 0, 0);
                

                dup2(p[0], 0);
                dup2(saved_stdout as i32, 1);
                let pid2 = spawn(path2, 0, 0, 0, 0);
                

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
                    println!("Error spawning pipeline processes");
                }
            } else {

                let mut path_buf = [0u8; 64];
                let path = resolve_path(cmd_str, &mut path_buf);
                if !path.is_empty() {
                    let pid = spawn(path, 0, 0, 0, 0);
                    if pid >= 0 {
                        let mut status = 0;
                        let _ = wait(&mut status);
                    } else {
                        println!("Error executing: {}", cmd_str);
                    }
                }
            }
        }
    }
    0
}

fn print_help() {
    println!("EspressoOS userland shell -- built-in commands:");
    println!("  help              show this help");
    println!("  exit              exit the shell");
    println!("  <cmd> | <cmd>     pipe one command's output into another");
    println!("");
    println!("Programs in /bin (run by name, e.g. 'ls' or '/bin/ls'):");
    let mut buf = [0u8; 1024];
    let n = readdir("/bin", &mut buf);
    if n < 0 {
        println!("  (could not read /bin)");
        return;
    }
    let mut pos = 0;
    let limit = n as usize;
    while pos < limit {
        if pos + 11 > limit {
            break;
        }
        let name_len = u16::from_le_bytes([buf[pos + 9], buf[pos + 10]]) as usize;
        pos += 11;
        if pos + name_len > limit {
            break;
        }
        if let Ok(name) = core::str::from_utf8(&buf[pos..pos + name_len]) {
            println!("  {}", name);
        }
        pos += name_len;
    }
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
