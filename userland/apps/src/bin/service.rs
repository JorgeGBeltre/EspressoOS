#![no_std]
#![no_main]

use libc::{arg, close, kill, open, print, println, read, spawn, unlink, write, O_CREATE, O_RDONLY, O_TRUNC, O_WRONLY};

struct ServiceInfo {
    name: &'static str,
    desc: &'static str,
    is_system: bool,
    default_state: &'static str,
}

const SERVICES: &[ServiceInfo] = &[
    ServiceInfo { name: "init", desc: "System supervisor and PID 1 reaper", is_system: true, default_state: "RUNNING" },
    ServiceInfo { name: "console", desc: "UART0 interactive serial console", is_system: true, default_state: "RUNNING" },
    ServiceInfo { name: "heartbeat", desc: "GPIO2 scheduler proof LED blinker", is_system: true, default_state: "RUNNING" },
    ServiceInfo { name: "vfs", desc: "Virtual File System & flash storage", is_system: true, default_state: "ACTIVE" },
    ServiceInfo { name: "net", desc: "Wi-Fi 802.11 STA & DHCP client", is_system: true, default_state: "RUNNING" },
    ServiceInfo { name: "ssh", desc: "Secure Shell SSH-2.0 daemon (:22)", is_system: true, default_state: "LISTENING" },
    ServiceInfo { name: "httpd", desc: "Embedded HTTP/1.1 metrics web server", is_system: false, default_state: "STOPPED" },
    ServiceInfo { name: "sntp", desc: "Network Time Protocol client", is_system: false, default_state: "ENABLED" },
    ServiceInfo { name: "ble", desc: "Bluetooth Low Energy advertiser", is_system: false, default_state: "READY" },
    ServiceInfo { name: "ota", desc: "Dual-slot A/B update supervisor", is_system: true, default_state: "READY" },
    ServiceInfo { name: "cron", desc: "Periodic task scheduler (/etc/crontab)", is_system: false, default_state: "ENABLED" },
    ServiceInfo { name: "watchdog", desc: "Hardware watchdog timer & health monitor", is_system: true, default_state: "RUNNING" },
];

fn write_pid(name: &str, pid: i32) {
    let mut path_buf = [0u8; 64];
    let prefix = b"/run/";
    let suffix = b".pid";
    let mut i = 0;
    for &b in prefix { path_buf[i] = b; i += 1; }
    for &b in name.as_bytes() { if i < 55 { path_buf[i] = b; i += 1; } }
    for &b in suffix { path_buf[i] = b; i += 1; }
    if let Ok(path) = core::str::from_utf8(&path_buf[..i]) {
        let fd = open(path, O_CREATE | O_WRONLY | O_TRUNC);
        if fd >= 0 {
            let mut pid_buf = [0u8; 16];
            let mut val = pid as usize;
            let mut p_idx = 0;
            if val == 0 {
                pid_buf[0] = b'0';
                p_idx = 1;
            } else {
                let mut tmp = [0u8; 16];
                let mut t_idx = 0;
                while val > 0 && t_idx < 16 {
                    tmp[t_idx] = b'0' + (val % 10) as u8;
                    val /= 10;
                    t_idx += 1;
                }
                while t_idx > 0 {
                    t_idx -= 1;
                    pid_buf[p_idx] = tmp[t_idx];
                    p_idx += 1;
                }
            }
            pid_buf[p_idx] = b'\n';
            p_idx += 1;
            let _ = write(fd as i32, &pid_buf[..p_idx]);
            close(fd as i32);
        }
    }
}

fn read_pid(name: &str) -> Option<i32> {
    let mut path_buf = [0u8; 64];
    let prefix = b"/run/";
    let suffix = b".pid";
    let mut i = 0;
    for &b in prefix { path_buf[i] = b; i += 1; }
    for &b in name.as_bytes() { if i < 55 { path_buf[i] = b; i += 1; } }
    for &b in suffix { path_buf[i] = b; i += 1; }
    if let Ok(path) = core::str::from_utf8(&path_buf[..i]) {
        let fd = open(path, O_RDONLY);
        if fd >= 0 {
            let mut buf = [0u8; 16];
            let n = read(fd as i32, &mut buf);
            close(fd as i32);
            if n > 0 {
                let mut val = 0i32;
                for &b in &buf[..n as usize] {
                    if b >= b'0' && b <= b'9' {
                        val = val * 10 + (b - b'0') as i32;
                    } else if b == b'\n' || b == b' ' {
                        break;
                    }
                }
                if val > 0 {
                    return Some(val);
                }
            }
        }
    }
    None
}

