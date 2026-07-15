#![allow(dead_code)]

const MMU_ACCESS_SPIRAM: u32 = 1 << 15;

pub const MMU_PAGE_SIZE: u32 = 0x1_0000;

pub const USER_IBUS_BASE: u32 = 0x4280_0000;

pub const USER_REGION_SIZE: u32 = 0x10_0000;

use core::sync::atomic::{AtomicU32, Ordering};

static DATA_BASE: AtomicU32 = AtomicU32::new(0);

pub fn set_data_base(base: u32) {
    DATA_BASE.store(base, Ordering::Relaxed);
}

pub fn user_data_base() -> u32 {
    DATA_BASE.load(Ordering::Relaxed)
}

pub fn ibus_to_data(instr_addr: u32) -> u32 {
    user_data_base() + (instr_addr - USER_IBUS_BASE)
}

pub fn is_ibus(addr: u32, size: u32) -> bool {
    addr >= USER_IBUS_BASE && addr.saturating_add(size) <= USER_IBUS_BASE + USER_REGION_SIZE
}

/// Which bus a link-time address belongs to. Only the bus matters, not whether the
/// address is in range: a binary is linked for slot 0 and the loader moves it, so
/// its addresses are not expected to land anywhere in particular.
pub fn is_ibus_range(addr: u32) -> bool {
    addr >= IBUS_MIN
}

/// Everything at or above this is instruction bus on the ESP32-S3; PSRAM data is
/// far below at 0x3c......
const IBUS_MIN: u32 = 0x4200_0000;

extern "C" {

    fn Cache_Ibus_MMU_Set(
        ext_ram: u32,
        vaddr: u32,
        paddr: u32,
        psize: u32,
        num: u32,
        fixed: u32,
    ) -> i32;

    fn Cache_WriteBack_All();
    fn Cache_Invalidate_ICache_All();
}

pub fn map_instruction(phys_page0: u32, num_pages: u32) -> Result<(), i32> {
    let r = unsafe {
        Cache_Ibus_MMU_Set(
            MMU_ACCESS_SPIRAM,
            USER_IBUS_BASE,
            phys_page0 * MMU_PAGE_SIZE,
            64,
            num_pages,
            0,
        )
    };
    if r != 0 {
        Err(r)
    } else {
        Ok(())
    }
}

#[inline]
pub fn ibus_addr(offset: u32) -> u32 {
    USER_IBUS_BASE + offset
}

// ---------------------------------------------------------------------------
// Slot pool
//
// The reserved megabyte is split in half: the low 512 KB is the text image,
// reachable for execution through the instruction-bus alias at USER_IBUS_BASE,
// and the high 512 KB is the data image, addressed directly. A slot is one index
// into both halves at once.
//
// 16 KB per slot: the largest binary today is sh at 6061 B of text (2.7x
// headroom), and the biggest p_align any of the ten segments declares is 4096,
// which 16384 is a multiple of. The bias a slot implies is i * 16384, so it is
// always a multiple of 4 -- which L32R needs, because it addresses its literal
// pool as (label - ((PC + 3) & !3)) >> 2 and a bias that is not word-aligned
// would shift that rounding and break every literal load.
//
// A binary can run in ANY slot, which is the whole point: two instances of the
// same program can coexist. That works because the loader relocates with the
// fixup table built by kernel/build.rs -- there is no PIE on this target, the
// LLVM Xtensa backend refuses to emit it.
// ---------------------------------------------------------------------------

pub const SLOT_SIZE: u32 = 16 * 1024;

pub const SLOT_COUNT: u32 = 32;

/// Half the reserved region; the other half is the data image.
const TEXT_REGION_SIZE: u32 = SLOT_SIZE * SLOT_COUNT;

const _: () = assert!(TEXT_REGION_SIZE * 2 == USER_REGION_SIZE);
const _: () = assert!(SLOT_COUNT <= 32, "the free bitmap is one u32");
const _: () = assert!(SLOT_SIZE % 4 == 0, "slot bias must keep L32R literals valid");

