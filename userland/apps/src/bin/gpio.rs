#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read, write};

const O_RDONLY: u32 = 1;
const O_WRONLY: u32 = 2;

fn show_status() -> i32 {
    let fd = open("/dev/gpio", O_RDONLY);
    if fd < 0 {
        println!("gpio: cannot open /dev/gpio");
        return 1;
    }
    let mut buf = [0u8; 128];
    let n = read(fd as i32, &mut buf);
    close(fd as i32);
    if n > 0 {
        let s = core::str::from_utf8(&buf[..n as usize]).unwrap_or("");
        print!("{}", s);
        0
    } else {
        println!("gpio: empty read from /dev/gpio");
        1
    }
}

fn send_cmd(cmd: &str) -> i32 {
    let fd = open("/dev/gpio", O_WRONLY);
    if fd < 0 {
        println!("gpio: cannot open /dev/gpio for writing");
        return 1;
    }
    let n = write(fd as i32, cmd.as_bytes());
    close(fd as i32);
    if n > 0 {
        0
    } else {
        println!("gpio: write command failed");
        1
    }
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        println!("Usage: gpio <mode|write|read|toggle|status> [pin] [val]");
        println!("Try 'gpio help' for more information.");
        return 1;
    }

    let verb = unsafe { arg(argv, 1) };
    match verb {
        "status" => show_status(),
        "mode" => {
            if argc < 4 {
                println!("Usage: gpio mode <pin> <in|out>");
                return 1;
            }
            let pin = unsafe { arg(argv, 2) };
            let mode = unsafe { arg(argv, 3) };
            let mut cmd_buf = [0u8; 32];
            let mut pos = 0;
            for b in b"mode " { cmd_buf[pos] = *b; pos += 1; }
            for b in pin.bytes() { if pos < 30 { cmd_buf[pos] = b; pos += 1; } }
            cmd_buf[pos] = b' '; pos += 1;
            for b in mode.bytes() { if pos < 31 { cmd_buf[pos] = b; pos += 1; } }
            if let Ok(cmd_str) = core::str::from_utf8(&cmd_buf[..pos]) {
                if send_cmd(cmd_str) == 0 {
                    println!("gpio: pin {} configured as {}", pin, mode);
                    0
                } else {
                    1
                }
            } else {
                1
            }
        }
        "write" => {
            if argc < 4 {
                println!("Usage: gpio write <pin> <0|1>");
                return 1;
            }
            let pin = unsafe { arg(argv, 2) };
            let val = unsafe { arg(argv, 3) };
            let mut cmd_buf = [0u8; 32];
            let mut pos = 0;
            for b in b"write " { cmd_buf[pos] = *b; pos += 1; }
            for b in pin.bytes() { if pos < 30 { cmd_buf[pos] = b; pos += 1; } }
            cmd_buf[pos] = b' '; pos += 1;
            for b in val.bytes() { if pos < 31 { cmd_buf[pos] = b; pos += 1; } }
            if let Ok(cmd_str) = core::str::from_utf8(&cmd_buf[..pos]) {
                if send_cmd(cmd_str) == 0 {
                    println!("gpio: pin {} set to {}", pin, val);
                    0
                } else {
                    1
                }
            } else {
                1
            }
        }
        "toggle" => {
            if argc < 3 {
                println!("Usage: gpio toggle <pin>");
                return 1;
            }
            let pin = unsafe { arg(argv, 2) };
            let mut cmd_buf = [0u8; 32];
            let mut pos = 0;
            for b in b"toggle " { cmd_buf[pos] = *b; pos += 1; }
            for b in pin.bytes() { if pos < 31 { cmd_buf[pos] = b; pos += 1; } }
            if let Ok(cmd_str) = core::str::from_utf8(&cmd_buf[..pos]) {
                if send_cmd(cmd_str) == 0 {
                    println!("gpio: pin {} toggled", pin);
                    0
                } else {
                    1
                }
            } else {
                1
            }
        }
        "help" | "--help" | "-h" => {
            println!("Usage: gpio <command> [arguments]");
            println!("Commands:");
            println!("  mode <pin> <in|out>    Configure pin direction");
            println!("  write <pin> <0|1>      Set pin output level");
            println!("  toggle <pin>           Toggle pin output level");
            println!("  status                 Show GPIO output mask registers");
            println!("  help                   Show this help message");
            println!("\nValid pins on ESP32-S3: 0 to 48");
            0
        }
        _ => {
            println!("gpio: unknown command '{}'. Try 'gpio help'", verb);
            1
        }
    }
}
