#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read, O_RDONLY};

fn is_ascii_text(buf: &[u8]) -> (bool, bool) {
    if buf.is_empty() {
        return (true, false);
    }
    let mut has_crlf = false;
    for (i, &b) in buf.iter().enumerate() {
        if b == 0 {
            return (false, false);
        }
        if b == b'\r' && i + 1 < buf.len() && buf[i + 1] == b'\n' {
            has_crlf = true;
        }
        if b < 0x20 && b != b'\t' && b != b'\n' && b != b'\r' && b != 0x0c {
            return (false, false);
        }
    }
    (true, has_crlf)
}

fn classify_content(buf: &[u8]) -> (&'static str, &'static str) {
    if buf.is_empty() {
        return ("empty", "");
    }

    // 1. ELF binary
    if buf.starts_with(b"\x7fELF") {
        if buf.len() >= 20 {
            let machine = (buf[18] as u16) | ((buf[19] as u16) << 8);
            if machine == 0x005e {
                return ("ELF 32-bit LSB executable, Tensilica Xtensa, version 1 (SYSV)", "");
            } else if machine == 0x00f3 {
                return ("ELF 32-bit LSB executable, UCB RISC-V, version 1 (SYSV)", "");
            } else if machine == 0x0028 {
                return ("ELF 32-bit LSB executable, ARM, version 1 (SYSV)", "");
            } else {
                return ("ELF 32-bit LSB executable, version 1 (SYSV)", "");
            }
        }
        return ("ELF executable", "");
    }

    // 2. Shebang script
    if buf.starts_with(b"#!") {
        if buf.len() >= 9 && buf[2..].starts_with(b"/bin/sh") {
            return ("POSIX shell script, ASCII text executable", "");
        }
        return ("script executable, ASCII text", "");
    }

    // 3. Gzip archive
    if buf.starts_with(b"\x1f\x8b") {
        return ("gzip compressed data", "");
    }

    // 4. Tar archive (POSIX ustar magic at offset 257)
    if buf.len() >= 263 && &buf[257..263] == b"ustar\0" {
        return ("POSIX tar archive", "");
    }
    if buf.len() >= 262 && &buf[257..262] == b"ustar" {
        return ("POSIX tar archive", "");
    }

    // 5. Text classification
    let (is_text, has_crlf) = is_ascii_text(buf);
    if is_text {
        let mut trimmed = buf;
        while !trimmed.is_empty() && (trimmed[0] == b' ' || trimmed[0] == b'\t' || trimmed[0] == b'\r' || trimmed[0] == b'\n') {
            trimmed = &trimmed[1..];
        }

        let suffix = if has_crlf { ", with CRLF line terminators" } else { "" };

        if trimmed.starts_with(b"{") || trimmed.starts_with(b"[") {
            return ("JSON data", suffix);
        }
        if trimmed.starts_with(b"<html") || trimmed.starts_with(b"<!DOCTYPE") || trimmed.starts_with(b"<?xml") {
            return ("HTML/XML document text", suffix);
        }
        return ("ASCII text", suffix);
    }

    ("data", "")
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut brief = false;
    let mut files = [core::ptr::null::<u8>(); 32];
    let mut num_files = 0;

    for i in 1..argc {
        let arg_str = unsafe { arg(argv, i) };
        if arg_str == "-b" || arg_str == "--brief" {
            brief = true;
        } else if arg_str == "-h" || arg_str == "--help" {
            println!("Usage: file [-b] [file...]");
            println!("Determine file type based on headers and magic bytes");
            println!("  -b, --brief    do not prepend filenames to output lines");
            return 0;
        } else if !arg_str.starts_with('-') {
            if num_files < files.len() {
                unsafe {
                    files[num_files] = *argv.offset(i as isize);
                    num_files += 1;
                }
            }
        }
    }

    if num_files == 0 {
        let mut buf = [0u8; 512];
        let n = read(0, &mut buf);
        if n >= 0 {
            let (desc, suffix) = classify_content(&buf[..n as usize]);
            if !suffix.is_empty() {
                print!("{}", desc);
                println!("{}", suffix);
            } else {
                println!("{}", desc);
            }
            return 0;
        } else {
            println!("Usage: file [-b] [file...]");
            return 1;
        }
    }

    let mut exit_code = 0;
    for i in 0..num_files {
        let path_ptr = files[i];
        let path_str = unsafe {
            let mut len = 0;
            while *path_ptr.add(len) != 0 {
                len += 1;
            }
            core::str::from_utf8_unchecked(core::slice::from_raw_parts(path_ptr, len))
        };

        let fd = open(path_str, O_RDONLY) as i32;
        if fd < 0 {
            print!("file: ");
            print!("{}", path_str);
            println!(": cannot open (No such file or directory)");
            exit_code = 1;
            continue;
        }

        let mut buf = [0u8; 512];
        let n = read(fd, &mut buf);
        close(fd);

        let (desc, suffix) = if n <= 0 {
            ("empty", "")
        } else {
            classify_content(&buf[..n as usize])
        };

        if !brief {
            print!("{}: ", path_str);
        }
        print!("{}", desc);
        if !suffix.is_empty() {
            print!("{}", suffix);
        }
        println!();
    }

    exit_code
}
