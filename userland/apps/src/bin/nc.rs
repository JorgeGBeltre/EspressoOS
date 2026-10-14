#![no_std]
#![no_main]

use libc::{
    accept, arg, bind, close, connect, listen, print, println, read, setsockopt_timeout,
    sockaddr_in, socket, write,
};

fn parse_ip(s: &str) -> Option<[u8; 4]> {
    let mut ip = [0u8; 4];
    let mut part = 0;
    let mut val = 0u32;
    let mut has_digits = false;
    for &b in s.as_bytes() {
        if b == b'.' {
            if part >= 3 || !has_digits {
                return None;
            }
            ip[part] = val as u8;
            part += 1;
            val = 0;
            has_digits = false;
        } else if b >= b'0' && b <= b'9' {
            val = val * 10 + (b - b'0') as u32;
            if val > 255 {
                return None;
            }
            has_digits = true;
        } else {
            return None;
        }
    }
    if part != 3 || !has_digits {
        return None;
    }
    ip[3] = val as u8;
    Some(ip)
}

fn parse_port(s: &str) -> Option<u16> {
    let mut v = 0u32;
    let mut any = false;
    for &b in s.as_bytes() {
        if b >= b'0' && b <= b'9' {
            v = v * 10 + (b - b'0') as u32;
            if v > 65535 {
                return None;
            }
            any = true;
        } else {
            return None;
        }
    }
    if any {
        Some(v as u16)
    } else {
        None
    }
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        println!("usage: nc [-l] [-p port] [host] [port]");
        println!("       Arbitrary TCP and UDP connections and listens");
        return 1;
    }

    let mut listen_mode = false;
    let mut port_opt: Option<u16> = None;
    let mut host_opt: Option<&str> = None;

    let mut i = 1;
    while i < argc {
        let a = unsafe { arg(argv, i) };
        if a == "-l" {
            listen_mode = true;
        } else if a == "-p" {
            i += 1;
            if i < argc {
                let p_str = unsafe { arg(argv, i) };
                port_opt = parse_port(p_str);
            }
        } else if a == "-h" || a == "--help" {
            println!("usage: nc [-l] [-p port] [host] [port]");
            return 0;
        } else if !a.starts_with('-') {
            if host_opt.is_none() && !listen_mode {
                host_opt = Some(a);
            } else if port_opt.is_none() {
                port_opt = parse_port(a);
            }
        }
        i += 1;
    }

    let sock = socket(2, 1, 0); // AF_INET, SOCK_STREAM
    if sock < 0 {
        println!("nc: cannot create socket");
        return 1;
    }

    if listen_mode {
        let port = port_opt.unwrap_or(8080);
        let bind_addr = sockaddr_in {
            sin_family: 2,
            sin_port: port.to_be(),
            sin_addr: 0,
            sin_zero: [0; 8],
        };

        if bind(sock as i32, &bind_addr) < 0 {
            println!("nc: bind failed on port {}", port);
            close(sock as i32);
            return 1;
        }

        if listen(sock as i32, 1) < 0 {
            println!("nc: listen failed on port {}", port);
            close(sock as i32);
            return 1;
        }

        println!("nc: listening on 0.0.0.0:{}", port);
        let mut client_addr = sockaddr_in {
            sin_family: 0,
            sin_port: 0,
            sin_addr: 0,
            sin_zero: [0; 8],
        };

        let client_fd = accept(sock as i32, &mut client_addr);
        if client_fd < 0 {
            println!("nc: accept error");
            close(sock as i32);
            return 1;
        }

        println!("nc: client connected");

        // Forward received bytes to stdout
        let mut buf = [0u8; 512];
        loop {
            let n = read(client_fd as i32, &mut buf);
            if n <= 0 {
                break;
            }
            if let Ok(s) = core::str::from_utf8(&buf[..n as usize]) {
                print!("{}", s);
            }
        }
        close(client_fd as i32);
        close(sock as i32);
        return 0;
    }

    // Client mode
    let host = match host_opt {
        Some(h) => h,
        None => {
            println!("nc: missing destination host");
            close(sock as i32);
            return 1;
        }
    };

    let port = match port_opt {
        Some(p) => p,
        None => {
            println!("nc: missing destination port");
            close(sock as i32);
            return 1;
        }
    };

    let ip = match parse_ip(host) {
        Some(addr) => addr,
        None => {
            println!("nc: invalid IPv4 address '{}'", host);
            close(sock as i32);
            return 1;
        }
    };

    let _ = setsockopt_timeout(sock as i32, 5000);

    let target_addr = sockaddr_in {
        sin_family: 2,
        sin_port: port.to_be(),
        sin_addr: u32::from_ne_bytes(ip),
        sin_zero: [0; 8],
    };

    if connect(sock as i32, &target_addr) < 0 {
        println!("nc: connect to {}:{} failed", host, port);
        close(sock as i32);
        return 1;
    }

    // Check if stdin has data (try nonblocking/short read on fd 0)
    let mut stdin_buf = [0u8; 512];
    let in_n = read(0, &mut stdin_buf);
    if in_n > 0 {
        let _ = write(sock as i32, &stdin_buf[..in_n as usize]);
    }

    // Read response from network
    let mut buf = [0u8; 512];
    loop {
        let n = read(sock as i32, &mut buf);
        if n <= 0 {
            break;
        }
        if let Ok(s) = core::str::from_utf8(&buf[..n as usize]) {
            print!("{}", s);
        }
    }

    close(sock as i32);
    0
}