// The linker script kernel/build.rs writes has to describe the same geometry this
// pool hands out. If it does not, a binary that outgrows a slot links fine and
// overflows into the neighbouring one on the board.
const _: () = assert!(
    SLOT_SIZE == crate::userland_bin::USERLAND_SLOT_SIZE,
    "userland linker script slot size disagrees with the pool"
);
const _: () = assert!(
    USER_IBUS_BASE == crate::userland_bin::USERLAND_LINK_TEXT,
    "userland links its text somewhere other than slot 0"
);

/// A claim on one slot. Only `slot_alloc` makes one, so an index can never name a
/// slot that was not reserved.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SlotIndex(u32);

impl SlotIndex {
    pub fn get(self) -> u32 {
        self.0
    }
}

const SLOT_MASK: u32 = if SLOT_COUNT == 32 {
    u32::MAX
} else {
    (1u32 << SLOT_COUNT) - 1
};

static SLOTS_USED: AtomicU32 = AtomicU32::new(0);

/// Claims a free slot, or None when all are in use.
///
/// A CAS loop rather than a Mutex on purpose: this kernel's Mutex disables
/// interrupts for the whole guard, and there is no reason to pay that to flip a
/// bit.
pub fn slot_alloc() -> Option<SlotIndex> {
    loop {
        let cur = SLOTS_USED.load(Ordering::Acquire);
        let free = !cur & SLOT_MASK;
        if free == 0 {
            return None;
        }
        let i = free.trailing_zeros();
        let next = cur | (1u32 << i);
        if SLOTS_USED
            .compare_exchange_weak(cur, next, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            return Some(SlotIndex(i));
        }
    }
}

/// Releases a slot. Idempotent, so a double free wastes nothing rather than
/// handing the same slot to two processes.
pub fn slot_free(idx: SlotIndex) {
    SLOTS_USED.fetch_and(!(1u32 << idx.0), Ordering::AcqRel);
}

pub fn slots_in_use() -> u32 {
    SLOTS_USED.load(Ordering::Relaxed).count_ones()
}

/// Where this slot's code EXECUTES: an instruction-bus address.
///
/// This is the number the loader biases against (`text_bias = this - the address
/// the binary was linked for`), and the value to jump to. It is NOT writable --
/// the mapping is for instruction fetch. Use `slot_text_write` to put bytes there.
pub fn slot_text_exec(idx: SlotIndex) -> u32 {
    USER_IBUS_BASE + idx.0 * SLOT_SIZE
}

/// Where this slot's code is WRITTEN: the data alias of the very same bytes.
///
/// Split from `slot_text_exec` deliberately. They are two addresses for one
/// piece of PSRAM, and writing through the wrong one is silent: the store lands
/// somewhere harmless-looking and the CPU keeps fetching the old instructions.
/// After writing here, `sync_caches()` before executing -- the icache still holds
/// what was there before.
pub fn slot_text_write(idx: SlotIndex) -> *mut u8 {
    ibus_to_data(slot_text_exec(idx)) as *mut u8
}

/// Where this slot's data lives. One address for both writing and access; the
/// data image needs no alias.
pub fn slot_data(idx: SlotIndex) -> *mut u8 {
    (user_data_base() + TEXT_REGION_SIZE + idx.0 * SLOT_SIZE) as *mut u8
}

pub fn sync_caches() {
    unsafe {
        Cache_WriteBack_All();
        Cache_Invalidate_ICache_All();
    }
}

core::arch::global_asm!(
    ".section .rwtext,\"ax\",@progbits",
    ".align 4",
    ".global __psram_probe_tpl",
    ".global __psram_probe_end",
    "__psram_probe_tpl:",
    "movi a2, 42",
    "ret",
    "__psram_probe_end:",
);

extern "C" {
    fn __psram_probe_tpl();
    fn __psram_probe_end();
}

pub fn selftest(data_base: u32) -> u32 {
    let src = __psram_probe_tpl as *const () as usize;
    let end = __psram_probe_end as *const () as usize;
    let len = end.saturating_sub(src).max(8);

    unsafe {
        core::ptr::copy_nonoverlapping(src as *const u8, data_base as *mut u8, len);
    }
    sync_caches();

    let ret: u32;
    unsafe {
        core::arch::asm!(
            "mov {t}, a0",
            "callx0 {addr}",
            "mov a0, {t}",
            addr = in(reg) USER_IBUS_BASE,
            t = out(reg) _,
            lateout("a2") ret,
        );
    }
    ret
}
