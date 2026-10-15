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

fn print_dec(mut val: u32) {
    if val == 0 {
        print!("0");
        return;
    }
    let mut buf = [0u8; 12];
    let mut idx = 0;
    while val > 0 {
        buf[idx] = b'0' + (val % 10) as u8;
        val /= 10;
        idx += 1;
    }
    for i in (0..idx).rev() {
        let ch = [buf[i], 0];
        print!("{}", unsafe { core::str::from_utf8_unchecked(&ch[..1]) });
    }
}

fn print_hex8(val: u32) {
    let hex_chars = b"0123456789abcdef";
    let mut buf = [0u8; 8];
    for i in (0..8).rev() {
        buf[i] = hex_chars[((val >> ((7 - i) * 4)) & 0xf) as usize];
    }
    let s = unsafe { core::str::from_utf8_unchecked(&buf) };
    print!("{}", s);
}

fn print_hex6(val: u32) {
    let hex_chars = b"0123456789abcdef";
    let mut buf = [0u8; 6];
    for i in (0..6).rev() {
        buf[i] = hex_chars[((val >> ((5 - i) * 4)) & 0xf) as usize];
    }
    let s = unsafe { core::str::from_utf8_unchecked(&buf) };
    print!("{}", s);
}

fn print_header(hdr: &Elf32Header) {
    println!("ELF Header:");
    print!("  Magic:   ");
    let hex_chars = b"0123456789abcdef";
    for i in 0..16 {
        let b = hdr.e_ident[i];
        let h = [hex_chars[(b >> 4) as usize], hex_chars[(b & 0xf) as usize], b' '];
        print!("{}", unsafe { core::str::from_utf8_unchecked(&h) });
    }
    println!();

    println!("  Class:                             ELF32");
    println!("  Data:                              2's complement, little endian");
    println!("  Version:                           1 (current)");
    println!("  OS/ABI:                            UNIX - System V");

    print!("  Type:                              ");
    match hdr.e_type {
        1 => println!("REL (Relocatable file)"),
        2 => println!("EXEC (Executable file)"),
        3 => println!("DYN (Shared object file)"),
        _ => println!("Unknown"),
    }

    print!("  Machine:                           ");
    match hdr.e_machine {
        0x005e => println!("Tensilica Xtensa"),
        0x00f3 => println!("UCB RISC-V"),
        0x0028 => println!("ARM"),
        0x0003 => println!("Intel 80386"),
        _ => {
            print!("0x");
            print_hex8(hdr.e_machine as u32);
            println!();
        }
    }

    print!("  Entry point address:               0x");
    print_hex8(hdr.e_entry);
    println!();

    print!("  Start of program headers:          ");
    print_dec(hdr.e_phoff);
    println!(" (bytes into file)");

    print!("  Start of section headers:          ");
    print_dec(hdr.e_shoff);
    println!(" (bytes into file)");

    print!("  Number of program headers:         ");
    print_dec(hdr.e_phnum as u32);
    println!();

    print!("  Number of section headers:         ");
    print_dec(hdr.e_shnum as u32);
    println!();
}

fn print_program_headers(fd: i32, hdr: &Elf32Header) {
    if hdr.e_phnum == 0 || hdr.e_phoff == 0 {
        println!("There are no program headers in this file.");
        return;
    }

    println!("Program Headers:");
    println!("  Type           Offset   VirtAddr   PhysAddr   FileSiz MemSiz  Flg Align");

    let mut phdr = Elf32Phdr {
        p_type: 0,
        p_offset: 0,
        p_vaddr: 0,
        p_paddr: 0,
        p_filesz: 0,
        p_memsz: 0,
        p_flags: 0,
        p_align: 0,
    };

    let p_slice = unsafe {
        core::slice::from_raw_parts_mut(
            &mut phdr as *mut _ as *mut u8,
            core::mem::size_of::<Elf32Phdr>(),
        )
    };

    for i in 0..hdr.e_phnum {
        let off = hdr.e_phoff as isize + (i as isize * hdr.e_phentsize as isize);
        let _ = seek(fd, off, 0);
        if read(fd, p_slice) != core::mem::size_of::<Elf32Phdr>() as isize {
            break;
        }

        print!("  ");
        match phdr.p_type {
            1 => print!("LOAD           "),
            2 => print!("DYNAMIC        "),
            6 => print!("PHDR           "),
            _ => print!("OTHER          "),
        }

        print!("0x");
        print_hex6(phdr.p_offset);
        print!(" 0x");
        print_hex8(phdr.p_vaddr);
        print!(" 0x");
        print_hex8(phdr.p_paddr);
        print!(" 0x");
        print_hex6(phdr.p_filesz);
        print!(" 0x");
        print_hex6(phdr.p_memsz);
        print!(" ");

        let r = if phdr.p_flags & 4 != 0 { "R" } else { " " };
        let w = if phdr.p_flags & 2 != 0 { "W" } else { " " };
        let x = if phdr.p_flags & 1 != 0 { "E" } else { " " };
        print!("{}", r);
        print!("{}", w);
        print!("{}", x);
        print!("  0x");
        print_hex6(phdr.p_align);
        println!();
    }
}

