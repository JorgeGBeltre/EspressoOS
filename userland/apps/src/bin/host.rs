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

fn print_ip(ip: [u8; 4]) {
    print_dec(ip[0] as usize);
    print!(".");
    print_dec(ip[1] as usize);
    print!(".");
    print_dec(ip[2] as usize);
    print!(".");
    print_dec(ip[3] as usize);
}

fn check_etc_hosts(host: &str) -> Option<[u8; 4]> {
    let fd = open("/etc/hosts", O_RDONLY);
    if fd < 0 {
        return None;
    }
    let mut buf = [0u8; 512];
    let n = read(fd as i32, &mut buf);
    close(fd as i32);
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

fn read_resolv_conf(server_buf: &mut [u8; 4]) {
    *server_buf = [8, 8, 8, 8]; // Default Google Public DNS
    let fd = open("/etc/resolv.conf", O_RDONLY);
    if fd < 0 {
        return;
    }
    let mut buf = [0u8; 256];
    let n = read(fd as i32, &mut buf);
    close(fd as i32);
    if n <= 0 {
        return;
    }
    if let Ok(s) = core::str::from_utf8(&buf[..n as usize]) {
        for line in s.split('\n') {
            let trimmed = line.trim();
            if trimmed.starts_with("nameserver") {
                let mut parts = trimmed.split_whitespace();
                let _ = parts.next();
                if let Some(ns) = parts.next() {
                    if let Some(parsed) = parse_ip(ns) {
                        *server_buf = parsed;
                        return;
                    }
                }
            }
        }
    }
}

fn build_dns_query(host: &str, buf: &mut [u8; 256]) -> usize {
    buf[0] = 0x12; buf[1] = 0x34; // ID
    buf[2] = 0x01; buf[3] = 0x00; // Flags: Standard query
    buf[4] = 0x00; buf[5] = 0x01; // QDCOUNT: 1
    buf[6] = 0x00; buf[7] = 0x00; // ANCOUNT: 0
    buf[8] = 0x00; buf[9] = 0x00; // NSCOUNT: 0
    buf[10] = 0x00; buf[11] = 0x00; // ARCOUNT: 0

    let mut idx = 12;
    for label in host.split('.') {
        let len = label.len();
        if len == 0 || len > 63 || idx + 1 + len >= buf.len() - 4 {
            return 0;
        }
        buf[idx] = len as u8;
        idx += 1;
        buf[idx..idx + len].copy_from_slice(label.as_bytes());
        idx += len;
    }
    buf[idx] = 0; // Terminating zero length
    idx += 1;

    buf[idx] = 0x00; buf[idx + 1] = 0x01; // QTYPE: A (IPv4)
    buf[idx + 2] = 0x00; buf[idx + 3] = 0x01; // QCLASS: IN
    idx + 4
}

fn parse_dns_response(buf: &[u8], len: usize) -> Option<[u8; 4]> {
    if len < 12 { return None; }
    let ancount = ((buf[6] as u16) << 8) | (buf[7] as u16);
    if ancount == 0 { return None; }

    let mut idx = 12;
    while idx < len && buf[idx] != 0 {
        if (buf[idx] & 0xC0) == 0xC0 {
            idx += 2;
            break;
        }
        idx += 1 + buf[idx] as usize;
    }
    if idx < len && buf[idx] == 0 {
        idx += 1;
    }
    idx += 4; // Skip QTYPE and QCLASS

    for _ in 0..ancount {
        if idx >= len { break; }
        if (buf[idx] & 0xC0) == 0xC0 {
            idx += 2;
        } else {
            while idx < len && buf[idx] != 0 {
                idx += 1 + buf[idx] as usize;
            }
            if idx < len { idx += 1; }
        }
        if idx + 10 > len { break; }
        let rtype = ((buf[idx] as u16) << 8) | (buf[idx + 1] as u16);
        let rdlength = ((buf[idx + 8] as u16) << 8) | (buf[idx + 9] as u16);
        idx += 10;
        if rtype == 1 && rdlength == 4 && idx + 4 <= len {
            return Some([buf[idx], buf[idx + 1], buf[idx + 2], buf[idx + 3]]);
        }
        idx += rdlength as usize;
    }
    None
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        println!("Usage: host [-t type] HOST [SERVER]");
        println!("DNS and static host lookup utility");
        return 1;
    }

    let mut host_arg = "";
    let mut server_arg = "";
    let mut i = 1;
    while i < argc {
        let s = unsafe { arg(argv, i) };
        if s == "-h" || s == "--help" {
            println!("Usage: host [-t type] HOST [SERVER]");
            println!("DNS and static host lookup utility");
            return 0;
        } else if s == "-t" && i + 1 < argc {
            i += 1;
        } else if host_arg.is_empty() {
            host_arg = s;
        } else if server_arg.is_empty() {
            server_arg = s;
        }
        i += 1;
    }

    if host_arg.is_empty() {
        println!("host: missing host operand");
        return 1;
    }

    // 1. Direct IP check
    if let Some(ip) = parse_ip(host_arg) {
        print!("{}", host_arg);
        print!(" has address ");
        print_ip(ip);
        println!("");
        return 0;
    }

    // 2. Static /etc/hosts resolution
    if let Some(ip) = check_etc_hosts(host_arg) {
        print!("{}", host_arg);
        print!(" has address ");
        print_ip(ip);
        println!("");
        return 0;
    }

    // 3. DNS Network query
    let mut dns_server = [8, 8, 8, 8];
    if !server_arg.is_empty() {
        if let Some(s_ip) = parse_ip(server_arg) {
            dns_server = s_ip;
        }
    } else {
        read_resolv_conf(&mut dns_server);
    }

    let sock = socket(2 /* AF_INET */, 2 /* SOCK_DGRAM */, 0);
    if sock < 0 {
        print!("Host ");
        print!("{}", host_arg);
        println!(" not found: 3(NXDOMAIN)");
        return 1;
    }

    let _ = setsockopt_timeout(sock as i32, 2000);
    let addr = sockaddr_in {
        sin_family: 2,
        sin_port: 53u16.to_be(),
        sin_addr: u32::from_ne_bytes(dns_server),
        sin_zero: [0; 8],
    };
    if connect(sock as i32, &addr) < 0 {
        close(sock as i32);
        print!("Host ");
        print!("{}", host_arg);
        println!(" not found: 3(NXDOMAIN)");
        return 1;
    }

    let mut query_buf = [0u8; 256];
    let qlen = build_dns_query(host_arg, &mut query_buf);
    if qlen == 0 {
        close(sock as i32);
        return 1;
    }

    if write(sock as i32, &query_buf[..qlen]) <= 0 {
        close(sock as i32);
        print!("Host ");
        print!("{}", host_arg);
        println!(" not found: 3(NXDOMAIN)");
        return 1;
    }

    let mut resp_buf = [0u8; 256];
    let rlen = read(sock as i32, &mut resp_buf);
    close(sock as i32);

    if rlen > 0 {
        if let Some(ip) = parse_dns_response(&resp_buf, rlen as usize) {
            print!("{}", host_arg);
            print!(" has address ");
            print_ip(ip);
            println!("");
            return 0;
        }
    }

    print!("Host ");
    print!("{}", host_arg);
    println!(" not found: 3(NXDOMAIN)");
    1
}
