#![no_std]
#![no_main]

use libc::{close, open, print, println, read, readdir, O_RDONLY};

#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    println!("------ EspressoOS Inter-Process Communication Facilities ------");

    // 1. Inspect /proc/ipc
    let fd_ipc = open("/proc/ipc", O_RDONLY);
    if fd_ipc >= 0 {
        let mut buf = [0u8; 512];
        let n = read(fd_ipc as i32, &mut buf);
        if n > 0 {
            for &b in &buf[..n as usize] {
                print!("{}", b as char);
            }
        }
        close(fd_ipc as i32);
    }

    println!("");
    println!("------ Named Pipes (FIFOs) in /tmp ------");
    let mut fifo_count = 0;
    let mut dir_buf = [0u8; 1024];
    let n = readdir("/tmp", &mut dir_buf);
    if n > 0 {
        let limit = n as usize;
        let mut pos = 0;
        while pos + 11 <= limit {
            let ino = u64::from_le_bytes([
                dir_buf[pos],
                dir_buf[pos + 1],
                dir_buf[pos + 2],
                dir_buf[pos + 3],
                dir_buf[pos + 4],
                dir_buf[pos + 5],
                dir_buf[pos + 6],
                dir_buf[pos + 7],
            ]);
            let kind = dir_buf[pos + 8];
            let name_len = u16::from_le_bytes([dir_buf[pos + 9], dir_buf[pos + 10]]) as usize;
            pos += 11;

            if pos + name_len > limit {
                break;
            }

            if let Ok(name) = core::str::from_utf8(&dir_buf[pos..pos + name_len]) {
                if kind == 5 {
                    // kind == 5 is InodeKind::Fifo
                    println!("fifo: /tmp/{:<20} ino={}", name, ino);
                    fifo_count += 1;
                }
            }
            pos += name_len;
        }
    }

    if fifo_count == 0 {
        println!("No active named pipes detected in /tmp");
    }

    println!("");
    println!("------ Network Sockets & Stream Endpoints ------");
    let fd_sock = open("/proc/net/sockets", O_RDONLY);
    if fd_sock >= 0 {
        let mut sbuf = [0u8; 512];
        let n = read(fd_sock as i32, &mut sbuf);
        if n > 0 {
            for &b in &sbuf[..n as usize] {
                print!("{}", b as char);
            }
        }
        close(fd_sock as i32);
    }

    0
}
