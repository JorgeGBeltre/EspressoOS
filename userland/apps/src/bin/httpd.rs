#![no_std]
#![no_main]

use libc::{
    accept, arg, bind, close, listen, open, println, read, sockaddr_in,
    socket, write, setsockopt_timeout,
};

fn parse_port(s: &str) -> Option<u16> {
    if s.is_empty() {
        return None;
    }
    let mut val = 0u32;
    for &b in s.as_bytes() {
        if b >= b'0' && b <= b'9' {
            val = val * 10 + (b - b'0') as u32;
            if val > 65535 {
                return None;
            }
        } else {
            return None;
        }
    }
    Some(val as u16)
}

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
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let port = if argc > 1 {
        let p_str = unsafe { arg(argv, 1) };
        match parse_port(p_str) {
            Some(p) => p,
            None => {
                println!("httpd: invalid port '{}'", p_str);
                return 1;
            }
        }
    } else {
        80
    };

    let fd = socket(2, 1, 0); // AF_INET, SOCK_STREAM
    if fd < 0 {
        println!("httpd: could not create TCP socket");
        return 1;
    }

    let addr = sockaddr_in {
        sin_family: 2,
        sin_port: port.to_be(),
        sin_addr: 0,
        sin_zero: [0; 8],
    };

    if bind(fd as i32, &addr) < 0 {
        println!("httpd: failed to bind on port {}", port);
        close(fd as i32);
        return 1;
    }

    if listen(fd as i32, 5) < 0 {
        println!("httpd: listen error on port {}", port);
        close(fd as i32);
        return 1;
    }

    println!("httpd: web server listening on port {}...", port);

    let mut client_addr = sockaddr_in {
        sin_family: 0,
        sin_port: 0,
        sin_addr: 0,
        sin_zero: [0; 8],
    };

    loop {
        let client_fd = accept(fd as i32, &mut client_addr);
        if client_fd >= 0 {
            // Set 3000 ms read timeout on client socket to prevent hanging
            let _ = setsockopt_timeout(client_fd as i32, 3000);

            let mut req = [0u8; 1024];
            let n = read(client_fd as i32, &mut req);

            if n <= 0 {
                close(client_fd as i32);
                continue;
            }

            // Read /proc/uptime dynamically per request
            let mut uptime_buf = [0u8; 64];
            let u_len = read_file_to_buf("/proc/uptime", &mut uptime_buf);
            let uptime_str = if u_len > 0 {
                unsafe { core::str::from_utf8_unchecked(&uptime_buf[..u_len]) }
            } else {
                "uptime: N/A\n"
            };

            // Read /proc/meminfo dynamically per request
            let mut mem_buf = [0u8; 128];
            let m_len = read_file_to_buf("/proc/meminfo", &mut mem_buf);
            let mem_str = if m_len > 0 {
                unsafe { core::str::from_utf8_unchecked(&mem_buf[..m_len]) }
            } else {
                "MemTotal: N/A\n"
            };

            let header = "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n";
            let html_p1 = "<html><head><title>EspressoOS Web Server</title>\
                <style>\
                body { font-family: sans-serif; background: #0f0f13; color: #e2e8f0; padding: 2rem; }\
                .card { background: #1e1e24; padding: 1.5rem; border-radius: 8px; border: 1px solid #2e2e38; max-width: 600px; margin: auto; }\
                h1 { color: #f59e0b; }\
                pre { background: #0f0f13; padding: 1rem; color: #10b981; border-radius: 4px; }\
                </style></head><body><div class='card'>\
                <h1>EspressoOS HTTP Server</h1>\
                <p>Status dynamically fetched from <code>/proc</code>:</p>\
                <h3>/proc/uptime</h3><pre>";

            let html_p2 = "</pre><h3>/proc/meminfo</h3><pre>";
            let html_p3 = "</pre></div></body></html>";

            let _ = write(client_fd as i32, header.as_bytes());
            let _ = write(client_fd as i32, html_p1.as_bytes());
            let _ = write(client_fd as i32, uptime_str.as_bytes());
            let _ = write(client_fd as i32, html_p2.as_bytes());
            let _ = write(client_fd as i32, mem_str.as_bytes());
            let _ = write(client_fd as i32, html_p3.as_bytes());

            close(client_fd as i32);
        }
    }
}
