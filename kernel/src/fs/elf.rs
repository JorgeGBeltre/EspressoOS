#![allow(dead_code)]

use crate::mm::psram_exec::{self, SlotIndex, SLOT_SIZE};
use crate::prelude::*;
use crate::vfs::{self, OpenFlags, SeekFrom};
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

const PT_LOAD: u32 = 1;
const EM_XTENSA: u16 = 94;

/// `<elf><fixups u32[]><count u32><magic u32>`, appended by kernel/build.rs. Found
/// by seeking from the end, so no section header is ever parsed.
const FIXUP_MAGIC: u32 = 0x4553_5046; // "ESPF"
const TRAILER_LEN: u64 = 8;

/// A program resident in a slot. `slot` travels with it because whoever reaps the
/// process has to hand the slot back -- nothing else knows which one it took.
pub struct LoadedElf {
    /// Instruction-bus address to jump to. Already biased.
    pub entry: u32,
    pub slot: SlotIndex,
    /// Instruction-bus base of the slot: `entry`'s frame of reference.
    pub text_base: u32,
    pub data_base: *mut u8,
}

fn rd32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

/// Loads a program into a free slot and relocates it there.
///
/// There is no PIE on this target -- the LLVM Xtensa backend refuses to emit it --
/// so the binary is an ordinary ET_EXEC linked at slot 0's addresses, and moving it
/// elsewhere means patching the words listed in its fixup trailer. Xtensa makes
/// that cheap: no instruction can hold a 32-bit absolute, so every far reference
/// goes through the literal pool, and literals are data. Tens of words per binary,
/// no instruction decoding. See kernel/build.rs for how the table is made.
pub fn load_elf(path: &str) -> KResult<LoadedElf> {
    let fd = vfs::open(path, OpenFlags::RDONLY)?;
    let r = load_inner(fd);
    let _ = vfs::close(fd);
    r
}

fn load_inner(fd: vfs::Fd) -> KResult<LoadedElf> {
    let mut eh = unsafe { core::mem::zeroed::<ElfHeader>() };
    let eh_slice = unsafe {
        core::slice::from_raw_parts_mut(&mut eh as *mut _ as *mut u8, size_of::<ElfHeader>())
    };
    if vfs::read(fd, eh_slice)? != size_of::<ElfHeader>() {
        return Err(KError::Corrupt);
    }
    if eh.e_ident[0..4] != [0x7f, b'E', b'L', b'F'] || eh.e_machine != EM_XTENSA {
        return Err(KError::Corrupt);
    }

    let phs = read_phdrs(fd, &eh)?;
    let (text, data) = measure(&phs)?;

    if text.size > SLOT_SIZE || data.size > SLOT_SIZE {
        return Err(KError::NoSpace);
    }

    let fixups = read_fixups(fd)?;

    let slot = psram_exec::slot_alloc().ok_or(KError::Busy)?;
    match place(fd, &eh, &phs, &text, &data, slot, &fixups) {
        Ok(loaded) => Ok(loaded),
        Err(e) => {
            psram_exec::slot_free(slot);
            Err(e)
        }
    }
}

#[derive(Clone, Copy)]
struct Region {
    base: u32,
    size: u32,
}

fn read_phdrs(fd: vfs::Fd, eh: &ElfHeader) -> KResult<Vec<ProgramHeader>> {
    let n = eh.e_phnum as usize;
    if n == 0 || eh.e_phentsize as usize != size_of::<ProgramHeader>() {
        return Err(KError::Corrupt);
    }
    let bytes = n * size_of::<ProgramHeader>();
    vfs::seek(fd, SeekFrom::Start(eh.e_phoff as u64))?;
    let mut raw = vec![0u8; bytes];
    if vfs::read(fd, &mut raw)? != bytes {
        return Err(KError::Corrupt);
    }
    Ok(unsafe { core::slice::from_raw_parts(raw.as_ptr() as *const ProgramHeader, n) }.to_vec())
}

/// Where the binary was linked, split by bus. The loader derives this rather than
/// trusting a constant, so it stays correct if build.rs ever moves the canonical
/// address.
fn measure(phs: &[ProgramHeader]) -> KResult<(Region, Region)> {
    let (mut t, mut d) = (None::<Region>, None::<Region>);
    for ph in phs.iter().filter(|p| p.p_type == PT_LOAD && p.p_memsz > 0) {
        let end = ph.p_vaddr.checked_add(ph.p_memsz).ok_or(KError::Corrupt)?;
        let slot = if psram_exec::is_ibus_range(ph.p_vaddr) {
            &mut t
        } else {
            &mut d
        };
        *slot = Some(match *slot {
            None => Region {
                base: ph.p_vaddr,
                size: ph.p_memsz,
            },
            Some(r) => {
                let base = r.base.min(ph.p_vaddr);
                Region {
                    base,
                    size: (r.base + r.size).max(end) - base,
                }
            }
        });
    }
    let text = t.ok_or(KError::Corrupt)?;
    // A program with no data at all is legal; give it an empty region anchored
    // wherever build.rs put UDATA so the bias math stays uniform.
    let data = d.unwrap_or(Region {
        base: crate::userland_bin::USERLAND_LINK_DATA,
        size: 0,
    });
    Ok((text, data))
}

