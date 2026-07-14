#![no_std]
#![no_main]

use libc::{println, socket, connect, close, uptime_ms, sockaddr_in};

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

#[no_mangle]
pub fn main() -> i32 {












    let ip_str = "192.168.1.1";
    let ip = match parse_ip(ip_str) {
        Some(addr) => addr,
        None => {
            println!("ping: invalid IP");
            return 1;
        }
    };
    
    println!("PING {} port 80 (TCP)...", ip_str);
    
    let fd = socket(2, 1, 0);
    if fd < 0 {
        println!("ping: could not create socket");
        return 1;
    }
    

    let ip_u32 = ((ip[0] as u32) << 0)
        | ((ip[1] as u32) << 8)
        | ((ip[2] as u32) << 16)
        | ((ip[3] as u32) << 24);
        
    let addr = sockaddr_in {
        sin_family: 2,
        sin_port: 80u16.to_be(),
        sin_addr: ip_u32,
        sin_zero: [0; 8],
    };
    
    let start = uptime_ms();
    let ret = connect(fd as i32, &addr);
    let end = uptime_ms();
    
    if ret == 0 {
        println!("Connection successful to {}, RTT = {} ms", ip_str, end - start);
    } else {
        println!("Failed to connect to {} (code {})", ip_str, ret);
    }
    
    close(fd as i32);
    0
}
