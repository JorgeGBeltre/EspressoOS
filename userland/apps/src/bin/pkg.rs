#![no_std]
#![no_main]

use libc::{arg, close, open, println, read, readdir, seek, O_RDONLY};

const FIXUP_MAGIC: u32 = 0x4553_5046; // "ESPF"
const EM_XTENSA: u16 = 94;
static mut DIR_BUF: [u8; 4096] = [0; 4096];

struct VerifyStats {
    size: usize,
    fixups: u32,
    entry: u32,
    ok: bool,
}

fn verify_elf(name: &str) -> VerifyStats {
    let mut pbuf = [0u8; 64];
    let prefix = b"/bin/";
    let nb = name.as_bytes();
    if prefix.len() + nb.len() >= pbuf.len() {
        return VerifyStats { size: 0, fixups: 0, entry: 0, ok: false };
    }
    pbuf[..prefix.len()].copy_from_slice(prefix);
    pbuf[prefix.len()..prefix.len() + nb.len()].copy_from_slice(nb);
    let path = match core::str::from_utf8(&pbuf[..prefix.len() + nb.len()]) {
        Ok(s) => s,
        Err(_) => return VerifyStats { size: 0, fixups: 0, entry: 0, ok: false },
    };

    let fd = open(path, O_RDONLY);
    if fd < 0 {
        return VerifyStats { size: 0, fixups: 0, entry: 0, ok: false };
    }
    let fd = fd as i32;

    let mut hdr = [0u8; 28];
    if read(fd, &mut hdr) < 28 {
        close(fd);
        return VerifyStats { size: 0, fixups: 0, entry: 0, ok: false };
    }

    let elf_ok = hdr[0..4] == [0x7f, b'E', b'L', b'F']
        && hdr[4] == 1 // ELFCLASS32
        && hdr[5] == 1 // ELFDATA2LSB
        && u16::from_le_bytes([hdr[18], hdr[19]]) == EM_XTENSA;
    let entry = u32::from_le_bytes([hdr[24], hdr[25], hdr[26], hdr[27]]);

    let end = seek(fd, 0, 2);
    if !elf_ok || end < 36 {
        close(fd);
        return VerifyStats { size: 0, fixups: 0, entry, ok: false };
    }

    let size = end as usize;
    let _ = seek(fd, -8, 2);
    let mut trailer = [0u8; 8];
    let tn = read(fd, &mut trailer);
    close(fd);
    if tn != 8 {
        return VerifyStats { size, fixups: 0, entry, ok: false };
    }

    let fixups = u32::from_le_bytes([trailer[0], trailer[1], trailer[2], trailer[3]]);
    let magic = u32::from_le_bytes([trailer[4], trailer[5], trailer[6], trailer[7]]);
    let trailer_bytes = (fixups as usize).saturating_mul(4).saturating_add(8);
    let ok = magic == FIXUP_MAGIC && trailer_bytes < size;
    VerifyStats { size, fixups, entry, ok }
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let cmd = if argc > 1 { unsafe { arg(argv, 1) } } else { "list" };

    match cmd {
        "list" => {
            println!("PACKAGE             ARCH                     STATUS");
            println!("------------------------------------------------------------");
            let buf = unsafe { &mut *core::ptr::addr_of_mut!(DIR_BUF) };
            let n = readdir("/bin", buf);
            if n <= 0 {
                println!("pkg: cannot read /bin repository");
                return 1;
            }
            let limit = n as usize;
            let mut pos = 0;
            let mut count = 0usize;
            while pos + 11 <= limit {
                let name_len = u16::from_le_bytes([buf[pos + 9], buf[pos + 10]]) as usize;
                pos += 11;
                if pos + name_len > limit {
                    break;
                }
                if let Ok(name) = core::str::from_utf8(&buf[pos..pos + name_len]) {
                    if name != "." && name != ".." {
                        println!("{:<19} xtensa-esp32s3-none-elf  installed", name);
                        count += 1;
                    }
                }
                pos += name_len;
            }
            println!("------------------------------------------------------------");
            println!("Total packages installed: {}", count);
            0
        }
        "info" => {
            if argc < 3 {
                println!("usage: pkg info <package>");
                return 1;
            }
            let pkg = unsafe { arg(argv, 2) };
            let st = verify_elf(pkg);
            if !st.ok {
                println!("pkg: package '{}' not installed or invalid ELF in /bin", pkg);
                return 1;
            }
            println!("Package:      {}", pkg);
            println!("Architecture: xtensa-esp32s3-none-elf (EM_XTENSA=94)");
            println!("Format:       ELF32-LE + ESPF relocation trailer");
            println!("Image Size:   {} bytes", st.size);
            println!("Entry Point:  0x{:08x}", st.entry);
            println!("Relocations:  {} R_XTENSA_32 fixups", st.fixups);
            println!("Execution:    PSRAM-Exec (16 KiB IBus + 16 KiB DBus slot)");
            0
        }
        "check" | "verify" => {
            let buf = unsafe { &mut *core::ptr::addr_of_mut!(DIR_BUF) };
            let n = readdir("/bin", buf);
            if n <= 0 {
                println!("pkg: cannot read /bin");
                return 1;
            }
            let limit = n as usize;
            let mut pos = 0usize;
            let mut checked = 0usize;
            let mut corrupt = 0usize;
            let mut total_bytes = 0usize;
            let mut total_fixups = 0usize;

            while pos + 11 <= limit {
                let name_len = u16::from_le_bytes([buf[pos + 9], buf[pos + 10]]) as usize;
                pos += 11;
                if pos + name_len > limit {
                    break;
                }
                if let Ok(name) = core::str::from_utf8(&buf[pos..pos + name_len]) {
                    if name != "." && name != ".." {
                        let st = verify_elf(name);
                        checked += 1;
                        if st.ok {
                            total_bytes += st.size;
                            total_fixups += st.fixups as usize;
                        } else {
                            println!("CORRUPT: /bin/{}", name);
                            corrupt += 1;
                        }
                    }
                }
                pos += name_len;
            }

            println!(
                "Verified {} binaries in /bin ({} bytes, {} ESPF fixups): {} corruptions",
                checked, total_bytes, total_fixups, corrupt
            );
            if corrupt == 0 { 0 } else { 1 }
        }
        _ => {
            println!("Usage: pkg [list | info <package> | check]");
            1
        }
    }
}