fn read_fixups(fd: vfs::Fd) -> KResult<Vec<u32>> {
    let size = vfs::seek(fd, SeekFrom::End(0))?;
    if size < TRAILER_LEN {
        return Err(KError::Corrupt);
    }
    vfs::seek(fd, SeekFrom::Start(size - TRAILER_LEN))?;
    let mut t = [0u8; TRAILER_LEN as usize];
    if vfs::read(fd, &mut t)? != t.len() {
        return Err(KError::Corrupt);
    }
    if rd32(&t, 4) != FIXUP_MAGIC {
        return Err(KError::Corrupt);
    }
    let count = rd32(&t, 0) as u64;
    let bytes = count
        .checked_mul(4)
        .and_then(|b| b.checked_add(TRAILER_LEN))
        .ok_or(KError::Corrupt)?;
    if bytes > size {
        return Err(KError::Corrupt);
    }

    vfs::seek(fd, SeekFrom::Start(size - bytes))?;
    let mut raw = vec![0u8; (count * 4) as usize];
    if vfs::read(fd, &mut raw)? != raw.len() {
        return Err(KError::Corrupt);
    }
    Ok((0..count as usize).map(|i| rd32(&raw, i * 4)).collect())
}

fn place(
    fd: vfs::Fd,
    eh: &ElfHeader,
    phs: &[ProgramHeader],
    text: &Region,
    data: &Region,
    slot: SlotIndex,
    fixups: &[u32],
) -> KResult<LoadedElf> {
    let text_exec = psram_exec::slot_text_exec(slot);
    // Every store below goes through the data alias. Writing to text_exec would not
    // fault -- the store would land somewhere and the CPU would keep fetching the
    // old bytes.
    let text_write = psram_exec::slot_text_write(slot) as u32;
    let data_write = psram_exec::slot_data(slot) as u32;

    let text_bias = text_exec.wrapping_sub(text.base);
    let data_bias = data_write.wrapping_sub(data.base);

    for ph in phs.iter().filter(|p| p.p_type == PT_LOAD && p.p_memsz > 0) {
        let (region, dest_base) = if psram_exec::is_ibus_range(ph.p_vaddr) {
            (text, text_write)
        } else {
            (data, data_write)
        };
        let off = ph.p_vaddr - region.base;
        let dest = (dest_base + off) as *mut u8;

        unsafe { core::ptr::write_bytes(dest, 0, ph.p_memsz as usize) };
        if ph.p_filesz > 0 {
            vfs::seek(fd, SeekFrom::Start(ph.p_offset as u64))?;
            let buf = unsafe { core::slice::from_raw_parts_mut(dest, ph.p_filesz as usize) };
            if vfs::read(fd, buf)? != ph.p_filesz as usize {
                return Err(KError::Corrupt);
            }
        }
    }

    apply_fixups(fixups, text, data, text_write, data_write, text_bias, data_bias)?;

    // Everything above wrote code through the DATA alias, so those bytes sit in the
    // data cache while the instruction cache still holds whatever this slot used to
    // contain. Write back first, then invalidate: the reverse order would drop the
    // stale lines and immediately refetch them from PSRAM before the new bytes had
    // landed there.
    //
    // Cache_WriteBack_All / Cache_Invalidate_ICache_All take no address, which is
    // the whole reason there is nothing to get wrong here -- no range and no alias
    // to pass, and therefore no way to pass the wrong one. It costs a full icache
    // invalidate per exec, which is the right trade for a path that runs once per
    // program launch.
    psram_exec::sync_caches();

    Ok(LoadedElf {
        entry: eh.e_entry.wrapping_add(text_bias),
        slot,
        text_base: text_exec,
        data_base: data_write as *mut u8,
    })
}

/// Adds the slot's bias to every word the host said holds an address.
///
/// Read-modify-write, never `S + A`: `ld --emit-relocs` leaves the addend at 0 for
/// section symbols and keeps the resolved value in the word (240 of the 273
/// relocations across the ten binaries have `S + A != word`). The word is right by
/// construction -- it is what runs today at the link address -- so all it needs is
/// the bias.
fn apply_fixups(
    fixups: &[u32],
    text: &Region,
    data: &Region,
    text_write: u32,
    data_write: u32,
    text_bias: u32,
    data_bias: u32,
) -> KResult<()> {
    for &f in fixups {
        let off = f & !3;
        let word_in_data = f & 2 != 0;
        let bias = if f & 1 != 0 { data_bias } else { text_bias };

        let (region, base) = if word_in_data {
            (data, data_write)
        } else {
            (text, text_write)
        };

        // The trailer comes from our own build, but it arrives via the filesystem
        // and nothing has authenticated it. A corrupt offset would otherwise patch
        // whatever sits past the slot -- the next slot's code, or the heap.
        if off.checked_add(4).ok_or(KError::Fault)? > region.size.min(SLOT_SIZE) {
            return Err(KError::Fault);
        }

        let p = (base + off) as *mut u32;
        unsafe { *p = (*p).wrapping_add(bias) };
    }
    Ok(())
}
