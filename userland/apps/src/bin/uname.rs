#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read};

const O_RDONLY: u32 = 1;

fn get_hostname(buf: &mut [u8]) -> &str {
    let fd = open("/etc/hostname", O_RDONLY);
    if fd >= 0 {
        let n = read(fd as i32, buf);
        close(fd as i32);
        if n > 0 {
            let s = core::str::from_utf8(&buf[..n as usize]).unwrap_or("");
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                return trimmed;
            }
        }
    }
    "esp32s3"
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut show_all = false;
    let mut show_sys = false;
    let mut show_nodename = false;
    let mut show_release = false;
    let mut show_version = false;
    let mut show_machine = false;

    if argc < 2 {
        show_sys = true;
    } else {
        let mut i = 1;
        while i < argc {
            let a = unsafe { arg(argv, i) };
            match a {
                "-a" | "--all" => show_all = true,
                "-s" | "--kernel-name" => show_sys = true,
                "-n" | "--nodename" => show_nodename = true,
                "-r" | "--kernel-release" => show_release = true,
                "-v" | "--kernel-version" => show_version = true,
                "-m" | "--machine" => show_machine = true,
                "-h" | "--help" => {
                    println!("Usage: uname [OPTION]...");
                    println!("Print system information.");
                    println!("  -a, --all                print all information");
                    println!("  -s, --kernel-name        print the kernel name");
                    println!("  -n, --nodename           print the network node hostname");
                    println!("  -r, --kernel-release     print the kernel release");
                    println!("  -v, --kernel-version     print the kernel version");
                    println!("  -m, --machine            print the machine hardware architecture");
                    return 0;
                }
                _ => {
                    println!("uname: invalid option '{}'. Try 'uname --help'", a);
                    return 1;
                }
            }
            i += 1;
        }
    }

    if show_all {
        show_sys = true;
        show_nodename = true;
        show_release = true;
        show_version = true;
        show_machine = true;
    }

    let mut first = true;
    let mut host_buf = [0u8; 64];

    if show_sys {
        if !first { print!(" "); }
        print!("EspressoOS");
        first = false;
    }
    if show_nodename {
        if !first { print!(" "); }
        let h = get_hostname(&mut host_buf);
        print!("{}", h);
        first = false;
    }
    if show_release {
        if !first { print!(" "); }
        print!("0.2.0-dev");
        first = false;
    }
    if show_version {
        if !first { print!(" "); }
        print!("#1 PREEMPT Xtensa-LX7-240MHz");
        first = false;
    }
    if show_machine {
        if !first { print!(" "); }
        print!("xtensa-esp32s3");
    }
    println!("");
    0
}
