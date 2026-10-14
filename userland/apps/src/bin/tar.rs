#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read, write, O_CREATE, O_RDONLY, O_TRUNC, O_WRONLY};

#[repr(C)]
struct TarHeader {
    name: [u8; 100],
    mode: [u8; 8],
    uid: [u8; 8],
    gid: [u8; 8],
    size: [u8; 12],
    mtime: [u8; 12],
    chksum: [u8; 8],
    typeflag: u8,
    linkname: [u8; 100],
    magic: [u8; 6],
    version: [u8; 2],
    uname: [u8; 32],
    gname: [u8; 32],
    devmajor: [u8; 8],
    devminor: [u8; 8],
    prefix: [u8; 155],
    pad: [u8; 12],
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 3 {
        println!("usage: tar -c <archive.tar> <files...>");
        println!("       tar -t <archive.tar>");
        println!("       tar -x <archive.tar>");
        return 1;
    }

    let mode = unsafe { arg(argv, 1) };
    let archive_path = unsafe { arg(argv, 2) };

    match mode {
        "-c" => {
            if argc < 4 {
                println!("tar: no files specified to archive");
                return 1;
            }
            create_archive(archive_path, argc, argv)
        }
        "-t" => list_archive(archive_path),
        "-x" => extract_archive(archive_path),
        _ => {
            println!("tar: unsupported mode '{}'", mode);
            1
        }
    }
}

fn create_archive(archive_path: &str, argc: i32, argv: *const *const u8) -> i32 {
    let out_fd = open(archive_path, O_CREATE | O_WRONLY | O_TRUNC);
    if out_fd < 0 {
        println!("tar: cannot create archive {}", archive_path);
        return 1;
    }

    let mut header = [0u8; 512];
    let mut file_buf = [0u8; 512];

    for i in 3..argc {
        let file_path = unsafe { arg(argv, i) };
        let in_fd = open(file_path, O_RDONLY);
        if in_fd < 0 {
            println!("tar: warning: cannot open file {}", file_path);
            continue;
        }

        // Determine file size
        let mut total_size = 0usize;
        loop {
            let n = read(in_fd as i32, &mut file_buf);
            if n <= 0 {
                break;
            }
            total_size += n as usize;
        }
        close(in_fd as i32);

        // Reopen to read data
        let in_fd = open(file_path, O_RDONLY);
        if in_fd < 0 {
            continue;
        }

        // Clear header
        for b in &mut header {
            *b = 0;
        }

        // Fill name (0..100)
        let name_bytes = file_path.as_bytes();
        let nlen = name_bytes.len().min(99);
        header[..nlen].copy_from_slice(&name_bytes[..nlen]);

        // Mode: 0000644\0 (100..108)
        header[100..108].copy_from_slice(b"0000644\0");
        // UID / GID
        header[108..116].copy_from_slice(b"0001750\0");
        header[116..124].copy_from_slice(b"0001750\0");

        // Format size in octal (124..136)
        let mut sz = total_size;
        for pos in (124..135).rev() {
            header[pos] = b'0' + (sz % 8) as u8;
            sz /= 8;
        }
        header[135] = 0;

        // Magic: ustar\0 (257..263)
        header[257..263].copy_from_slice(b"ustar\0");
        header[263..265].copy_from_slice(b"00");

        // Checksum calculation (148..156 treated as spaces)
        for pos in 148..156 {
            header[pos] = b' ';
        }
        let mut chk: u32 = 0;
        for &b in &header {
            chk += b as u32;
        }
        for pos in (148..154).rev() {
            header[pos] = b'0' + (chk % 8) as u8;
            chk /= 8;
        }
        header[154] = 0;
        header[155] = b' ';

        // Write header
        let _ = write(out_fd as i32, &header);

        // Write file payload padded to 512
        let mut written_bytes = 0usize;
        loop {
            let n = read(in_fd as i32, &mut file_buf);
            if n <= 0 {
                break;
            }
            let _ = write(out_fd as i32, &file_buf[..n as usize]);
            written_bytes += n as usize;
        }
        close(in_fd as i32);

        // Padding to 512
        let rem = written_bytes % 512;
        if rem != 0 {
            let pad = [0u8; 512];
            let _ = write(out_fd as i32, &pad[..(512 - rem)]);
        }

        println!("tar: added {}", file_path);
    }

    // Two 512-byte zero blocks at end of tar
    let zeros = [0u8; 1024];
    let _ = write(out_fd as i32, &zeros);
    close(out_fd as i32);

    println!("tar: archive created {}", archive_path);
    0
}

fn list_archive(archive_path: &str) -> i32 {
    let fd = open(archive_path, O_RDONLY);
    if fd < 0 {
        println!("tar: cannot open {}", archive_path);
        return 1;
    }

    let mut header = [0u8; 512];
    println!("PERMS     SIZE  NAME");
    println!("--------------------------------------------------");

    loop {
        let n = read(fd as i32, &mut header);
        if n < 512 || header[0] == 0 {
            break;
        }

        let name_end = header[..100].iter().position(|&b| b == 0).unwrap_or(100);
        let name = core::str::from_utf8(&header[..name_end]).unwrap_or("?");

        // Parse octal size
        let mut size = 0usize;
        for &b in &header[124..135] {
            if (b'0'..=b'7').contains(&b) {
                size = size * 8 + (b - b'0') as usize;
            }
        }

        println!("-rw-r--r-- {:5}  {}", size, name);

        // Skip payload + padding
        let blocks = (size + 511) / 512;
        for _ in 0..blocks {
            let mut discard = [0u8; 512];
            let _ = read(fd as i32, &mut discard);
        }
    }

    close(fd as i32);
    0
}

fn extract_archive(archive_path: &str) -> i32 {
    let fd = open(archive_path, O_RDONLY);
    if fd < 0 {
        println!("tar: cannot open {}", archive_path);
        return 1;
    }

    let mut header = [0u8; 512];
    let mut block = [0u8; 512];

    loop {
        let n = read(fd as i32, &mut header);
        if n < 512 || header[0] == 0 {
            break;
        }

        let name_end = header[..100].iter().position(|&b| b == 0).unwrap_or(100);
        let name = match core::str::from_utf8(&header[..name_end]) {
            Ok(s) => s,
            Err(_) => {
                println!("tar: invalid filename in header");
                break;
            }
        };

        let mut size = 0usize;
        for &b in &header[124..135] {
            if (b'0'..=b'7').contains(&b) {
                size = size * 8 + (b - b'0') as usize;
            }
        }

        let out_fd = open(name, O_CREATE | O_WRONLY | O_TRUNC);
        if out_fd < 0 {
            println!("tar: cannot extract {}", name);
            break;
        }

        let mut remaining = size;
        while remaining > 0 {
            let chunk = remaining.min(512);
            let n = read(fd as i32, &mut block[..512]);
            if n <= 0 {
                break;
            }
            let _ = write(out_fd as i32, &block[..chunk]);
            remaining -= chunk;
        }
        close(out_fd as i32);

        // Consume trailing block padding
        let pad = (512 - (size % 512)) % 512;
        if pad > 0 && (size % 512 != 0) {
            // Already read into block
        }

        println!("tar: extracted {} ({} bytes)", name, size);
    }

    close(fd as i32);
    0
}
