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
