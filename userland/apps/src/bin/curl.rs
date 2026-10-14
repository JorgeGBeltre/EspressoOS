#![no_std]
#![no_main]

use libc::{arg, close, connect, print, println, read, setsockopt_timeout, sockaddr_in, socket, write};

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
        println!("Usage: curl [-i] <IPv4> [port] [path]");
        return 1;
    }

    let mut arg_idx = 1;
    let mut include_headers = false;
    let first = unsafe { arg(argv, arg_idx) };
    if first == "-i" {
        include_headers = true;
        arg_idx += 1;
        if arg_idx >= argc {
            println!("Usage: curl [-i] <IPv4> [port] [path]");
            return 1;
        }
    }

    let host = unsafe { arg(argv, arg_idx) };
    let ip = match parse_ip(host) {
        Some(addr) => addr,
        None => {
            println!("curl: invalid IPv4 address '{}'", host);
            return 1;
        }
    };
    arg_idx += 1;

    let port = if arg_idx < argc {
        let p_str = unsafe { arg(argv, arg_idx) };
        if let Some(p) = parse_port(p_str) {
            arg_idx += 1;
            p
        } else {
            80u16
        }
    } else {
        80u16
    };

    let path = if arg_idx < argc {
        unsafe { arg(argv, arg_idx) }
    } else {
        "/"
    };

    let fd = socket(2, 1, 0); // AF_INET, SOCK_STREAM
    if fd < 0 {
        println!("curl: failed to create TCP socket");
        return 1;
    }

    let _ = setsockopt_timeout(fd as i32, 5000);

    let addr = sockaddr_in {
        sin_family: 2,
        sin_port: port.to_be(),
        sin_addr: u32::from_ne_bytes(ip),
        sin_zero: [0; 8],
    };

    if connect(fd as i32, &addr) < 0 {
        println!("curl: failed to connect to {}:{}", host, port);
        close(fd as i32);
        return 1;
    }

    // Send HTTP GET request
    let _ = write(fd as i32, b"GET ");
    let _ = write(fd as i32, path.as_bytes());
    let _ = write(fd as i32, b" HTTP/1.1\r\nHost: ");
    let _ = write(fd as i32, host.as_bytes());
    let _ = write(fd as i32, b"\r\nUser-Agent: EspressoOS/0.2\r\nConnection: close\r\n\r\n");

    let mut buf = [0u8; 512];
    let mut header_skipped = include_headers;
    let mut pending_header = [0u8; 1024];
    let mut pending_len = 0;

    loop {
        let n = read(fd as i32, &mut buf);
        if n <= 0 {
            break;
        }
        let chunk = &buf[..n as usize];
        if header_skipped {
            let _ = write(1, chunk);
        } else {
            let copy_len = chunk.len().min(pending_header.len() - pending_len);
            pending_header[pending_len..pending_len + copy_len].copy_from_slice(&chunk[..copy_len]);
            pending_len += copy_len;

            // Search for \r\n\r\n
            if let Some(pos) = pending_header[..pending_len]
                .windows(4)
                .position(|w| w == b"\r\n\r\n")
            {
                header_skipped = true;
                let body_start = pos + 4;
                if body_start < pending_len {
                    let _ = write(1, &pending_header[body_start..pending_len]);
                }
            }
        }
    }

    close(fd as i32);
    0
}
