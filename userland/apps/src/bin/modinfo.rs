#![no_std]
#![no_main]

use libc::{arg, print, println};

struct ModMetadata {
    name: &'static str,
    desc: &'static str,
    author: &'static str,
    license: &'static str,
    version: &'static str,
}

const BUILTIN_MODULES: &[ModMetadata] = &[
    ModMetadata {
        name: "espfs",
        desc: "Log-structured persistent filesystem on external NOR flash",
        author: "EspressoOS Core Team",
        license: "MIT",
        version: "0.2.0",
    },
    ModMetadata {
        name: "procfs",
        desc: "Virtual process and system state introspection filesystem",
        author: "EspressoOS Core Team",
        license: "MIT",
        version: "0.2.0",
    },
    ModMetadata {
        name: "sysfs",
        desc: "Kernel device and subsystem hierarchy filesystem",
        author: "EspressoOS Core Team",
        license: "MIT",
        version: "0.2.0",
    },
    ModMetadata {
        name: "devfs",
        desc: "Device nodes filesystem mounted at /dev",
        author: "EspressoOS Core Team",
        license: "MIT",
        version: "0.2.0",
    },
    ModMetadata {
        name: "ramfs",
        desc: "Volatile RAM scratch filesystem mounted at /tmp",
        author: "EspressoOS Core Team",
        license: "MIT",
        version: "0.2.0",
    },
    ModMetadata {
        name: "littlefs",
        desc: "LittleFS secondary storage resilient filesystem",
        author: "ARM / EspressoOS Port",
        license: "BSD-3-Clause",
        version: "2.5.1",
    },
    ModMetadata {
        name: "esp_wifi",
        desc: "Espressif Wi-Fi STA driver with smoltcp TCP/IP stack",
        author: "Espressif Systems / EspressoOS",
        license: "MIT OR Apache-2.0",
        version: "0.23.0",
    },
    ModMetadata {
        name: "ssh_server",
        desc: "SSH-2.0 hardened secure shell server daemon",
        author: "EspressoOS Security Team",
        license: "MIT",
        version: "2.0.0",
    },
    ModMetadata {
        name: "psram_exec",
        desc: "8MB Octal PSRAM execution pool allocator and literal relocations",
        author: "EspressoOS MM Team",
        license: "MIT",
        version: "0.2.0",
    },
    ModMetadata {
        name: "systimer",
        desc: "Xtensa SYSTIMER 100 Hz preemptive multitasking scheduler timer",
        author: "EspressoOS Arch Team",
        license: "MIT",
        version: "0.2.0",
    },
    ModMetadata {
        name: "wdt",
        desc: "Hardware watchdog timer and recovery supervisor",
        author: "EspressoOS Driver Team",
        license: "MIT",
        version: "0.2.0",
    },
    ModMetadata {
        name: "gpio",
        desc: "GPIO matrix pin configuration and digital I/O driver",
        author: "EspressoOS Driver Team",
        license: "MIT",
        version: "0.2.0",
    },
    ModMetadata {
        name: "i2c0",
        desc: "I2C master bus peripheral driver",
        author: "EspressoOS Driver Team",
        license: "MIT",
        version: "0.2.0",
    },
    ModMetadata {
        name: "spi2",
        desc: "SPI master bus peripheral driver",
        author: "EspressoOS Driver Team",
        license: "MIT",
        version: "0.2.0",
    },
    ModMetadata {
        name: "ble0",
        desc: "Bluetooth Low Energy peripheral advertiser driver",
        author: "Espressif Systems / EspressoOS",
        license: "MIT OR Apache-2.0",
        version: "0.23.0",
    },
];

fn print_info(meta: &ModMetadata, field_filter: Option<&str>) {
    if let Some(field) = field_filter {
        if field == "filename" {
            println!("(builtin)");
        } else if field == "description" {
            println!("{}", meta.desc);
        } else if field == "author" {
            println!("{}", meta.author);
        } else if field == "license" {
            println!("{}", meta.license);
        } else if field == "version" {
            println!("{}", meta.version);
        } else if field == "name" {
            println!("{}", meta.name);
        }
        return;
    }

    println!("filename:       (builtin)");
    println!("description:    {}", meta.desc);
    println!("author:         {}", meta.author);
    println!("license:        {}", meta.license);
    println!("version:        {}", meta.version);
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        println!("Usage: modinfo [-F field] MODULE...");
        println!("Show information about a kernel module");
        println!("  -F, --field FIELD  Print only the specified field value");
        println!("  -h, --help         Display this help");
        return 1;
    }

    let mut field_filter: Option<&str> = None;
    let mut i = 1;

    while i < argc {
        let s = unsafe { arg(argv, i) };
        if s == "-h" || s == "--help" {
            println!("Usage: modinfo [-F field] MODULE...");
            println!("Show information about a kernel module");
            println!("  -F, --field FIELD  Print only the specified field value");
            println!("  -h, --help         Display this help");
            return 0;
        } else if s == "-F" || s == "--field" {
            if i + 1 < argc {
                field_filter = Some(unsafe { arg(argv, i + 1) });
                i += 2;
            } else {
                println!("modinfo: option requires an argument -- 'F'");
                return 1;
            }
        } else if s.starts_with("-F") && s.len() > 2 {
            field_filter = Some(&s[2..]);
            i += 1;
        } else {
            break;
        }
    }

    if i >= argc {
        println!("modinfo: missing module name");
        return 1;
    }

    let mut exit_code = 0;
    while i < argc {
        let mod_target = unsafe { arg(argv, i) };
        let mut found = false;
        for meta in BUILTIN_MODULES {
            if meta.name == mod_target {
                print_info(meta, field_filter);
                found = true;
                break;
            }
        }
        if !found {
            print!("modinfo: ERROR: Module ");
            print!("{}", mod_target);
            println!(" not found.");
            exit_code = 1;
        }
        i += 1;
    }

    exit_code
}
