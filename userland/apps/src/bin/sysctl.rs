#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read, write, O_CREATE, O_RDONLY, O_TRUNC, O_WRONLY};

const PARAMS: &[(&str, &str)] = &[
    ("kernel.hostname", "/proc/sys/kernel/hostname"),
    ("kernel.osrelease", "/proc/sys/kernel/osrelease"),
    ("kernel.ostype", "/proc/sys/kernel/ostype"),
    ("kernel.version", "/proc/sys/kernel/version"),
    ("vm.free_ram", "/proc/sys/vm/free_ram"),
    ("vm.total_ram", "/proc/sys/vm/total_ram"),
];

fn print_param(key: &str, path: &str) {
    let fd = open(path, O_RDONLY);
    if fd >= 0 {
        let mut buf = [0u8; 128];
        let n = read(fd as i32, &mut buf);
        let _ = close(fd as i32);
        if n > 0 {
            if let Ok(s) = core::str::from_utf8(&buf[..n as usize]) {
                let trimmed = s.trim();
                print!("{} = ", key);
                println!("{}", trimmed);
                return;
            }
        }
    }
    print!("{} = ", key);
    println!("<error>");
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        println!("Usage: sysctl [-a] [-w key=value] [key...]");
        return 1;
    }

    let mut i = 1;
    let mut show_all = false;

    while i < argc {
        let a = unsafe { arg(argv, i) };
        if a == "--help" || a == "-h" {
            println!("Usage: sysctl [options] [variable[=value]]...");
            println!("Configure or view kernel parameters at runtime");
            println!("  -a, --all        display all variables");
            println!("  -w, --write      set a variable value (e.g. -w kernel.hostname=name)");
            return 0;
        } else if a == "-a" || a == "--all" {
            show_all = true;
        } else if a == "-w" && i + 1 < argc {
            i += 1;
            let pair = unsafe { arg(argv, i) };
            if let Some(idx) = pair.find('=') {
                let (key, val) = (&pair[..idx], &pair[idx + 1..]);
                let mut found = false;
                for &(k, path) in PARAMS {
                    if k == key {
                        found = true;
                        let fd = open(path, O_WRONLY | O_CREATE | O_TRUNC);
                        if fd >= 0 {
                            let _ = write(fd as i32, val.as_bytes());
                            let _ = close(fd as i32);
                            print!("{} = ", key);
                            println!("{}", val);
                        } else {
                            println!("sysctl: permission denied or parameter not writable: {}", key);
                            return 1;
                        }
                        break;
                    }
                }
                if !found {
                    println!("sysctl: cannot stat {}: No such file or directory", key);
                    return 1;
                }
            } else {
                println!("sysctl: invalid syntax for -w: expected key=value");
                return 1;
            }
        } else if let Some(idx) = a.find('=') {
            let (key, val) = (&a[..idx], &a[idx + 1..]);
            let mut found = false;
            for &(k, path) in PARAMS {
                if k == key {
                    found = true;
                    let fd = open(path, O_WRONLY | O_CREATE | O_TRUNC);
                    if fd >= 0 {
                        let _ = write(fd as i32, val.as_bytes());
                        let _ = close(fd as i32);
                        print!("{} = ", key);
                        println!("{}", val);
                    } else {
                        println!("sysctl: permission denied or parameter not writable: {}", key);
                        return 1;
                    }
                    break;
                }
            }
            if !found {
                println!("sysctl: cannot stat {}: No such file or directory", key);
                return 1;
            }
        } else if !a.starts_with('-') {
            let mut found = false;
            for &(k, path) in PARAMS {
                if k == a {
                    found = true;
                    print_param(k, path);
                    break;
                }
            }
            if !found {
                println!("sysctl: cannot stat {}: No such file or directory", a);
                return 1;
            }
        }
        i += 1;
    }

    if show_all {
        for &(k, path) in PARAMS {
            print_param(k, path);
        }
    }

    0
}
