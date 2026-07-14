#![no_std]
#![no_main]

use libc::{println, socket, connect, write, read, close, timeval, settimeofday, sockaddr_in, yield_now};

#[no_mangle]
pub extern "C" fn main() -> i32 {
    println!("[sntp] Starting time synchronization...");

    let fd = socket(2, 2, 0);
    if fd < 0 {
        println!("[sntp] Failed to create UDP socket");
        return -1;
    }

    let ip: u32 = u32::from_ne_bytes([128, 138, 140, 44]);
    let port = 123u16;

    let addr = sockaddr_in {
        sin_family: 2,
        sin_port: port.to_be(),
        sin_addr: ip,
        sin_zero: [0; 8],
    };

    println!("[sntp] Connecting to 128.138.140.44:123...");
    if connect(fd as i32, &addr) < 0 {
        println!("[sntp] Failed to connect UDP socket");
        close(fd as i32);
        return -1;
    }

    let mut pkt = [0u8; 48];
    pkt[0] = 0x1B;

    println!("[sntp] Sending SNTP request...");
    if write(fd as i32, &pkt) < 0 {
        println!("[sntp] Failed to send request");
        close(fd as i32);
        return -1;
    }

    println!("[sntp] Waiting for response...");
    let mut resp = [0u8; 48];
    let mut attempts = 0;
    loop {
        let n = read(fd as i32, &mut resp);
        if n > 0 {
            if n == 48 {
                break;
            }
        }
        attempts += 1;
        if attempts > 200 {
            println!("[sntp] Timeout waiting for response");
            close(fd as i32);
            return -1;
        }
        yield_now();
    }

    let seconds_1900 = u32::from_be_bytes([resp[40], resp[41], resp[42], resp[43]]);
    
    let ntp_offset = 2208988800u32;
    if seconds_1900 < ntp_offset {
        println!("[sntp] Error: received timestamp is invalid");
        close(fd as i32);
        return -1;
    }
    let unix_secs = seconds_1900 - ntp_offset;
    
    let tv = timeval {
        tv_sec: unix_secs as i32,
        tv_usec: 0,
    };

    println!("[sntp] Time received: {} s (UNIX Epoch). Setting clock...", unix_secs);
    if settimeofday(&tv) < 0 {
        println!("[sntp] Failed to update settimeofday");
        close(fd as i32);
        return -1;
    }

    println!("[sntp] Clock synchronized successfully!");
    close(fd as i32);
    0
}