fn print_section_headers(fd: i32, hdr: &Elf32Header) {
    if hdr.e_shnum == 0 || hdr.e_shoff == 0 {
        println!("There are no sections in this file.");
        return;
    }

    println!("Section Headers:");
    println!("  [Nr] Name              Type            Addr     Off    Size   Flg Al");

    let mut shdr = Elf32Shdr {
        sh_name: 0,
        sh_type: 0,
        sh_flags: 0,
        sh_addr: 0,
        sh_offset: 0,
        sh_size: 0,
        sh_link: 0,
        sh_info: 0,
        sh_addralign: 0,
        sh_entsize: 0,
    };

    let s_slice = unsafe {
        core::slice::from_raw_parts_mut(
            &mut shdr as *mut _ as *mut u8,
            core::mem::size_of::<Elf32Shdr>(),
        )
    };

    for i in 0..hdr.e_shnum {
        let off = hdr.e_shoff as isize + (i as isize * hdr.e_shentsize as isize);
        let _ = seek(fd, off, 0);
        if read(fd, s_slice) != core::mem::size_of::<Elf32Shdr>() as isize {
            break;
        }

        print!("  [");
        if i < 10 {
            print!(" ");
        }
        print_dec(i as u32);
        print!("] ");

        match shdr.sh_type {
            0 => print!("NULL            "),
            1 => print!("PROGBITS        "),
            2 => print!("SYMTAB          "),
            3 => print!("STRTAB          "),
            8 => print!("NOBITS          "),
            9 => print!("REL             "),
            _ => print!("OTHER           "),
        }

        print_hex8(shdr.sh_addr);
        print!(" ");
        print_hex6(shdr.sh_offset);
        print!(" ");
        print_hex6(shdr.sh_size);
        print!(" ");

        let a = if shdr.sh_flags & 2 != 0 { "A" } else { " " };
        let w = if shdr.sh_flags & 1 != 0 { "W" } else { " " };
        let x = if shdr.sh_flags & 4 != 0 { "X" } else { " " };
        print!("{}", w);
        print!("{}", a);
        print!("{}", x);
        print!(" ");
        print_dec(shdr.sh_addralign);
        println!();
    }
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut show_header = false;
    let mut show_prog_headers = false;
    let mut show_section_headers = false;
    let mut files = [core::ptr::null::<u8>(); 16];
    let mut num_files = 0;

    for i in 1..argc {
        let arg_str = unsafe { arg(argv, i) };
        if arg_str == "-h" || arg_str == "--file-header" {
            show_header = true;
        } else if arg_str == "-l" || arg_str == "--program-headers" {
            show_prog_headers = true;
        } else if arg_str == "-S" || arg_str == "--section-headers" {
            show_section_headers = true;
        } else if arg_str == "-a" || arg_str == "--all" {
            show_header = true;
            show_prog_headers = true;
            show_section_headers = true;
        } else if arg_str == "--help" {
            println!("Usage: readelf <option(s)> elf-file(s)");
            println!("Display information about the contents of ELF format files");
            println!("  -h, --file-header      Display the ELF file header");
            println!("  -l, --program-headers  Display the program headers");
            println!("  -S, --section-headers  Display the sections' header");
            println!("  -a, --all              Equivalent to: -h -l -S");
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

    if !show_header && !show_prog_headers && !show_section_headers {
        show_header = true;
    }

    if num_files == 0 {
        println!("readelf: Error: Nothing to do.");
        println!("Usage: readelf <option(s)> elf-file(s)");
        return 1;
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
            print!("readelf: Error: '");
            print!("{}", path_str);
            println!("': No such file");
            exit_code = 1;
            continue;
        }

        let mut hdr = Elf32Header {
            e_ident: [0; 16],
            e_type: 0,
            e_machine: 0,
            e_version: 0,
            e_entry: 0,
            e_phoff: 0,
            e_shoff: 0,
            e_flags: 0,
            e_ehsize: 0,
            e_phentsize: 0,
            e_phnum: 0,
            e_shentsize: 0,
            e_shnum: 0,
            e_shstrndx: 0,
        };

        let hdr_slice = unsafe {
            core::slice::from_raw_parts_mut(
                &mut hdr as *mut _ as *mut u8,
                core::mem::size_of::<Elf32Header>(),
            )
        };

        if read(fd, hdr_slice) != core::mem::size_of::<Elf32Header>() as isize
            || &hdr.e_ident[0..4] != b"\x7fELF"
        {
            print!("readelf: Error: ");
            print!("{}", path_str);
            println!(": Not an ELF file - it has the wrong magic bytes at the start");
            close(fd);
            exit_code = 1;
            continue;
        }

        if show_header {
            print_header(&hdr);
            println!();
        }

        if show_prog_headers {
            print_program_headers(fd, &hdr);
            println!();
        }

        if show_section_headers {
            print_section_headers(fd, &hdr);
            println!();
        }

        close(fd);
    }

    exit_code
}
