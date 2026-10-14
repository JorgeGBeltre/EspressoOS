#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read};

const O_RDONLY: u32 = 1;

fn dump_file(header: &str, path: &str) {
    let fd = open(path, O_RDONLY);
    if fd >= 0 {
        let mut buf = [0u8; 1024];
        let n = read(fd as i32, &mut buf);
        close(fd as i32);
        if n > 0 {
            if let Ok(s) = core::str::from_utf8(&buf[..n as usize]) {
                println!("{}", header);
                print!("{}", s);
                println!();
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc > 1 {
        let first = unsafe { arg(argv, 1) };
        if first == "-h" || first == "--help" {
            println!("Usage: lsof");
            println!("List open files, active mounts and network sockets");
            return 0;
        }
    }

    println!("COMMAND    PID   FD  TYPE  DEVICE  NAME");
    println!("kernel       0    0   chr  console /dev/console");
    println!("kernel       0    1   chr  console /dev/console");
    println!("kernel       0    2   chr  console /dev/console");
    println!("init         1    0   chr  console /dev/console");
    println!("init         1    1   chr  console /dev/console");
    println!("init         1    2   chr  console /dev/console");
    println!("sshd         3    3  sock  network :22 (LISTEN)");
    println!();

    dump_file("--- Active Network Sockets (/proc/net/sockets) ---", "/proc/net/sockets");
    dump_file("--- Mounted Filesystems (/proc/mounts) ---", "/proc/mounts");
    0
}
