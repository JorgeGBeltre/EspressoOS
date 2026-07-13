#![allow(dead_code)]

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::prelude::layout;

#[derive(Clone, Copy, Debug, Default)]
pub struct HeapStats {

    pub total: usize,

    pub used: usize,

    pub free: usize,
}

static TOTAL_CONFIGURED: AtomicUsize = AtomicUsize::new(0);

static INITIALIZED: AtomicBool = AtomicBool::new(false);
static PSRAM_ADDED: AtomicBool = AtomicBool::new(false);

#[repr(C, align(8))]
struct HeapBuf([u8; layout::KERNEL_HEAP_SIZE]);
static mut KERNEL_HEAP: HeapBuf = HeapBuf([0; layout::KERNEL_HEAP_SIZE]);

pub fn init() {

    if INITIALIZED.swap(true, Ordering::AcqRel) {
        return;
    }

    let base = core::ptr::addr_of_mut!(KERNEL_HEAP) as *mut u8;
    let len = layout::KERNEL_HEAP_SIZE;

    unsafe {
        esp_alloc::HEAP.add_region(esp_alloc::HeapRegion::new(
            base,
            len,
            esp_alloc::MemoryCapability::Internal.into(),
        ));
    }

    TOTAL_CONFIGURED.fetch_add(len, Ordering::Relaxed);
}

pub fn add_psram(base: *mut u8, len: usize) {

    if len == 0 {
        return;
    }

    if PSRAM_ADDED.swap(true, Ordering::AcqRel) {
        return;
    }

    unsafe {
        esp_alloc::HEAP.add_region(esp_alloc::HeapRegion::new(
            base,
            len,
            esp_alloc::MemoryCapability::External.into(),
        ));
    }

    TOTAL_CONFIGURED.fetch_add(len, Ordering::Relaxed);
}

pub fn size() -> usize {
    TOTAL_CONFIGURED.load(Ordering::Relaxed)
}

pub fn stats() -> HeapStats {
    HeapStats {
        total: TOTAL_CONFIGURED.load(Ordering::Relaxed),

        used: esp_alloc::HEAP.used(),
        free: esp_alloc::HEAP.free(),
    }
}
