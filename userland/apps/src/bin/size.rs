#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read, seek, O_RDONLY};

#[repr(C)]
struct Elf32Header {
    e_ident: [u8; 16],
    e_type: u16,
    e_machine: u16,
    e_version: u32,
    e_entry: u32,
    e_phoff: u32,
    e_shoff: u32,
    e_flags: u32,
    e_ehsize: u16,
    e_phentsize: u16,
    e_phnum: u16,
    e_shentsize: u16,
    e_shnum: u16,
    e_shstrndx: u16,
}

#[repr(C)]
struct Elf32Phdr {
    p_type: u32,
    p_offset: u32,
    p_vaddr: u32,
    p_paddr: u32,
    p_filesz: u32,
    p_memsz: u32,
    p_flags: u32,
    p_align: u32,
}

#[repr(C)]
struct Elf32Shdr {
    sh_name: u32,
    sh_type: u32,
    sh_flags: u32,
    sh_addr: u32,
    sh_offset: u32,
    sh_size: u32,
    sh_link: u32,
    sh_info: u32,
    sh_addralign: u32,
    sh_entsize: u32,
}

const PT_LOAD: u32 = 1;
const SHF_WRITE: u32 = 0x1;
const SHF_ALLOC: u32 = 0x2;
const SHT_NOBITS: u32 = 8;

fn measure_file(path: &str) -> Option<(u32, u32, u32)> {
    let fd = open(path, O_RDONLY);
    if fd < 0 {
        print!("size: '");
        print!("{}", path);
        println!("': No such file");
        return None;
    }
    let fd = fd as i32;

    let mut eh = unsafe { core::mem::zeroed::<Elf32Header>() };
    let eh_buf = unsafe {
        core::slice::from_raw_parts_mut(
            &mut eh as *mut _ as *mut u8,
            core::mem::size_of::<Elf32Header>(),
        )
    };

    let n = read(fd, eh_buf);
    if n != core::mem::size_of::<Elf32Header>() as isize {
        close(fd);
        print!("size: ");
        print!("{}", path);
        println!(": file format not recognized");
        return None;
    }

    if eh.e_ident[0..4] != [0x7f, b'E', b'L', b'F'] {
        close(fd);
        print!("size: ");
        print!("{}", path);
        println!(": file format not recognized");
        return None;
    }

    let mut text: u32 = 0;
    let mut data: u32 = 0;
    let mut bss: u32 = 0;

    if eh.e_shoff != 0 && eh.e_shnum > 0 && eh.e_shentsize as usize == core::mem::size_of::<Elf32Shdr>() {
        // Compute from section headers
        for i in 0..eh.e_shnum {
            let offset = (eh.e_shoff as usize) + (i as usize) * core::mem::size_of::<Elf32Shdr>();
            if seek(fd, offset as isize, 0) < 0 {
                break;
            }
            let mut sh = unsafe { core::mem::zeroed::<Elf32Shdr>() };
            let sh_buf = unsafe {
                core::slice::from_raw_parts_mut(
                    &mut sh as *mut _ as *mut u8,
                    core::mem::size_of::<Elf32Shdr>(),
                )
            };
            if read(fd, sh_buf) != core::mem::size_of::<Elf32Shdr>() as isize {
                break;
            }

            if (sh.sh_flags & SHF_ALLOC) != 0 {
                if sh.sh_type == SHT_NOBITS {
                    bss = bss.saturating_add(sh.sh_size);
                } else if (sh.sh_flags & SHF_WRITE) != 0 {
                    data = data.saturating_add(sh.sh_size);
                } else {
                    text = text.saturating_add(sh.sh_size);
                }
            }
        }
    } else if eh.e_phoff != 0 && eh.e_phnum > 0 && eh.e_phentsize as usize == core::mem::size_of::<Elf32Phdr>() {
        // Fallback: compute from program headers
        for i in 0..eh.e_phnum {
            let offset = (eh.e_phoff as usize) + (i as usize) * core::mem::size_of::<Elf32Phdr>();
            if seek(fd, offset as isize, 0) < 0 {
                break;
            }
            let mut ph = unsafe { core::mem::zeroed::<Elf32Phdr>() };
            let ph_buf = unsafe {
                core::slice::from_raw_parts_mut(
                    &mut ph as *mut _ as *mut u8,
                    core::mem::size_of::<Elf32Phdr>(),
                )
            };
            if read(fd, ph_buf) != core::mem::size_of::<Elf32Phdr>() as isize {
                break;
            }

            if ph.p_type == PT_LOAD {
                if (ph.p_flags & 0x2) != 0 {
                    data = data.saturating_add(ph.p_filesz);
                    bss = bss.saturating_add(ph.p_memsz.saturating_sub(ph.p_filesz));
                } else {
                    text = text.saturating_add(ph.p_filesz);
                }
            }
        }
    }

    close(fd);
    Some((text, data, bss))
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut hex_mode = false;
    let mut files_start = 1;

    while files_start < argc {
        let s = unsafe { arg(argv, files_start) };
        if s == "-h" || s == "--help" {
            println!("Usage: size [OPTION]... [FILE]...");
            println!("List section sizes of ELF object/executable files.");
            println!("  -d, --radix=10     Print size numbers in decimal (default)");
            println!("  -x, --radix=16     Print size numbers in hexadecimal");
            println!("  -h, --help         Display this help");
            return 0;
        } else if s == "-x" || s == "--radix=16" {
            hex_mode = true;
            files_start += 1;
        } else if s == "-d" || s == "--radix=10" {
            hex_mode = false;
            files_start += 1;
        } else if s.starts_with("-") {
            print!("size: unrecognized option '");
            print!("{}", s);
            println!("'");
            return 1;
        } else {
            break;
        }
    }

    println!("   text\t   data\t    bss\t    dec\t    hex\tfilename");

    let mut exit_code = 0;
    if files_start >= argc {
        // Default target is a.out if none specified
        if let Some((t, d, b)) = measure_file("a.out") {
            let total = t.saturating_add(d).saturating_add(b);
            if hex_mode {
                println!("{:#x}\t{:#x}\t{:#x}\t{}\t{:x}\ta.out", t, d, b, total, total);
            } else {
                println!("{}\t{}\t{}\t{}\t{:x}\ta.out", t, d, b, total, total);
            }
        } else {
            exit_code = 1;
        }
    } else {
        for idx in files_start..argc {
            let path = unsafe { arg(argv, idx) };
            if let Some((t, d, b)) = measure_file(path) {
                let total = t.saturating_add(d).saturating_add(b);
                if hex_mode {
                    println!("{:#x}\t{:#x}\t{:#x}\t{}\t{:x}\t{}", t, d, b, total, total, path);
                } else {
                    println!("{}\t{}\t{}\t{}\t{:x}\t{}", t, d, b, total, total, path);
                }
            } else {
                exit_code = 1;
            }
        }
    }

    exit_code
}
