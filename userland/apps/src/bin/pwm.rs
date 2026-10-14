#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read, write};

const O_RDONLY: u32 = 1;
const O_WRONLY: u32 = 2;

fn show_status() -> i32 {
    let fd = open("/dev/pwm0", O_RDONLY);
    if fd < 0 {
        println!("pwm: cannot open /dev/pwm0");
        return 1;
    }
    let mut buf = [0u8; 1024];
    let n = read(fd as i32, &mut buf);
    close(fd as i32);
    if n > 0 {
        let s = core::str::from_utf8(&buf[..n as usize]).unwrap_or("");
        print!("{}", s);
        0
    } else {
        println!("pwm: empty read from /dev/pwm0");
        1
    }
}

fn send_cmd(cmd: &str) -> i32 {
    let fd = open("/dev/pwm0", O_WRONLY);
    if fd < 0 {
        println!("pwm: cannot open /dev/pwm0 for writing");
        return 1;
    }
    let n = write(fd as i32, cmd.as_bytes());
    close(fd as i32);
    if n > 0 {
        0
    } else {
        println!("pwm: write command failed");
        1
    }
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        return show_status();
    }

    let verb = unsafe { arg(argv, 1) };
    match verb {
        "-h" | "--help" | "help" => {
            println!("Usage: pwm <command> [args...]");
            println!("ESP32-S3 LEDC Hardware PWM Controller (8 channels, 0-7)");
            println!("Commands:");
            println!("  status                        Show status of all 8 PWM channels");
            println!("  config <ch> <pin> <hz> <duty> Configure channel pin, frequency and duty (0-100%)");
            println!("  duty <ch> <duty>              Update duty cycle (0-100%)");
            println!("  disable <ch>, off <ch>        Disable PWM channel");
            0
        }
        "status" => show_status(),
        "config" | "set" => {
            if argc < 6 {
                println!("Usage: pwm config <ch> <pin> <freq_hz> <duty_percent>");
                return 1;
            }
            let ch = unsafe { arg(argv, 2) };
            let pin = unsafe { arg(argv, 3) };
            let freq = unsafe { arg(argv, 4) };
            let duty = unsafe { arg(argv, 5) };

            let mut cmd = [0u8; 64];
            let mut idx = 0;
            for &s in &[b"config ", ch.as_bytes(), b" ", pin.as_bytes(), b" ", freq.as_bytes(), b" ", duty.as_bytes()] {
                for &b in s {
                    if idx < cmd.len() - 1 {
                        cmd[idx] = b;
                        idx += 1;
                    }
                }
            }
            if let Ok(cmd_str) = core::str::from_utf8(&cmd[..idx]) {
                if send_cmd(cmd_str) == 0 {
                    println!("pwm: ch {} configured on pin {} @ {}Hz, {}% duty", ch, pin, freq, duty);
                    0
                } else {
                    1
                }
            } else {
                1
            }
        }
        "duty" => {
            if argc < 4 {
                println!("Usage: pwm duty <ch> <duty_percent>");
                return 1;
            }
            let ch = unsafe { arg(argv, 2) };
            let duty = unsafe { arg(argv, 3) };

            let mut cmd = [0u8; 32];
            let mut idx = 0;
            for &s in &[b"duty ", ch.as_bytes(), b" ", duty.as_bytes()] {
                for &b in s {
                    if idx < cmd.len() - 1 {
                        cmd[idx] = b;
                        idx += 1;
                    }
                }
            }
            if let Ok(cmd_str) = core::str::from_utf8(&cmd[..idx]) {
                if send_cmd(cmd_str) == 0 {
                    println!("pwm: ch {} duty set to {}%", ch, duty);
                    0
                } else {
                    1
                }
            } else {
                1
            }
        }
        "disable" | "off" | "stop" => {
            if argc < 3 {
                println!("Usage: pwm disable <ch>");
                return 1;
            }
            let ch = unsafe { arg(argv, 2) };
            let mut cmd = [0u8; 32];
            let mut idx = 0;
            for &s in &[b"disable ", ch.as_bytes()] {
                for &b in s {
                    if idx < cmd.len() - 1 {
                        cmd[idx] = b;
                        idx += 1;
                    }
                }
            }
            if let Ok(cmd_str) = core::str::from_utf8(&cmd[..idx]) {
                if send_cmd(cmd_str) == 0 {
                    println!("pwm: ch {} disabled", ch);
                    0
                } else {
                    1
                }
            } else {
                1
            }
        }
        _ => {
            println!("pwm: unknown command '{}'", verb);
            println!("Try 'pwm help' for usage.");
            1
        }
    }
}
