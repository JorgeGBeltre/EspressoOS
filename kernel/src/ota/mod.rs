#![allow(dead_code)]

use crate::arch::xtensa::sync::Mutex;
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

// ===========================================================================
// Recepción de imagen OTA (Fase 5).
//
// Estrategia segura frente al hazard "flash + WiFi": la imagen se recibe por TCP
// a un buffer en PSRAM (NO se escribe flash durante la transferencia, así la
// radio no se perturba). El flasheo real (que sí deshabilita interrupciones por
// sector y puede molestar al WiFi) se dispara aparte con `apply_buffered()`, que
// el usuario invoca desde la shell (`ota apply`) — idealmente por la consola UART.
// ===========================================================================

/// Tamaño máximo de imagen que se acepta en el buffer (= tamaño del slot OTA).
const MAX_IMAGE: usize = layout::OTA0_SIZE as usize;

static RX_IMAGE: Mutex<Option<Vec<u8>>> = Mutex::new(None);

/// Empieza (o reinicia) la recepción de una imagen.
pub fn rx_begin() {
    *RX_IMAGE.lock() = Some(Vec::new());
}

/// Añade un trozo recibido al buffer. Devuelve el total acumulado.
pub fn rx_push(data: &[u8]) -> KResult<usize> {
    let mut g = RX_IMAGE.lock();
    let buf = g.get_or_insert_with(Vec::new);
    if buf.len().saturating_add(data.len()) > MAX_IMAGE {
        return Err(KError::NoSpace);
    }
    buf.try_reserve(data.len()).map_err(|_| KError::NoMem)?;
    buf.extend_from_slice(data);
    Ok(buf.len())
}

/// Bytes actualmente en el buffer de recepción.
pub fn rx_len() -> usize {
    RX_IMAGE.lock().as_ref().map(|b| b.len()).unwrap_or(0)
}

/// Descarta el buffer de recepción.
pub fn rx_clear() {
    *RX_IMAGE.lock() = None;
}

/// Flashea la imagen recibida en el slot inactivo y marca el arranque.
///
/// ⚠️ Escribe varios MB a flash: con el WiFi activo puede perturbar la radio.
/// Preferir ejecutarlo desde la consola UART. Un `Ok(slot)` deja el otadata
/// apuntando al nuevo slot (efectivo sólo con un bootloader que honre otadata).
pub fn apply_buffered() -> KResult<Slot> {
    let image = RX_IMAGE.lock().take().ok_or(KError::NotFound)?;
    if image.is_empty() {
        return Err(KError::Corrupt);
    }
    validate_header(&image)?;
    apply_image(&image)
}

pub fn get_state() -> KResult<OtaImgState> {
    partition::get_state()
}

pub fn set_state(state: OtaImgState) -> KResult<()> {
    partition::set_state(state)
}
