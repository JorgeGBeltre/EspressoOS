#![no_std]
#![no_main]

use libc::{
    arg, close, connect, open, print, println, read, setsockopt_timeout, sockaddr_in, socket, write,
    O_RDONLY,
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

fn check_etc_hosts(host: &str) -> Option<[u8; 4]> {
    let fd = open("/etc/hosts", O_RDONLY);
    if fd < 0 {
        return None;
    }
    let mut buf = [0u8; 512];
    let n = read(fd as i32, &mut buf);
    let _ = close(fd as i32);
    if n <= 0 {
        return None;
    }
    let s = core::str::from_utf8(&buf[..n as usize]).ok()?;
    for line in s.split('\n') {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        if let Some(ip_str) = parts.next() {
            for h in parts {
                if h.eq_ignore_ascii_case(host) {
                    return parse_ip(ip_str);
                }
            }
        }
    }
    None
}

fn get_nameserver_ip() -> Option<[u8; 4]> {
    let fd = open("/etc/resolv.conf", O_RDONLY);
    if fd < 0 {
        return None;
    }
    let mut buf = [0u8; 256];
    let n = read(fd as i32, &mut buf);
    let _ = close(fd as i32);
    if n <= 0 {
        return None;
    }
    let s = core::str::from_utf8(&buf[..n as usize]).ok()?;
    for line in s.split('\n') {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        if let Some(tok) = parts.next() {
            if tok == "nameserver" {
                if let Some(ip_str) = parts.next() {
                    if let Some(ip) = parse_ip(ip_str) {
                        return Some(ip);
                    }
                }
            }
        }
    }
    None
}

fn print_dec(mut n: usize) {
    if n == 0 {
        print!("0");
        return;
    }
    let mut buf = [0u8; 16];
    let mut i = 0;
    while n > 0 {
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        i += 1;
    }
    while i > 0 {
        i -= 1;
        let s = core::slice::from_ref(&buf[i]);
        if let Ok(c) = core::str::from_utf8(s) {
            print!("{}", c);
        }
    }
}

fn print_ip(ip: &[u8; 4]) {
    print_dec(ip[0] as usize);
    print!(".");
    print_dec(ip[1] as usize);
    print!(".");
    print_dec(ip[2] as usize);
    print!(".");
    print_dec(ip[3] as usize);
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        println!("Usage: nslookup <hostname> [dns_server_ip]");
        return 1;
    }

    let host = unsafe { arg(argv, 1) };

    // 1. Check static mapping in /etc/hosts
    if let Some(ip) = check_etc_hosts(host) {
        println!("Server:    /etc/hosts");
        println!("Address:   127.0.0.1:53\n");
        println!("Non-authoritative answer:");
        print!("Name:      ");
        println!("{}", host);
        print!("Address:   ");
        print_ip(&ip);
        println!("");
        return 0;
    }

    let (server_ip, server_str) = if argc > 2 {
        let s = unsafe { arg(argv, 2) };
        match parse_ip(s) {
            Some(ip) => (ip, s),
            None => {
                println!("nslookup: invalid DNS server IP '{}'", s);
                return 1;
            }
        }
    } else if let Some(ip) = get_nameserver_ip() {
        (ip, "")
    } else {
        ([8, 8, 8, 8], "8.8.8.8")
    };

    let fd = socket(2, 2, 0); // AF_INET, SOCK_DGRAM
    if fd < 0 {
        println!("nslookup: failed to create UDP socket");
        return 1;
    }

    let _ = setsockopt_timeout(fd as i32, 3000);

    let server_addr = sockaddr_in {
        sin_family: 2,
        sin_port: 53u16.to_be(),
        sin_addr: u32::from_ne_bytes(server_ip),
        sin_zero: [0; 8],
    };

    if connect(fd as i32, &server_addr) < 0 {
        print!("nslookup: failed to connect UDP socket to ");
        if !server_str.is_empty() {
            print!("{}", server_str);
        } else {
            print_ip(&server_ip);
        }
        println!(":53");
        close(fd as i32);
        return 1;
    }

    // Build DNS Question packet (RFC 1035)
    let mut pkt = [0u8; 512];
    pkt[0] = 0x12; // ID high
    pkt[1] = 0x34; // ID low
    pkt[2] = 0x01; // Flags: RD = 1 (Recursion Desired)
    pkt[3] = 0x00;
    pkt[4] = 0x00; // QDCOUNT = 1
    pkt[5] = 0x01;
    // ANCOUNT, NSCOUNT, ARCOUNT = 0
    let mut off = 12;

    // Encode QNAME labels
    for part in host.split('.') {
        let len = part.len();
        if len == 0 || len > 63 || off + len + 1 >= pkt.len() {
            println!("nslookup: invalid hostname '{}'", host);
            close(fd as i32);
            return 1;
        }
        pkt[off] = len as u8;
        off += 1;
        pkt[off..off + len].copy_from_slice(part.as_bytes());
        off += len;
    }
    pkt[off] = 0x00; // root label
    off += 1;

    pkt[off] = 0x00; // QTYPE = A (1)
    pkt[off + 1] = 0x01;
    pkt[off + 2] = 0x00; // QCLASS = IN (1)
    pkt[off + 3] = 0x01;
    off += 4;

    if write(fd as i32, &pkt[..off]) < 0 {
        println!("nslookup: failed to send query packet");
        close(fd as i32);
        return 1;
    }

    let mut resp = [0u8; 512];
    let n = read(fd as i32, &mut resp);
    close(fd as i32);

    if n < 12 {
        println!("nslookup: connection timed out; no servers could be reached");
        return 1;
    }

    let n = n as usize;
    let rcode = resp[3] & 0x0F;
    if rcode != 0 {
        println!("nslookup: server failed to resolve (RCODE={})", rcode);
        return 1;
    }

    let ancount = ((resp[6] as usize) << 8) | (resp[7] as usize);
    if ancount == 0 {
        println!("nslookup: ** server can't find {}: No answer", host);
        return 1;
    }

    print!("Server:    ");
    if !server_str.is_empty() {
        println!("{}", server_str);
        print!("Address:   ");
        print!("{}", server_str);
    } else {
        print_ip(&server_ip);
        println!("");
        print!("Address:   ");
        print_ip(&server_ip);
    }
    println!(":53\n");
    println!("Non-authoritative answer:");
    print!("Name:      ");
    println!("{}", host);

    // Skip Header (12 bytes) and Question Section
    let mut idx = 12;
    while idx < n && resp[idx] != 0 {
        if resp[idx] & 0xC0 == 0xC0 {
            idx += 2;
            break;
        }
        idx += 1 + resp[idx] as usize;
    }
    if idx < n && resp[idx] == 0 {
        idx += 1;
    }
    idx += 4; // Skip QTYPE and QCLASS

    // Parse Answers
    let mut found = false;
    for _ in 0..ancount {
        if idx >= n {
            break;
        }
        // Skip NAME (pointer or labels)
        if resp[idx] & 0xC0 == 0xC0 {
            idx += 2;
        } else {
            while idx < n && resp[idx] != 0 {
                idx += 1 + resp[idx] as usize;
            }
            if idx < n && resp[idx] == 0 {
                idx += 1;
            }
        }
        if idx + 10 > n {
            break;
        }
        let atype = ((resp[idx] as u16) << 8) | (resp[idx + 1] as u16);
        let rdlen = ((resp[idx + 8] as usize) << 8) | (resp[idx + 9] as usize);
        idx += 10;

        if atype == 1 && rdlen == 4 && idx + 4 <= n {
            let mut ip = [0u8; 4];
            ip.copy_from_slice(&resp[idx..idx + 4]);
            print!("Address:   ");
            print_ip(&ip);
            println!("");
            found = true;
        }
        idx += rdlen;
    }

    if !found {
        println!("nslookup: no IPv4 address (A record) in answer");
        return 1;
    }

    0
}
