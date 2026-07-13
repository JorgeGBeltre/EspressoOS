#![no_std]
#![no_main]

use libc::{println, socket, bind, listen, accept, close, write, read, open, sockaddr_in};

fn read_file_to_buf(path: &str, buf: &mut [u8]) -> usize {
    let fd = open(path, 0);
    if fd < 0 {
        return 0;
    }
    let n = read(fd as i32, buf);
    close(fd as i32);
    if n > 0 { n as usize } else { 0 }
}

#[no_mangle]
pub fn main() -> i32 {
    let fd = socket(2, 1, 0); // AF_INET, SOCK_STREAM
    if fd < 0 {
        println!("httpd: no se pudo crear el socket");
        return 1;
    }
    
    let addr = sockaddr_in {
        sin_family: 2, // AF_INET
        sin_port: 80u16.to_be(),
        sin_addr: 0, // INADDR_ANY (0.0.0.0)
        sin_zero: [0; 8],
    };
    
    if bind(fd as i32, &addr) < 0 {
        println!("httpd: error al hacer bind en puerto 80");
        close(fd as i32);
        return 1;
    }
    
    if listen(fd as i32, 5) < 0 {
        println!("httpd: error en listen");
        close(fd as i32);
        return 1;
    }
    
    println!("httpd: servidor web escuchando en el puerto 80...");
    
    let mut client_addr = sockaddr_in {
        sin_family: 0,
        sin_port: 0,
        sin_addr: 0,
        sin_zero: [0; 8],
    };
    
    loop {
        let client_fd = accept(fd as i32, &mut client_addr);
        if client_fd >= 0 {
            let mut req = [0u8; 1024];
            let _n = read(client_fd as i32, &mut req);
            
            // Leer estado del sistema de /proc
            let mut uptime_buf = [0u8; 64];
            let u_len = read_file_to_buf("/proc/uptime", &mut uptime_buf);
            let uptime_str = if u_len > 0 {
                unsafe { core::str::from_utf8_unchecked(&uptime_buf[..u_len]) }
            } else {
                "uptime: N/A\n"
            };
            
            let mut mem_buf = [0u8; 128];
            let m_len = read_file_to_buf("/proc/meminfo", &mut mem_buf);
            let mem_str = if m_len > 0 {
                unsafe { core::str::from_utf8_unchecked(&mem_buf[..m_len]) }
            } else {
                "MemTotal: N/A\n"
            };

            // Construir respuesta HTML
            let body_prefix = "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n\
            <html>\
            <head>\
                <title>EspressoOS Web Server</title>\
                <style>\
                    body { font-family: 'Segoe UI', Tahoma, Geneva, Verdana, sans-serif; background: #0f0f13; color: #e2e8f0; padding: 3rem; margin: 0; }\
                    .container { max-width: 600px; margin: 0 auto; background: #1e1e24; padding: 2rem; border-radius: 12px; box-shadow: 0 4px 20px rgba(0,0,0,0.5); border: 1px solid #2e2e38; }\
                    h1 { color: #f59e0b; margin-top: 0; border-bottom: 2px solid #2e2e38; padding-bottom: 0.5rem; }\
                    pre { background: #0f0f13; padding: 1rem; border-radius: 6px; border: 1px solid #2e2e38; color: #10b981; font-family: monospace; }\
                    .footer { text-align: center; margin-top: 2rem; font-size: 0.85rem; color: #64748b; }\
                </style>\
            </head>\
            <body>\
                <div class='container'>\
                    <h1>EspressoOS Web Server</h1>\
                    <p>Este recurso es servido dinamicamente por el proceso <code>httpd</code> en World-1.</p>\
                    <h3>Estado de /proc/uptime:</h3>";
                    
            let body_mid = "<h3>Estado de /proc/meminfo:</h3>";
            
            let body_suffix = "</div>\
                <div class='footer'>EspressoOS &copy; 2026 - Multicore SMP IoT OS</div>\
            </body>\
            </html>";
            
            let _ = write(client_fd as i32, body_prefix.as_bytes());
            let _ = write(client_fd as i32, uptime_str.as_bytes());
            let _ = write(client_fd as i32, body_mid.as_bytes());
            let _ = write(client_fd as i32, mem_str.as_bytes());
            let _ = write(client_fd as i32, body_suffix.as_bytes());
            
            close(client_fd as i32);
        }
    }
}
