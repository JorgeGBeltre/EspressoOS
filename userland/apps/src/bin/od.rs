#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read, O_RDONLY};

#[derive(Clone, Copy, PartialEq, Eq)]
enum DumpFormat {
    OctalWord, // -o: 2-byte octal
    OctalByte, // -b: 1-byte octal
    Character, // -c: ASCII character / escape
    HexWord,   // -x: 2-byte hexadecimal
    DecWord,   // -d: 2-byte unsigned decimal
}

fn dump_fd(fd: i32, fmt: DumpFormat) {
    let mut offset: usize = 0;
    let mut buf = [0u8; 16];

    loop {
        let mut read_len = 0;
        while read_len < 16 {
            let n = read(fd, &mut buf[read_len..16]);
            if n <= 0 {
                break;
            }
            read_len += n as usize;
        }

        if read_len == 0 {
            break;
        }

        print!("{:07o}", offset);

        match fmt {
            DumpFormat::OctalWord => {
                let mut i = 0;
                while i < read_len {
                    if i + 1 < read_len {
                        let word = (buf[i] as u16) | ((buf[i + 1] as u16) << 8);
                        print!(" {:06o}", word);
                        i += 2;
                    } else {
                        let word = buf[i] as u16;
                        print!(" {:06o}", word);
                        i += 1;
                    }
                }
            }
            DumpFormat::OctalByte => {
                for i in 0..read_len {
                    print!(" {:03o}", buf[i]);
                }
            }
            DumpFormat::Character => {
                for i in 0..read_len {
                    let b = buf[i];
                    match b {
                        b'\0' => print!("  \\0"),
                        b'\n' => print!("  \\n"),
                        b'\r' => print!("  \\r"),
                        b'\t' => print!("  \\t"),
                        b'\\' => print!("  \\\\"),
                        0x20..=0x7E => print!("   {}", b as char),
                        _ => print!(" {:03o}", b),
                    }
                }
            }
            DumpFormat::HexWord => {
                let mut i = 0;
                while i < read_len {
                    if i + 1 < read_len {
                        let word = (buf[i] as u16) | ((buf[i + 1] as u16) << 8);
                        print!(" {:04x}", word);
                        i += 2;
                    } else {
                        let word = buf[i] as u16;
                        print!(" {:04x}", word);
                        i += 1;
                    }
                }
            }
            DumpFormat::DecWord => {
                let mut i = 0;
                while i < read_len {
                    if i + 1 < read_len {
                        let word = (buf[i] as u16) | ((buf[i + 1] as u16) << 8);
                        print!(" {:05}", word);
                        i += 2;
                    } else {
                        let word = buf[i] as u16;
                        print!(" {:05}", word);
                        i += 1;
                    }
                }
            }
        }
        println!();
        offset += read_len;
    }

    if offset > 0 {
        println!("{:07o}", offset);
    }
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut fmt = DumpFormat::OctalWord;
    let mut files: [&str; 16] = [""; 16];
    let mut file_count = 0;

    let mut i = 1;
    while i < argc {
        let a = unsafe { arg(argv, i) };
        if a == "--help" || a == "-h" {
            println!("Usage: od [OPTION]... [FILE]...");
            println!("Write an unambiguous representation, octal bytes by default, of FILE to standard output.");
            println!("  -b       1-byte octal display");
            println!("  -c       character display");
            println!("  -x       2-byte hexadecimal display");
            println!("  -d       2-byte unsigned decimal display");
            println!("  -o       2-byte octal display (default)");
            return 0;
        } else if a == "-b" {
            fmt = DumpFormat::OctalByte;
        } else if a == "-c" {
            fmt = DumpFormat::Character;
        } else if a == "-x" {
            fmt = DumpFormat::HexWord;
        } else if a == "-d" {
            fmt = DumpFormat::DecWord;
        } else if a == "-o" {
            fmt = DumpFormat::OctalWord;
        } else {
            if file_count < 16 {
                files[file_count] = a;
                file_count += 1;
            }
        }
        i += 1;
    }

    if file_count == 0 {
        dump_fd(0, fmt);
        return 0;
    }

    let mut exit_code = 0;
    for &file in &files[..file_count] {
        if file == "-" {
            dump_fd(0, fmt);
        } else {
            let fd = open(file, O_RDONLY);
            if fd < 0 {
                println!("od: cannot open '{}'", file);
                exit_code = 1;
                continue;
            }
            dump_fd(fd as i32, fmt);
            let _ = close(fd as i32);
        }
    }
    exit_code
}
