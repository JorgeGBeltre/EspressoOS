#![allow(dead_code)]

use crate::drivers::flash;
use crate::prelude::*;

pub mod partition;

pub use partition::{OtaImgState, OtaSelectEntry, Slot};

pub const ESP_IMAGE_MAGIC: u8 = 0xE9;

pub fn active_slot() -> Slot {
    partition::active_slot()
}

/// Estado de otadata (dos copias) para inspección desde la shell.
pub fn otadata_entries() -> KResult<[OtaSelectEntry; 2]> {
    partition::otadata_entries()
}

/// Valida la cabecera de una imagen de aplicación ESP (magic 0xE9).
pub fn validate_header(image: &[u8]) -> KResult<()> {
    match image.first() {
        Some(&b) if b == ESP_IMAGE_MAGIC => Ok(()),
        _ => Err(KError::Corrupt),
    }
}

pub fn inactive_slot() -> Slot {
    partition::active_slot().other()
}

pub fn set_boot_slot(slot: Slot) -> KResult<()> {
    partition::set_boot_slot(slot)
}

pub struct OtaUpdate {

    slot: Slot,

    base: u32,

    capacity: u32,

    written: u32,

    next_erase: u32,

    header_ok: bool,
}

impl OtaUpdate {

    pub fn begin() -> KResult<OtaUpdate> {
        let slot = partition::active_slot().other();
        let (base, capacity) = slot.region();
        Ok(OtaUpdate {
            slot,
            base,
            capacity,
            written: 0,
            next_erase: base,
            header_ok: false,
        })
    }

    pub fn slot(&self) -> Slot {
        self.slot
    }

    pub fn written(&self) -> u32 {
        self.written
    }

    pub fn write(&mut self, data: &[u8]) -> KResult<()> {
        if data.is_empty() {
            return Ok(());
        }
        let len = u32::try_from(data.len()).map_err(|_| KError::InvalidArgument)?;

        let end = self.written.checked_add(len).ok_or(KError::NoSpace)?;
        if end > self.capacity {
            return Err(KError::NoSpace);
        }

        if !self.header_ok {
            match data.first() {
                Some(&b) if b == ESP_IMAGE_MAGIC => self.header_ok = true,
                _ => return Err(KError::Corrupt),
            }
        }

        let write_at = self
            .base
            .checked_add(self.written)
            .ok_or(KError::InvalidArgument)?;
        let write_end = self.base.checked_add(end).ok_or(KError::InvalidArgument)?;

        let sector = flash::SECTOR_SIZE as u32;
        if sector == 0 {
            return Err(KError::InvalidArgument);
        }
        while self.next_erase < write_end {
            flash::erase_sector(self.next_erase)?;
            self.next_erase = self
                .next_erase
                .checked_add(sector)
                .ok_or(KError::InvalidArgument)?;
        }

        flash::write(write_at, data)?;
        self.written = end;
        Ok(())
    }

    pub fn finish(self) -> KResult<()> {
        if !self.header_ok || self.written == 0 {
            return Err(KError::Corrupt);
        }
        partition::set_boot_slot(self.slot)
    }

    pub fn abort(self) {
        let _ = self;
    }
}

pub fn apply_image(image: &[u8]) -> KResult<Slot> {
    let mut upd = OtaUpdate::begin()?;
    let slot = upd.slot;
    upd.write(image)?;
    upd.finish()?;
    Ok(slot)
}
