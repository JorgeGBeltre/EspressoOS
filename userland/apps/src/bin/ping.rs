#![no_std]
#![no_main]

use libc::{
    arg, close, connect, println, read, setsockopt_timeout, sockaddr_in,
    socket, uptime_ms, write,
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

fn calc_checksum(buf: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut i = 0;
    while i + 1 < buf.len() {
        let word = u16::from_be_bytes([buf[i], buf[i + 1]]);
        sum = sum.wrapping_add(word as u32);
        i += 2;
    }
    if i < buf.len() {
        sum = sum.wrapping_add((buf[i] as u32) << 8);
    }
    while (sum >> 16) != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !sum as u16
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        println!("Usage: ping <IPv4>");
        return 1;
    }

    let ip_str = unsafe { arg(argv, 1) };
    let ip = match parse_ip(ip_str) {
        Some(addr) => addr,
        None => {
            println!("ping: invalid IPv4 address '{}'", ip_str);
            return 1;
        }
    };

    println!("PING {} (ICMP Echo Request)...", ip_str);

    let fd = socket(2, 3, 1); // AF_INET, SOCK_RAW, IPPROTO_ICMP
    if fd < 0 {
        println!("ping: could not create ICMP raw socket");
        return 1;
    }

    // Configure 1000 ms timeout per ICMP probe
    let _ = setsockopt_timeout(fd as i32, 1000);

    let ip_u32 = u32::from_ne_bytes(ip);
    let addr = sockaddr_in {
        sin_family: 2,
        sin_port: 0,
        sin_addr: ip_u32,
        sin_zero: [0; 8],
    };

    if connect(fd as i32, &addr) < 0 {
        println!("ping: failed to connect ICMP socket");
        close(fd as i32);
        return 1;
    }

    let mut successful = 0;
    let mut failed = 0;
    let count = 4;
    let ident: u16 = 0x1234;

    for seq in 0..count {
        let mut pkt = [0u8; 64];
        pkt[0] = 8; // Type = Echo Request
        pkt[1] = 0; // Code = 0
        pkt[2] = 0; // Checksum High
        pkt[3] = 0; // Checksum Low
        pkt[4..6].copy_from_slice(&ident.to_be_bytes());
        pkt[6..8].copy_from_slice(&(seq as u16).to_be_bytes());

        // Fill payload
        let payload = b"EspressoOS ICMP Ping Probe 2026";
        pkt[8..8 + payload.len()].copy_from_slice(payload);

        // Compute checksum
        let csum = calc_checksum(&pkt);
        pkt[2..4].copy_from_slice(&csum.to_be_bytes());

        let start = uptime_ms();
        if write(fd as i32, &pkt) < 0 {
            println!("ping: failed to send ICMP Echo Request seq={}", seq);
            failed += 1;
            continue;
        }

        let mut resp = [0u8; 128];
        let n = read(fd as i32, &mut resp);
        let end = uptime_ms();

        if n >= 8 {
            let rtt = end.saturating_sub(start);
            let reply_type = resp[0];
            let reply_code = resp[1];
            let reply_id = u16::from_be_bytes([resp[4], resp[5]]);

            if reply_type == 0 && reply_code == 0 && reply_id == ident {
                println!(
                    "64 bytes from {}: icmp_seq={} time={} ms",
                    ip_str, seq, rtt
                );
                successful += 1;
            } else {
                println!(
                    "From {}: icmp_seq={} invalid ICMP response type={} code={}",
                    ip_str, seq, reply_type, reply_code
                );
                failed += 1;
            }
        } else if n < 0 {
            println!("Request timeout for icmp_seq {}", seq);
            failed += 1;
        } else {
            println!("Short ICMP reply received ({} bytes) for icmp_seq {}", n, seq);
            failed += 1;
        }
    }

    close(fd as i32);

    println!("\n--- {} ping statistics ---", ip_str);
    let loss = (failed * 100) / count;
    println!(
        "{} packets transmitted, {} received, {}% packet loss",
        count, successful, loss
    );

    if successful > 0 { 0 } else { 1 }
}
