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

#[repr(C)]
struct DynEntry {
    d_tag: i32,
    d_val: u32,
}

#[repr(C)]
struct RelEntry {
    r_offset: u32,
    r_info: u32,
}

#[repr(C)]
struct RelaEntry {
    r_offset: u32,
    r_info: u32,
    r_addend: i32,
}

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

    if eh.e_type == 3 {
        let layout = core::alloc::Layout::from_size_align(total_size, 4096)
            .map_err(|_| KError::InvalidArgument)?;
        let load_addr = unsafe { alloc::alloc::alloc(layout) };
        if load_addr.is_null() {
            let _ = crate::vfs::close(fd);
            return Err(KError::NoMem);
        }
        unsafe {
            core::ptr::write_bytes(load_addr, 0, total_size);
        }
        let load_bias = load_addr as usize as i32 - min_vaddr as i32;

        for ph in phs {
            if ph.p_type == 1 {
                if ph.p_filesz > ph.p_memsz {
                    unsafe {
                        alloc::alloc::dealloc(load_addr, layout);
                    }
                    let _ = crate::vfs::close(fd);
                    return Err(KError::Corrupt);
                }
                let dest_addr = (ph.p_vaddr as i32 + load_bias) as *mut u8;
                if let Err(e) =
                    crate::vfs::seek(fd, crate::vfs::SeekFrom::Start(ph.p_offset as u64))
                {
                    unsafe {
                        alloc::alloc::dealloc(load_addr, layout);
                    }
                    let _ = crate::vfs::close(fd);
                    return Err(e);
                }
                let dest_slice =
                    unsafe { core::slice::from_raw_parts_mut(dest_addr, ph.p_filesz as usize) };
                if crate::vfs::read(fd, dest_slice)? != ph.p_filesz as usize {
                    unsafe {
                        alloc::alloc::dealloc(load_addr, layout);
                    }
                    let _ = crate::vfs::close(fd);
                    return Err(KError::Corrupt);
                }
            }
        }

        for ph in phs {
            if ph.p_type == 2 {
                let dyn_addr = (ph.p_vaddr as i32 + load_bias) as *const DynEntry;
                let dyn_count = ph.p_memsz as usize / size_of::<DynEntry>();
                let dyns = unsafe { core::slice::from_raw_parts(dyn_addr, dyn_count) };
                let (mut rel_addr, mut rel_sz, mut rela_addr, mut rela_sz) =
                    (0u32, 0u32, 0u32, 0u32);
                for entry in dyns {
                    match entry.d_tag {
                        17 => rel_addr = entry.d_val,
                        18 => rel_sz = entry.d_val,
                        7 => rela_addr = entry.d_val,
                        8 => rela_sz = entry.d_val,
                        0 => break,
                        _ => {}
                    }
                }
                if rel_addr != 0 && rel_sz != 0 {
                    let rels = unsafe {
                        core::slice::from_raw_parts(
                            (rel_addr as i32 + load_bias) as *const RelEntry,
                            rel_sz as usize / size_of::<RelEntry>(),
                        )
                    };
                    for rel in rels {
                        if rel.r_info & 0xFF == 17 {
                            let ptr = (rel.r_offset as i32 + load_bias) as *mut u32;
                            unsafe {
                                *ptr = (*ptr).wrapping_add(load_bias as u32);
                            }
                        }
                    }
                }
                if rela_addr != 0 && rela_sz != 0 {
                    let relas = unsafe {
                        core::slice::from_raw_parts(
                            (rela_addr as i32 + load_bias) as *const RelaEntry,
                            rela_sz as usize / size_of::<RelaEntry>(),
                        )
                    };
                    for rela in relas {
                        if rela.r_info & 0xFF == 17 {
                            let ptr = (rela.r_offset as i32 + load_bias) as *mut u32;
                            unsafe {
                                *ptr = (rela.r_addend as i32 + load_bias) as u32;
                            }
                        }
                    }
                }
            }
        }

        let _ = crate::vfs::close(fd);
        let entry = (eh.e_entry as i32 + load_bias) as u32;
        return Ok((entry, total_size, load_addr));
    }

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
