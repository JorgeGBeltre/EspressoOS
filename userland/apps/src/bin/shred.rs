#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read, seek, unlink, write, O_RDWR};

fn parse_u32(s: &str) -> Option<u32> {
    if s.is_empty() {
        return None;
    }
    let mut val = 0u32;
    for &b in s.as_bytes() {
        if b < b'0' || b > b'9' {
            return None;
        }
        val = val.saturating_mul(10).saturating_add((b - b'0') as u32);
    }
    Some(val)
}

fn fill_random(buf: &mut [u8], rng_seed: &mut u32) {
    let fd = open("/dev/urandom", libc::O_RDONLY) as i32;
    if fd >= 0 {
        let n = read(fd, buf);
        close(fd);
        if n == buf.len() as isize {
            return;
        }
    }

    // Fallback: Xorshift32 PRNG
    for b in buf.iter_mut() {
        let mut x = *rng_seed;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        *rng_seed = x;
        *b = (x & 0xff) as u8;
    }
}

fn shred_file(path: &str, passes: u32, zero_pass: bool, remove_file: bool, verbose: bool) -> i32 {
    let fd = open(path, O_RDWR) as i32;
    if fd < 0 {
        print!("shred: ");
        print!("{}", path);
        println!(": failed to open for writing");
        return 1;
    }

    // Determine file size via seek to end
    let file_len = seek(fd, 0, 2);
    if file_len < 0 {
        close(fd);
        print!("shred: ");
        print!("{}", path);
        println!(": failed to determine file size");
        return 1;
    }
    let len = file_len as usize;

    let mut buf = [0u8; 512];
    let mut seed = 0x12345678u32 ^ (len as u32);

    let total_passes = passes + if zero_pass { 1 } else { 0 };

    for p in 1..=passes {
        if verbose {
            print!("shred: {}: pass {}/{} (random)...\n", path, p, total_passes);
        }

        let _ = seek(fd, 0, 0);
        let mut written = 0;
        while written < len {
            let chunk = core::cmp::min(buf.len(), len - written);
            fill_random(&mut buf[..chunk], &mut seed);
            let n = write(fd, &buf[..chunk]);
            if n <= 0 {
                break;
            }
            written += n as usize;
        }
    }

    if zero_pass {
        if verbose {
            print!("shred: {}: pass {}/{} (000000)...\n", path, total_passes, total_passes);
        }

        let _ = seek(fd, 0, 0);
        for b in buf.iter_mut() {
            *b = 0;
        }
        let mut written = 0;
        while written < len {
            let chunk = core::cmp::min(buf.len(), len - written);
            let n = write(fd, &buf[..chunk]);
            if n <= 0 {
                break;
            }
            written += n as usize;
        }
    }

    close(fd);

    if remove_file {
        if verbose {
            print!("shred: {}: removing\n", path);
        }
        if unlink(path) < 0 {
            print!("shred: {}: failed to remove\n", path);
            return 1;
        }
    }

    0
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut passes = 3u32;
    let mut zero_pass = false;
    let mut remove_file = false;
    let mut verbose = false;
    let mut files = [core::ptr::null::<u8>(); 16];
    let mut num_files = 0;

    let mut i = 1;
    while i < argc {
        let arg_str = unsafe { arg(argv, i) };
        if arg_str == "-u" || arg_str == "--remove" {
            remove_file = true;
        } else if arg_str == "-z" || arg_str == "--zero" {
            zero_pass = true;
        } else if arg_str == "-v" || arg_str == "--verbose" {
            verbose = true;
        } else if arg_str == "-n" || arg_str == "--iterations" {
            if i + 1 < argc {
                i += 1;
                let n_str = unsafe { arg(argv, i) };
                if let Some(n) = parse_u32(n_str) {
                    passes = n;
                }
            }
        } else if arg_str == "-h" || arg_str == "--help" {
            println!("Usage: shred [OPTION]... FILE...");
            println!("Overwrite the specified FILE(s) repeatedly, in order to make it harder");
            println!("for even very expensive hardware probing to recover the data.");
            println!("  -n, --iterations=N  overwrite N times instead of the default (3)");
            println!("  -u, --remove        truncate and remove file after overwriting");
            println!("  -v, --verbose       show progress");
            println!("  -z, --zero          add a final overwrite with zeros to hide shredding");
            return 0;
        } else if !arg_str.starts_with('-') {
            if num_files < files.len() {
                unsafe {
                    files[num_files] = *argv.offset(i as isize);
                    num_files += 1;
                }
            }
        }
        i += 1;
    }

    if num_files == 0 {
        println!("shred: missing file operand");
        println!("Usage: shred [OPTION]... FILE...");
        return 1;
    }

    let mut exit_code = 0;
    for idx in 0..num_files {
        let path_ptr = files[idx];
        let path_str = unsafe {
            let mut len = 0;
            while *path_ptr.add(len) != 0 {
                len += 1;
            }
            core::str::from_utf8_unchecked(core::slice::from_raw_parts(path_ptr, len))
        };

        let ret = shred_file(path_str, passes, zero_pass, remove_file, verbose);
        if ret != 0 {
            exit_code = 1;
        }
    }

    exit_code
}
