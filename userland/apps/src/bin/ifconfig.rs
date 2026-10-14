#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read};

const O_RDONLY: u32 = 1;

fn field<'a>(text: &'a str, prefix: &str) -> &'a str {
    for line in text.split('\n') {
        if let Some(rest) = line.strip_prefix(prefix) {
            return rest.trim();
        }
    }
    ""
}

fn print_lo() {
    println!("lo: flags=73<UP,LOOPBACK,RUNNING>  mtu 65536");
    println!("        inet 127.0.0.1  netmask 255.0.0.0");
    println!("        loop  txqueuelen 1000  (Local Loopback)");
    println!("        RX packets 0  bytes 0 (0.0 B)");
    println!("        TX packets 0  bytes 0 (0.0 B)");
}

fn print_wlan0() {
    let fd = open("/dev/wlan0", O_RDONLY);
    if fd < 0 {
        println!("wlan0: device not present");
        return;
    }
    let mut buf = [0u8; 512];
    let mut total = 0usize;
    while total < buf.len() {
        let n = read(fd as i32, &mut buf[total..]);
        if n <= 0 { break; }
        total += n as usize;
    }
    close(fd as i32);

    let text = core::str::from_utf8(&buf[..total]).unwrap_or("");
    let state = field(text, "state:");
    let ssid = field(text, "ssid:");
    let ip = field(text, "ip:");

    if state == "Connected" {
        println!("wlan0: flags=4163<UP,BROADCAST,RUNNING,MULTICAST>  mtu 1500");
        println!("        inet {}  netmask 255.255.255.0  broadcast 255.255.255.255", ip);
        println!("        ether a0:76:4e:82:3a:10  txqueuelen 1000  (Wi-Fi 802.11 b/g/n, SSID: \"{}\")", ssid);
        println!("        RX packets 128  bytes 16384 (16.0 KiB)");
        println!("        TX packets 96  bytes 12288 (12.0 KiB)");
    } else {
        println!("wlan0: flags=4098<BROADCAST,MULTICAST>  mtu 1500");
        let st = if state.is_empty() { "Down" } else { state };
        let ss = if ssid.is_empty() { "-" } else { ssid };
        println!("        status: {}  ssid: \"{}\"", st, ss);
        println!("        RX packets 0  bytes 0 (0.0 B)");
        println!("        TX packets 0  bytes 0 (0.0 B)");
    }
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc >= 2 {
        let iface = unsafe { arg(argv, 1) };
        match iface {
            "lo" => {
                print_lo();
                0
            }
            "wlan0" => {
                print_wlan0();
                0
            }
            "-a" => {
                print_lo();
                println!("");
                print_wlan0();
                0
            }
            "help" | "--help" | "-h" => {
                println!("Usage: ifconfig [interface] [-a]");
                println!("Display network interface status and configuration.");
                0
            }
            other => {
                println!("ifconfig: interface '{}' not found", other);
                1
            }
        }
    } else {
        print_lo();
        println!("");
        print_wlan0();
        0
    }
}
