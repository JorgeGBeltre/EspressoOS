//! Actualización Over-The-Air (OTA): orquestación de escritura y conmutación.
//!
//! Coordina la instalación de una imagen nueva en el slot INACTIVO
//! (`factory`/`ota_0`), su validación mínima y la conmutación de arranque vía
//! `partition::set_boot_slot`. La selección/estado de slots vive en
//! `partition` (lógica pura); aquí solo se orquesta la E/S de flash a través de
//! `drivers::flash`.
//!
//! Flujo típico:
//! ```ignore
//! let mut upd = ota::OtaUpdate::begin()?;   // apunta al slot inactivo
//! upd.write(&chunk1)?;                        // escritura secuencial, borrado perezoso
//! upd.write(&chunk2)?;
//! upd.finish()?;                             // valida + conmuta arranque
//! ```
// COMPILE-STATUS: borrador (sin compilar)
#![allow(dead_code)]

use crate::drivers::flash;
use crate::prelude::*;

pub mod partition;

pub use partition::{OtaImgState, Slot};

/// Primer byte del header de una imagen de aplicación ESP32 (magic).
/// Validación mínima de que lo que se escribe parece una imagen real.
pub const ESP_IMAGE_MAGIC: u8 = 0xE9;

/// Slot desde el que arrancó (o arrancará) el sistema.
pub fn active_slot() -> Slot {
    partition::active_slot()
}

/// Slot inactivo: destino natural de una nueva imagen OTA.
pub fn inactive_slot() -> Slot {
    partition::active_slot().other()
}

/// Marca explícitamente `slot` como el de arranque (rollback manual incluido).
pub fn set_boot_slot(slot: Slot) -> KResult<()> {
    partition::set_boot_slot(slot)
}

/// Sesión de escritura de una imagen OTA hacia el slot inactivo.
///
/// La escritura es SECUENCIAL: cada `write` continúa donde acabó el anterior. Los
/// sectores se borran de forma perezosa justo antes de necesitarse, así una
/// imagen que no llene el slot no paga el borrado del resto.
pub struct OtaUpdate {
    /// Slot destino (el inactivo en el momento de `begin`).
    slot: Slot,
    /// Offset base del slot en flash.
    base: u32,
    /// Capacidad del slot en bytes.
    capacity: u32,
    /// Bytes ya escritos.
    written: u32,
    /// Próximo offset de flash aún sin borrar (avanza por sectores).
    next_erase: u32,
    /// El primer fragmento traía el magic válido.
    header_ok: bool,
}

impl OtaUpdate {
    /// Inicia una actualización hacia el slot INACTIVO. No borra nada todavía.
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

    /// Slot destino de esta sesión.
    pub fn slot(&self) -> Slot {
        self.slot
    }

    /// Bytes escritos hasta ahora.
    pub fn written(&self) -> u32 {
        self.written
    }

    /// Escribe el siguiente fragmento de imagen de forma secuencial.
    ///
    /// - Comprueba que no se rebase la capacidad del slot (`KError::NoSpace`).
    /// - Valida el magic (`0xE9`) del primer byte de la imagen (`KError::Corrupt`).
    /// - Borra perezosamente los sectores que el fragmento vaya a ocupar.
    pub fn write(&mut self, data: &[u8]) -> KResult<()> {
        if data.is_empty() {
            return Ok(());
        }
        let len = u32::try_from(data.len()).map_err(|_| KError::InvalidArgument)?;

        // Capacidad del slot.
        let end = self.written.checked_add(len).ok_or(KError::NoSpace)?;
        if end > self.capacity {
            return Err(KError::NoSpace);
        }

        // Validación del magic en el primer fragmento no vacío.
        if !self.header_ok {
            match data.first() {
                Some(&b) if b == ESP_IMAGE_MAGIC => self.header_ok = true,
                _ => return Err(KError::Corrupt),
            }
        }

        // Direcciones absolutas de flash [write_at, write_end).
        let write_at = self
            .base
            .checked_add(self.written)
            .ok_or(KError::InvalidArgument)?;
        let write_end = self.base.checked_add(end).ok_or(KError::InvalidArgument)?;

        // Borrado perezoso: erase de cada sector aún no borrado que toque el rango.
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

    /// Finaliza: valida mínimamente la imagen y conmuta el slot de arranque.
    ///
    /// Falla con `KError::Corrupt` si no se escribió una imagen con magic válido.
    pub fn finish(self) -> KResult<()> {
        if !self.header_ok || self.written == 0 {
            return Err(KError::Corrupt);
        }
        partition::set_boot_slot(self.slot)
    }

    /// Aborta la sesión sin conmutar el arranque; el slot activo queda intacto.
    ///
    /// Consume la sesión. Los sectores ya borrados/escritos del slot inactivo
    /// quedan como están, pero `otadata` no se toca, así que el próximo arranque
    /// sigue usando el slot activo previo.
    pub fn abort(self) {
        let _ = self;
    }
}

/// Conveniencia: aplica una imagen completa en memoria al slot inactivo y conmuta
/// el arranque. Devuelve el slot en el que quedó instalada.
pub fn apply_image(image: &[u8]) -> KResult<Slot> {
    let mut upd = OtaUpdate::begin()?;
    let slot = upd.slot;
    upd.write(image)?;
    upd.finish()?;
    Ok(slot)
}
