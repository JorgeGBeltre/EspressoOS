#![allow(dead_code)]

use crate::arch::xtensa::sync::Mutex;
use crate::prelude::*;

use embedded_storage::nor_flash::{NorFlash, ReadNorFlash};
use esp_storage::FlashStorage;

pub const SECTOR_SIZE: usize = 4096;

const WRITE_ALIGN: u32 = 4;

static FLASH: Mutex<Option<FlashStorage>> = Mutex::new(None);

fn with_flash<R>(f: impl FnOnce(&mut FlashStorage) -> R) -> R {
    let mut guard = FLASH.lock();
    let fs = guard.get_or_insert_with(FlashStorage::new);
    f(fs)
}

fn check_range(offset: u32, len: usize) -> KResult<()> {
    let end = (offset as u64)
        .checked_add(len as u64)
        .ok_or(KError::InvalidArgument)?;
    if end > layout::FLASH_SIZE as u64 {
        return Err(KError::InvalidArgument);
    }
    Ok(())
}

const fn sector_base(offset: u32) -> u32 {
    offset & !((SECTOR_SIZE as u32) - 1)
}

pub fn read(offset: u32, buf: &mut [u8]) -> KResult<()> {
    check_range(offset, buf.len())?;

    with_flash(|fs| fs.read(offset, buf).map_err(|_| KError::IoError))
}

pub fn write(offset: u32, buf: &[u8]) -> KResult<()> {
    check_range(offset, buf.len())?;

    if offset % WRITE_ALIGN != 0 || (buf.len() as u32) % WRITE_ALIGN != 0 {
        return Err(KError::InvalidArgument);
    }
    if buf.is_empty() {
        return Ok(());
    }

    with_flash(|fs| fs.write(offset, buf).map_err(|_| KError::IoError))
}

pub fn erase_sector(offset: u32) -> KResult<()> {
    let base = sector_base(offset);

    check_range(base, SECTOR_SIZE)?;
    let end = base + SECTOR_SIZE as u32;

    with_flash(|fs| fs.erase(base, end).map_err(|_| KError::IoError))
}
