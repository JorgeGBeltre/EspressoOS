#![no_std]
#![no_main]

use libc::{arg, close, connect, println, sockaddr_in, socket, uptime_ms};

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

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        println!("Usage: tcping <IPv4> [port]");
        return 1;
    }

    let ip_str = unsafe { arg(argv, 1) };
    let ip = match parse_ip(ip_str) {
        Some(addr) => addr,
        None => {
            println!("tcping: invalid IPv4 address '{}'", ip_str);
            return 1;
        }
    };

    let port = if argc >= 3 {
        let p_str = unsafe { arg(argv, 2) };
        match parse_port(p_str) {
            Some(p) => p,
            None => {
                println!("tcping: invalid port number '{}'", p_str);
                return 1;
            }
        }
    } else {
        80
    };

    println!("TCPING {}:{} (TCP handshake)...", ip_str, port);

    let ip_u32 = ((ip[0] as u32) << 0)
        | ((ip[1] as u32) << 8)
        | ((ip[2] as u32) << 16)
        | ((ip[3] as u32) << 24);

    let addr = sockaddr_in {
        sin_family: 2,
        sin_port: port.to_be(),
        sin_addr: ip_u32,
        sin_zero: [0; 8],
    };

    let mut successful = 0;
    let mut failed = 0;
    let count = 4;

    for seq in 0..count {
        let fd = socket(2, 1, 0);
        if fd < 0 {
            println!("tcping: could not create socket");
            return 1;
        }

        let start = uptime_ms();
        let ret = connect(fd as i32, &addr);
        let end = uptime_ms();
        close(fd as i32);

        if ret == 0 {
            let rtt = end.saturating_sub(start);
            println!(
                "Port {} open on {}: seq={} time={} ms",
                port, ip_str, seq, rtt
            );
            successful += 1;
        } else {
            println!(
                "Port {} closed/unreachable on {}: seq={} (code {})",
                port, ip_str, seq, ret
            );
            failed += 1;
        }
    }

    println!("\n--- {} tcping statistics ---", ip_str);
    let loss = (failed * 100) / count;
    println!(
        "{} packets transmitted, {} received, {}% packet loss",
        count, successful, loss
    );

    if successful > 0 { 0 } else { 1 }
}
