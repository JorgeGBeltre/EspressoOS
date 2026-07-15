#![allow(dead_code)]

use crate::prelude::*;
use alloc::vec;
use core::mem::size_of;

#[repr(C)]
struct ElfHeader {
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
#[derive(Clone, Copy)]
struct ProgramHeader {
    p_type: u32,
    p_offset: u32,
    p_vaddr: u32,
    p_paddr: u32,
    p_filesz: u32,
    p_memsz: u32,
    p_flags: u32,
    p_align: u32,
}

// DynEntry / RelEntry / RelaEntry lived here to walk PT_DYNAMIC. They went with the
// ET_DYN branch: the kernel does not parse ELF relocations at all any more. The host
// digests them at build time into a flat fixup table, which is the only ELF knowledge
// this loader needs.

pub fn load_elf(path: &str) -> KResult<(u32, usize, *mut u8)> {
    let fd = crate::vfs::open(path, crate::vfs::OpenFlags::RDONLY)?;

    let mut eh = unsafe { core::mem::zeroed::<ElfHeader>() };
    let eh_slice = unsafe {
        core::slice::from_raw_parts_mut(&mut eh as *mut _ as *mut u8, size_of::<ElfHeader>())
    };
    if crate::vfs::read(fd, eh_slice)? != size_of::<ElfHeader>() {
        let _ = crate::vfs::close(fd);
        return Err(KError::Corrupt);
    }

    if eh.e_ident[0..4] != [0x7f, b'E', b'L', b'F'] {
        let _ = crate::vfs::close(fd);
        return Err(KError::Corrupt);
    }

    if eh.e_machine != 94 {
        let _ = crate::vfs::close(fd);
        return Err(KError::Corrupt);
    }

    let ph_size = eh.e_phnum as usize * eh.e_phentsize as usize;
    let mut ph_bytes = vec![0u8; ph_size];
    if let Err(e) = crate::vfs::seek(fd, crate::vfs::SeekFrom::Start(eh.e_phoff as u64)) {
        let _ = crate::vfs::close(fd);
        return Err(e);
    }
    if crate::vfs::read(fd, &mut ph_bytes)? != ph_size {
        let _ = crate::vfs::close(fd);
        return Err(KError::Corrupt);
    }

    let mut min_vaddr = u32::MAX;
    let mut max_vaddr = 0;

    let phs = unsafe {
        core::slice::from_raw_parts(
            ph_bytes.as_ptr() as *const ProgramHeader,
            eh.e_phnum as usize,
        )
    };
    for ph in phs {
        if ph.p_type == 1 {
            if ph.p_vaddr < min_vaddr {
                min_vaddr = ph.p_vaddr;
            }
            let end = ph.p_vaddr.saturating_add(ph.p_memsz);
            if end > max_vaddr {
                max_vaddr = end;
            }
        }
    }

    if min_vaddr == u32::MAX || max_vaddr == 0 {
        let _ = crate::vfs::close(fd);
        return Err(KError::Corrupt);
    }

    let total_size = (max_vaddr - min_vaddr) as usize;

    // No ET_DYN branch here on purpose.
    //
    // There used to be one. It was never reachable and it was wrong twice over:
    //   * It matched relocations with `r_info & 0xFF == 17`. On Xtensa 17 is
    //     R_XTENSA_DIFF8, an assembler-internal relocation; R_XTENSA_RELATIVE is 5.
    //     17 is not RELATIVE on any common arch either (ARM 23, x86_64 8, RISC-V 3),
    //     so the constant was guessed. It would have matched nothing and jumped
    //     into unrelocated code.
    //   * Its DT_RELA arm wrote `addend + bias`, but `ld` leaves 0 in the addend for
    //     section symbols and the resolved value in the word, so that stores the bare
    //     bias.
    // And it could never have run: the LLVM Xtensa backend rejects PIC outright
    //   ("PIC relocations is not supported" -- XtensaISelLowering.cpp), so no ET_DYN
    //   binary can be produced for this target at all. Verified against core itself.
    //
    // Relocatable loading is done instead from a build-time fixup table over an
    // ordinary ET_EXEC (ld --emit-relocs); see kernel/build.rs.

    let dbase = crate::mm::psram_exec::user_data_base();
    if dbase == 0 {
        let _ = crate::vfs::close(fd);
        return Err(KError::NotSupported);
    }
    let dend = dbase + crate::mm::psram_exec::USER_REGION_SIZE;

    for ph in phs {
        if ph.p_type != 1 {
            continue;
        }
        if ph.p_filesz > ph.p_memsz {
            let _ = crate::vfs::close(fd);
            return Err(KError::Corrupt);
        }
        let dest = if crate::mm::psram_exec::is_ibus(ph.p_vaddr, ph.p_memsz) {
            crate::mm::psram_exec::ibus_to_data(ph.p_vaddr)
        } else if ph.p_vaddr >= dbase && ph.p_vaddr.saturating_add(ph.p_memsz) <= dend {
            ph.p_vaddr
        } else {
            let _ = crate::vfs::close(fd);
            return Err(KError::PermissionDenied);
        };
        let dptr = dest as *mut u8;
        unsafe {
            core::ptr::write_bytes(dptr, 0, ph.p_memsz as usize);
        }
        if let Err(e) = crate::vfs::seek(fd, crate::vfs::SeekFrom::Start(ph.p_offset as u64)) {
            let _ = crate::vfs::close(fd);
            return Err(e);
        }
        if ph.p_filesz > 0 {
            let ds = unsafe { core::slice::from_raw_parts_mut(dptr, ph.p_filesz as usize) };
            if crate::vfs::read(fd, ds)? != ph.p_filesz as usize {
                let _ = crate::vfs::close(fd);
                return Err(KError::Corrupt);
            }
        }
    }

    crate::mm::psram_exec::sync_caches();
    let _ = crate::vfs::close(fd);

    Ok((eh.e_entry, total_size, core::ptr::null_mut()))
}
