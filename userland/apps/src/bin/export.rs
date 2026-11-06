#![no_std]
#![no_main]

use libc::{arg, close, open, println, read, rename, write, O_CREATE, O_RDONLY, O_TRUNC, O_WRONLY};

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut ebuf = [0u8; 1024];
    let mut elen = 0usize;
    let fd = open("/run/sh.env", O_RDONLY);
    if fd >= 0 {
        let n = read(fd as i32, &mut ebuf);
        close(fd as i32);
        if n > 0 {
            elen = n as usize;
        }
    }

    if argc == 1 {
        if let Ok(text) = core::str::from_utf8(&ebuf[..elen]) {
            for line in text.lines() {
                let t = line.trim();
                if !t.is_empty() {
                    println!("export {}", t);
                }
            }
        }
        return 0;
    }

    let first = unsafe { arg(argv, 1) };
    if first == "-h" || first == "--help" {
        println!("Usage: export [NAME[=VALUE]...]");
        return 0;
    }

    for i in 1..argc {
        let spec = unsafe { arg(argv, i) };
        if let Some(eq) = spec.find('=') {
            let key = &spec[..eq];
            let tfd = open("/run/sh.env.tmp", O_WRONLY | O_CREATE | O_TRUNC);
            if tfd >= 0 {
                let mut replaced = false;
                if let Ok(text) = core::str::from_utf8(&ebuf[..elen]) {
                    for line in text.lines() {
                        let t = line.trim();
                        if t.is_empty() {
                            continue;
                        }
                        if t.split('=').next() == Some(key) {
                            let _ = write(tfd as i32, spec.as_bytes());
                            let _ = write(tfd as i32, b"\n");
                            replaced = true;
                        } else {
                            let _ = write(tfd as i32, t.as_bytes());
                            let _ = write(tfd as i32, b"\n");
                        }
                    }
                }
                if !replaced {
                    let _ = write(tfd as i32, spec.as_bytes());
                    let _ = write(tfd as i32, b"\n");
                }
                close(tfd as i32);
                let _ = rename("/run/sh.env.tmp", "/run/sh.env");
                // Reload buffer for subsequent args
                let rfd = open("/run/sh.env", O_RDONLY);
                if rfd >= 0 {
                    let rn = read(rfd as i32, &mut ebuf);
                    close(rfd as i32);
                    elen = if rn > 0 { rn as usize } else { 0 };
                }
            }
        }
    }
    0
}
