#![no_std]
#![no_main]

use libc::{arg, close, ioctl, open, println, read, uptime_ms, yield_now};

const O_RDONLY: u32 = 1;
const WLAN_CONNECT: u32 = 1;
const WLAN_DISCONNECT: u32 = 2;
const WLAN_SCAN: u32 = 3;

/// Espejo del struct del kernel (`drivers::wifi::WlanConnectReq`, D-1). Campos `usize`.
#[repr(C)]
struct ConnectReq {
    ssid_ptr: usize,
    ssid_len: usize,
    pass_ptr: usize,
    pass_len: usize,
}

/// Lee el snapshot de `/dev/wlan0` en `buf`; devuelve su longitud.
fn snapshot(buf: &mut [u8]) -> usize {
    let fd = open("/dev/wlan0", O_RDONLY);
    if fd < 0 {
        return 0;
    }
    let mut total = 0usize;
    while total < buf.len() {
        let n = read(fd as i32, &mut buf[total..]);
        if n <= 0 {
            break;
        }
        total += n as usize;
    }
    close(fd as i32);
    total
}

/// Imprime las líneas del snapshot cuyo prefijo esté en `want`.
fn print_lines(buf: &[u8], want: &[&str]) {
    let text = core::str::from_utf8(buf).unwrap_or("");
    for line in text.split('\n') {
        if want.iter().any(|w| line.starts_with(w)) {
            println!("{}", line);
        }
    }
}

fn cmd_status() -> i32 {
    let mut buf = [0u8; 256];
    let n = snapshot(&mut buf);
    print_lines(&buf[..n], &["state:", "ssid:", "ip:"]);
    0
}

fn cmd_connect(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 3 {
        println!("usage: wifi connect \"SSID\" [PASS]");
        return 1;
    }
    let ssid = unsafe { arg(argv, 2) };
    let pass = if argc >= 4 { unsafe { arg(argv, 3) } } else { "" };
    let req = ConnectReq {
        ssid_ptr: ssid.as_ptr() as usize,
        ssid_len: ssid.len(),
        pass_ptr: pass.as_ptr() as usize,
        pass_len: pass.len(),
    };
    let fd = open("/dev/wlan0", O_RDONLY);
    if fd < 0 {
        println!("wifi: cannot open /dev/wlan0");
        return 1;
    }
    let rc = ioctl(fd as i32, WLAN_CONNECT, &req as *const ConnectReq as usize);
    close(fd as i32);
    if rc < 0 {
        println!("wifi: connect rejected ({})", rc);
        1
    } else {
        println!("Connecting to '{}'... (use 'wifi status' to check)", ssid);
        0
    }
}

fn cmd_simple(cmd: u32, msg: &str) -> i32 {
    let fd = open("/dev/wlan0", O_RDONLY);
    if fd < 0 {
        println!("wifi: cannot open /dev/wlan0");
        return 1;
    }
    let rc = ioctl(fd as i32, cmd, 0);
    close(fd as i32);
    if rc < 0 {
        println!("wifi: command failed ({})", rc);
        1
    } else {
        println!("{}", msg);
        0
    }
}

fn cmd_scan() -> i32 {
    if cmd_simple(WLAN_SCAN, "Scanning (the network drops during the scan)...") != 0 {
        return 1;
    }
    let t0 = uptime_ms();
    loop {
        let mut buf = [0u8; 1024];
        let n = snapshot(&mut buf);
        let text = core::str::from_utf8(&buf[..n]).unwrap_or("");
        if text.contains("scan: done\n") {
            println!("SSID\tRSSI\tCH\tSEC");
            print_lines(&buf[..n], &["ap:"]);
            return 0;
        }
        if text.contains("scan: error\n") {
            println!("wifi: scan error");
            return 1;
        }
        if uptime_ms().saturating_sub(t0) > 12000 {
            println!("wifi: scan timeout");
            return 1;
        }
        for _ in 0..80_000 {
            yield_now();
        }
    }
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        println!("usage: wifi status | scan | connect \"SSID\" [PASS] | disconnect");
        return 1;
    }
    match unsafe { arg(argv, 1) } {
        "status" => cmd_status(),
        "connect" => cmd_connect(argc, argv),
        "disconnect" => cmd_simple(WLAN_DISCONNECT, "Disconnecting..."),
        "scan" => cmd_scan(),
        other => {
            println!("wifi: unknown subcommand '{}'", other);
            1
        }
    }
}
