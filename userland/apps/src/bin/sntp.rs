#![no_std]
#![no_main]

use libc::{
    arg, close, connect, println, read, settimeofday, sockaddr_in, socket,
    timeval, write, setsockopt_timeout,
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

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let server_str = if argc > 1 {
        unsafe { arg(argv, 1) }
    } else {
        "128.138.140.44" // Default NIST NTP server
    };

    let ip = match parse_ip(server_str) {
        Some(addr) => addr,
        None => {
            println!("[sntp] Error: invalid IPv4 server address '{}'", server_str);
            return 1;
        }
    };

    println!("[sntp] Starting time synchronization with {}...", server_str);

    let fd = socket(2, 2, 0); // AF_INET, SOCK_DGRAM
    if fd < 0 {
        println!("[sntp] Error: failed to create UDP socket");
        return 1;
    }

    // Configure 2000 ms (2 seconds) read timeout on UDP socket
    if setsockopt_timeout(fd as i32, 2000) < 0 {
        println!("[sntp] Warning: failed to set socket read timeout");
    }

    let ip_u32 = u32::from_ne_bytes(ip);
    let port = 123u16;

    let addr = sockaddr_in {
        sin_family: 2,
        sin_port: port.to_be(),
        sin_addr: ip_u32,
        sin_zero: [0; 8],
    };

    if connect(fd as i32, &addr) < 0 {
        println!("[sntp] Error: failed to connect UDP socket to {}:123", server_str);
        close(fd as i32);
        return 1;
    }

    let mut pkt = [0u8; 48];
    pkt[0] = 0x1B; // LI=0, VN=3 (SNTPv3), Mode=3 (Client)

    let max_attempts = 3;
    let mut resp = [0u8; 48];
    let mut success = false;

    for attempt in 1..=max_attempts {
        println!("[sntp] Sending SNTP request to {} (attempt {}/{})...", server_str, attempt, max_attempts);
        
        if write(fd as i32, &pkt) < 0 {
            println!("[sntp] Error sending request on attempt {}", attempt);
            continue;
        }

        let n = read(fd as i32, &mut resp);
        if n == 48 {
            success = true;
            break;
        } else if n < 0 {
            println!("[sntp] Attempt {} timed out or failed (code {})", attempt, n);
        } else {
            println!("[sntp] Attempt {} received invalid packet length ({})", attempt, n);
        }
    }

    close(fd as i32);

    if !success {
        println!("[sntp] Error: NTP server {} did not respond after {} attempts.", server_str, max_attempts);
        return 1;
    }

    let seconds_1900 = u32::from_be_bytes([resp[40], resp[41], resp[42], resp[43]]);
    let ntp_offset = 2208988800u32; // Offset between 1900-01-01 and 1970-01-01

    if seconds_1900 < ntp_offset {
        println!("[sntp] Error: received timestamp (0x{:08X}) is invalid or prior to 1970", seconds_1900);
        return 1;
    }

    let unix_secs = seconds_1900 - ntp_offset;
    let tv = timeval {
        tv_sec: unix_secs as i32,
        tv_usec: 0,
    };

    println!("[sntp] Received NTP time: {} s (Unix Epoch)", unix_secs);
    if settimeofday(&tv) < 0 {
        println!("[sntp] Error: settimeofday failed");
        return 1;
    }

    println!("[sntp] Clock synchronized successfully!");
    0
}