fn remove_pid(name: &str) {
    let mut path_buf = [0u8; 64];
    let prefix = b"/run/";
    let suffix = b".pid";
    let mut i = 0;
    for &b in prefix { path_buf[i] = b; i += 1; }
    for &b in name.as_bytes() { if i < 55 { path_buf[i] = b; i += 1; } }
    for &b in suffix { path_buf[i] = b; i += 1; }
    if let Ok(path) = core::str::from_utf8(&path_buf[..i]) {
        let _ = unlink(path);
    }
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let cmd = if argc > 1 { unsafe { arg(argv, 1) } } else { "list" };

    match cmd {
        "list" => {
            println!("UNIT         STATUS      TYPE        DESCRIPTION");
            println!("-----------------------------------------------------------------");
            for s in SERVICES {
                let typ = if s.is_system { "system  " } else { "optional" };
                let running = read_pid(s.name).is_some();
                let status = if running { "RUNNING (PID)" } else { s.default_state };
                println!("{:<12} {:<15} {}    {}", s.name, status, typ, s.desc);
            }
            0
        }
        "status" => {
            if argc <= 2 {
                println!("UNIT         STATUS      TYPE        DESCRIPTION");
                println!("-----------------------------------------------------------------");
                for s in SERVICES {
                    let typ = if s.is_system { "system  " } else { "optional" };
                    let running = read_pid(s.name).is_some();
                    let status = if running { "RUNNING (PID)" } else { s.default_state };
                    println!("{:<12} {:<15} {}    {}", s.name, status, typ, s.desc);
                }
                return 0;
            }
            let target = unsafe { arg(argv, 2) };
            for s in SERVICES {
                if s.name == target {
                    println!("* {}.service - {}", s.name, s.desc);
                    println!("     Loaded: loaded (/etc/services.d/{}.service)", s.name);
                    if let Some(pid) = read_pid(s.name) {
                        println!("     Active: active (running) PID={}", pid);
                    } else {
                        println!("     Active: active ({})", s.default_state);
                    }
                    println!("     System: {}", if s.is_system { "mandatory kernel service" } else { "user-managed" });
                    return 0;
                }
            }
            println!("service: {}: unit not found", target);
            1
        }
        "start" => {
            if argc < 3 {
                println!("usage: service start <name>");
                return 1;
            }
            let target = unsafe { arg(argv, 2) };
            match target {
                "httpd" => {
                    println!("[service] Starting httpd web server in background...");
                    let argv_child = [b"httpd\0".as_ptr(), core::ptr::null()];
                    let pid = spawn("/bin/httpd", argv_child.as_ptr());
                    if pid >= 0 {
                        write_pid("httpd", pid);
                        println!("[service] httpd started (pid={})", pid);
                        0
                    } else {
                        println!("[service] failed to spawn httpd: {}", pid);
                        1
                    }
                }
                "sntp" => {
                    println!("[service] Running SNTP time synchronization...");
                    let argv_child = [b"sntp\0".as_ptr(), core::ptr::null()];
                    let pid = spawn("/bin/sntp", argv_child.as_ptr());
                    if pid >= 0 {
                        println!("[service] sntp executed (pid={})", pid);
                        0
                    } else {
                        println!("[service] failed to spawn sntp: {}", pid);
                        1
                    }
                }
                "ble" => {
                    println!("[service] Starting BLE advertiser...");
                    let argv_child = [b"ble\0".as_ptr(), b"advertise\0".as_ptr(), core::ptr::null()];
                    let pid = spawn("/bin/ble", argv_child.as_ptr());
                    if pid >= 0 {
                        write_pid("ble", pid);
                        println!("[service] ble started (pid={})", pid);
                        0
                    } else {
                        println!("[service] failed to spawn ble: {}", pid);
                        1
                    }
                }
                "cron" => {
                    println!("[service] Running scheduled cron tasks...");
                    let argv_child = [b"cron\0".as_ptr(), b"run\0".as_ptr(), core::ptr::null()];
                    let pid = spawn("/bin/cron", argv_child.as_ptr());
                    if pid >= 0 {
                        println!("[service] cron executed (pid={})", pid);
                        0
                    } else {
                        println!("[service] failed to spawn cron: {}", pid);
                        1
                    }
                }
                _ => {
                    println!("service: {}: is a static system service or unknown", target);
                    1
                }
            }
        }
        "stop" => {
            if argc < 3 {
                println!("usage: service stop <name>");
                return 1;
            }
            let target = unsafe { arg(argv, 2) };
            if let Some(pid) = read_pid(target) {
                println!("[service] Stopping {} (pid={})...", target, pid);
                let ret = kill(pid as u32, 15);
                remove_pid(target);
                if ret == 0 {
                    println!("[service] {} stopped", target);
                    0
                } else {
                    println!("[service] kill returned {}", ret);
                    1
                }
            } else {
                println!("service: stopping {}: not running or core system unit", target);
                0
            }
        }
        "restart" => {
            if argc < 3 {
                println!("usage: service restart <name>");
                return 1;
            }
            let target = unsafe { arg(argv, 2) };
            if let Some(pid) = read_pid(target) {
                let _ = kill(pid as u32, 15);
                remove_pid(target);
            }
            println!("service: restarting {}...", target);
            0
        }
        _ => {
            println!("Usage: service [list | status <name> | start <name> | stop <name> | restart <name>]");
            1
        }
    }
}
