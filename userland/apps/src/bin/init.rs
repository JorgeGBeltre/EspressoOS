#![no_std]
#![no_main]

use libc::{println, open, read, close, spawn, wait, yield_now};

#[no_mangle]
pub extern "C" fn main() -> i32 {
    println!("[init] Initialization process PID 1 started");
    
    // Intentar leer /etc/rc
    let fd = open("/etc/rc", 0);
    if fd >= 0 {
        println!("[init] Reading /etc/rc...");
        let mut buf = [0u8; 1024];
        let n = read(fd as i32, &mut buf);
        if n > 0 {
            execute_rc(&buf[..n as usize]);
        }
        close(fd as i32);
    } else {
        println!("[init] /etc/rc not found, skipping startup script");
    }

    // Bucle infinito: spawnear shell en /dev/console y esperar
    loop {
        println!("[init] Launching interactive console (/bin/sh)...");
        let pid = spawn("/bin/sh", 0, 0, 0, 0);
        if pid >= 0 {
            let mut status = 0;
            let _ = wait(&mut status);
            println!("[init] /bin/sh exited with status {}. Restarting...", status);
        } else {
            println!("[init] ERROR spawning /bin/sh. Retrying...");
            for _ in 0..100000 {
                yield_now();
            }
        }
    }
}

fn execute_rc(data: &[u8]) {
    let content = match core::str::from_utf8(data) {
        Ok(s) => s,
        Err(_) => return,
    };
    
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        
        let mut bg = false;
        let mut cmd_str = line;
        if line.ends_with('&') {
            bg = true;
            cmd_str = line[..line.len() - 1].trim();
        }
        
        let mut parts = cmd_str.split_whitespace();
        if let Some(bin_path) = parts.next() {
            println!("[init] Executing: {} (bg={})", bin_path, bg);
            let pid = spawn(bin_path, 0, 0, 0, 0);
            if pid >= 0 {
                if !bg {
                    let mut status = 0;
                    let _ = wait(&mut status);
                }
            } else {
                println!("[init] ERROR executing: {}", bin_path);
            }
        }
    }
}
