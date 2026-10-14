#![no_std]
#![no_main]

use libc::{arg, println, spawn};

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

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let cmd = if argc > 1 { unsafe { arg(argv, 1) } } else { "list" };

    match cmd {
        "list" => {
            println!("UNIT         STATUS      TYPE        DESCRIPTION");
            println!("-----------------------------------------------------------------");
            for s in SERVICES {
                let typ = if s.is_system { "system  " } else { "optional" };
                println!("{:<12} {:<11} {}    {}", s.name, s.default_state, typ, s.desc);
            }
            0
        }
        "status" => {
            if argc <= 2 {
                println!("UNIT         STATUS      TYPE        DESCRIPTION");
                println!("-----------------------------------------------------------------");
                for s in SERVICES {
                    let typ = if s.is_system { "system  " } else { "optional" };
                    println!("{:<12} {:<11} {}    {}", s.name, s.default_state, typ, s.desc);
                }
                return 0;
            }
            let target = unsafe { arg(argv, 2) };
            for s in SERVICES {
                if s.name == target {
                    println!("* {}.service - {}", s.name, s.desc);
                    println!("     Loaded: loaded (/etc/services.conf)");
                    println!("     Active: active ({})", s.default_state);
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
            println!("service: stopping {}: not allowed for core system unit or already stopped", target);
            0
        }
        "restart" => {
            if argc < 3 {
                println!("usage: service restart <name>");
                return 1;
            }
            let target = unsafe { arg(argv, 2) };
            println!("service: restarting {}...", target);
            0
        }
        _ => {
            println!("Usage: service [list | status <name> | start <name> | stop <name> | restart <name>]");
            1
        }
    }
}
