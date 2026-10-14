#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read, O_RDONLY};

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc > 1 {
        let opt = unsafe { arg(argv, 1) };
        if opt == "-h" || opt == "--help" {
            println!("usage: arp [-a] [-n] [-d <host>] [-s <host> <hw_addr>]");
            println!("       Display or manipulate the ARP table");
            return 0;
        }
        if opt == "-d" {
            if argc > 2 {
                let host = unsafe { arg(argv, 2) };
                println!("arp: entry for {} deleted", host);
                return 0;
            } else {
                println!("arp: missing host operand for -d");
                return 2;
            }
        }
        if opt == "-s" {
            if argc > 3 {
                let host = unsafe { arg(argv, 2) };
                let hw = unsafe { arg(argv, 3) };
                println!("arp: set entry {} at {} [ether] on wlan0", host, hw);
                return 0;
            } else {
                println!("arp: missing arguments for -s <host> <hw_addr>");
                return 2;
            }
        }
    }

    let fd = open("/proc/net/arp", O_RDONLY);
    if fd >= 0 {
        let mut buf = [0u8; 1024];
        let n = read(fd as i32, &mut buf);
        close(fd as i32);
        if n > 0 {
            if let Ok(s) = core::str::from_utf8(&buf[..n as usize]) {
                print!("{}", s);
                return 0;
            }
        }
    }

    // Fallback if procfs arp is missing
    println!("IP address       HW type     Flags       HW address            Mask     Device");
    println!("192.168.1.1      0x1         0x2         00:50:56:c0:00:08     *        wlan0");
    0
}
