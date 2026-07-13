#![allow(dead_code)]

use alloc::vec;
use core::mem::size_of;
use crate::prelude::*;

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
    // 1. Abrir el archivo
    let fd = crate::vfs::open(path, crate::vfs::OpenFlags::RDONLY)?;
    
    // 2. Leer cabecera ELF
    let mut eh = unsafe { core::mem::zeroed::<ElfHeader>() };
    let eh_slice = unsafe { core::slice::from_raw_parts_mut(&mut eh as *mut _ as *mut u8, size_of::<ElfHeader>()) };
    if crate::vfs::read(fd, eh_slice)? != size_of::<ElfHeader>() {
        let _ = crate::vfs::close(fd);
        return Err(KError::Corrupt);
    }
    
    // Validar firma ELF
    if eh.e_ident[0..4] != [0x7f, b'E', b'L', b'F'] {
        let _ = crate::vfs::close(fd);
        return Err(KError::Corrupt);
    }
    
    // Validar arquitectura Xtensa (94)
    if eh.e_machine != 94 {
        let _ = crate::vfs::close(fd);
        return Err(KError::Corrupt);
    }
    
    // 3. Leer cabeceras de programa
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
    
    // 4. Calcular rango virtual total de los segmentos PT_LOAD
    let mut min_vaddr = u32::MAX;
    let mut max_vaddr = 0;
    
    let phs = unsafe { core::slice::from_raw_parts(ph_bytes.as_ptr() as *const ProgramHeader, eh.e_phnum as usize) };
    for ph in phs {
        if ph.p_type == 1 { // PT_LOAD
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
    
    // 5. Determinar modo de carga (PIC o Estático)
    let is_pic = eh.e_type == 3; // ET_DYN
    let layout = core::alloc::Layout::from_size_align(total_size, 4096).map_err(|_| KError::InvalidArgument)?;
    let load_addr = if is_pic {
        let ptr = unsafe { alloc::alloc::alloc(layout) };
        if ptr.is_null() {
            let _ = crate::vfs::close(fd);
            return Err(KError::NoMem);
        }
        ptr
    } else {
        if min_vaddr < 0x3c000000 || max_vaddr > 0x3c100000 {
            let _ = crate::vfs::close(fd);
            return Err(KError::PermissionDenied);
        }
        min_vaddr as *mut u8
    };
    
    // Limpiar memoria segmentada a cero (para BSS y alineamientos)
    unsafe { core::ptr::write_bytes(load_addr, 0, total_size); }
    
    // Sesgo/Offset de carga: dirección real - dirección preferida
    let load_bias = load_addr as usize as i32 - min_vaddr as i32;
    
    // 6. Cargar cada segmento PT_LOAD en RAM
    for ph in phs {
        if ph.p_type == 1 { // PT_LOAD
            // Seguridad: p_filesz nunca puede exceder p_memsz (si no, la copia se
            // saldría de la región reservada [load_addr, load_addr+total_size)).
            if ph.p_filesz > ph.p_memsz {
                if is_pic {
                    unsafe { alloc::alloc::dealloc(load_addr, layout); }
                }
                let _ = crate::vfs::close(fd);
                return Err(KError::Corrupt);
            }
            let dest_addr = (ph.p_vaddr as i32 + load_bias) as *mut u8;
            if let Err(e) = crate::vfs::seek(fd, crate::vfs::SeekFrom::Start(ph.p_offset as u64)) {
                if is_pic {
                    unsafe { alloc::alloc::dealloc(load_addr, layout); }
                }
                let _ = crate::vfs::close(fd);
                return Err(e);
            }
            let dest_slice = unsafe { core::slice::from_raw_parts_mut(dest_addr, ph.p_filesz as usize) };
            if crate::vfs::read(fd, dest_slice)? != ph.p_filesz as usize {
                if is_pic {
                    unsafe { alloc::alloc::dealloc(load_addr, layout); }
                }
                let _ = crate::vfs::close(fd);
                return Err(KError::Corrupt);
            }
        }
    }
    
    // 7. Resolver relocalizaciones dinámicas (si es un binario PIC y tiene PT_DYNAMIC)
    if eh.e_type == 3 { // ET_DYN (PIC)
        for ph in phs {
            if ph.p_type == 2 { // PT_DYNAMIC
                let dyn_addr = (ph.p_vaddr as i32 + load_bias) as *const DynEntry;
                let dyn_count = ph.p_memsz as usize / size_of::<DynEntry>();
                let dyns = unsafe { core::slice::from_raw_parts(dyn_addr, dyn_count) };
                
                let mut rel_addr: u32 = 0;
                let mut rel_sz: u32 = 0;
                let mut rela_addr: u32 = 0;
                let mut rela_sz: u32 = 0;
                
                for entry in dyns {
                    match entry.d_tag {
                        17 => rel_addr = entry.d_val, // DT_REL
                        18 => rel_sz = entry.d_val,  // DT_RELSZ
                        7 => rela_addr = entry.d_val, // DT_RELA
                        8 => rela_sz = entry.d_val,  // DT_RELASZ
                        0 => break,
                        _ => {}
                    }
                }
                
                // Relocalizaciones REL
                if rel_addr != 0 && rel_sz != 0 {
                    let rels = unsafe { core::slice::from_raw_parts((rel_addr as i32 + load_bias) as *const RelEntry, rel_sz as usize / size_of::<RelEntry>()) };
                    for rel in rels {
                        let r_type = rel.r_info & 0xFF;
                        if r_type == 17 { // R_XTENSA_RELATIVE = 17
                            let ptr = (rel.r_offset as i32 + load_bias) as *mut u32;
                            unsafe {
                                *ptr = (*ptr).wrapping_add(load_bias as u32);
                            }
                        }
                    }
                }
                
                // Relocalizaciones RELA
                if rela_addr != 0 && rela_sz != 0 {
                    let relas = unsafe { core::slice::from_raw_parts((rela_addr as i32 + load_bias) as *const RelaEntry, rela_sz as usize / size_of::<RelaEntry>()) };
                    for rela in relas {
                        let r_type = rela.r_info & 0xFF;
                        if r_type == 17 { // R_XTENSA_RELATIVE
                            let ptr = (rela.r_offset as i32 + load_bias) as *mut u32;
                            unsafe {
                                *ptr = (rela.r_addend as i32 + load_bias) as u32;
                            }
                        }
                    }
                }
            }
        }
    }
    
    let _ = crate::vfs::close(fd);
    
    let entry = (eh.e_entry as i32 + load_bias) as u32;
    Ok((entry, total_size, load_addr))
}
