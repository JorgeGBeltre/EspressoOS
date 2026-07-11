//! Gestión de slots A/B para OTA: parsing, selección y escritura de `otadata`.
//!
//! Implementa el mecanismo de esp-idf: la partición `otadata` contiene DOS copias
//! de una entrada `esp_ota_select_entry_t` (una por sector de 4 KB). Cada entrada
//! lleva un contador de secuencia (`ota_seq`) protegido por CRC-32. En cada boot
//! se elige la copia VÁLIDA con mayor `ota_seq`; su secuencia determina el slot.
//! `set_boot_slot` escribe una entrada nueva (seq mayor) en la copia INACTIVA, de
//! modo que un corte de energía a mitad de escritura preserva siempre la copia
//! buena anterior.
//!
//! Modelo A/B de este kernel (adaptación del contrato): hay 2 slots de arranque
//! en rotación —Factory (índice 0) y Ota0 (índice 1)—, de forma que
//! `slot_index = (ota_seq - 1) % 2`. A diferencia de esp-idf puro (donde `factory`
//! queda fuera de la rotación y se arranca solo cuando NO hay otadata válida),
//! aquí Factory es el "slot A" del par y se puede seleccionar por otadata. La
//! ausencia total de otadata válida arranca Factory por seguridad.
// COMPILE-STATUS: borrador (sin compilar)
#![allow(dead_code)]

use crate::drivers::flash;
use crate::prelude::*;

/// Número de slots de arranque en el esquema A/B (Factory + Ota0).
pub const SLOT_COUNT: u32 = 2;

/// Tamaño en bytes de una entrada `esp_ota_select_entry_t`.
pub const OTA_SELECT_ENTRY_SIZE: usize = 32;

/// Valor de `ota_seq` de una entrada vacía/borrada (flash a 0xFF).
pub const OTA_SEQ_EMPTY: u32 = 0xFFFF_FFFF;

// ---------------------------------------------------------------------------
// Estado de imagen (compatible con `esp_ota_img_states_t` de esp-idf).
// ---------------------------------------------------------------------------

/// Estados de una imagen OTA. Se almacena en `ota_state` de la entrada.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u32)]
pub enum OtaImgState {
    /// Imagen recién escrita, aún sin arrancar.
    New = 0x0,
    /// Arrancada una vez; pendiente de auto-validación.
    PendingVerify = 0x1,
    /// Confirmada como buena.
    Valid = 0x2,
    /// Marcada como inválida (candidata a rollback).
    Invalid = 0x3,
    /// Arranque abortado.
    Aborted = 0x4,
    /// Sin definir (flash borrada).
    Undefined = 0xFFFF_FFFF,
}

impl OtaImgState {
    /// Convierte el valor crudo del otadata a variante. Valores desconocidos -> `Undefined`.
    pub const fn from_raw(v: u32) -> OtaImgState {
        match v {
            0x0 => OtaImgState::New,
            0x1 => OtaImgState::PendingVerify,
            0x2 => OtaImgState::Valid,
            0x3 => OtaImgState::Invalid,
            0x4 => OtaImgState::Aborted,
            _ => OtaImgState::Undefined,
        }
    }

    /// Valor crudo para serializar.
    pub const fn as_raw(self) -> u32 {
        self as u32
    }
}

// ---------------------------------------------------------------------------
// Slot de arranque.
// ---------------------------------------------------------------------------

/// Slot de arranque A/B. `Factory` = slot A, `Ota0` = slot B.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Slot {
    /// Slot A: partición `factory` (`layout::FACTORY_OFFSET`).
    Factory,
    /// Slot B: partición `ota_0` (`layout::OTA0_OFFSET`).
    Ota0,
}

impl Slot {
    /// `(offset, size)` de la partición del slot, tomados de `prelude::layout`.
    pub const fn region(self) -> (u32, u32) {
        match self {
            Slot::Factory => (layout::FACTORY_OFFSET, layout::FACTORY_SIZE),
            Slot::Ota0 => (layout::OTA0_OFFSET, layout::OTA0_SIZE),
        }
    }

    /// Índice del slot en la rotación de secuencias (0 = Factory, 1 = Ota0).
    pub const fn index(self) -> u32 {
        match self {
            Slot::Factory => 0,
            Slot::Ota0 => 1,
        }
    }

    /// Slot a partir de un índice ya reducido módulo `SLOT_COUNT`.
    pub const fn from_index(idx: u32) -> Slot {
        // Solo hay 2 slots; cualquier índice != 0 mapea a Ota0.
        match idx {
            0 => Slot::Factory,
            _ => Slot::Ota0,
        }
    }

    /// El otro slot del par A/B (destino natural de una actualización).
    pub const fn other(self) -> Slot {
        match self {
            Slot::Factory => Slot::Ota0,
            Slot::Ota0 => Slot::Factory,
        }
    }
}

// ---------------------------------------------------------------------------
// Entrada `esp_ota_select_entry_t` (32 bytes).
//
//   offset 0  : ota_seq   (u32 LE)   4 bytes
//   offset 4  : seq_label ([u8; 20]) 20 bytes
//   offset 24 : ota_state (u32 LE)   4 bytes
//   offset 28 : crc       (u32 LE)   4 bytes  (CRC-32 del campo ota_seq)
// ---------------------------------------------------------------------------

/// Copia de selección OTA almacenada en `otadata`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct OtaSelectEntry {
    /// Contador de secuencia; mayor = más reciente. `0xFFFFFFFF` = vacío.
    pub ota_seq: u32,
    /// Etiqueta libre (esp-idf guarda aquí una versión). No afecta a la lógica.
    pub seq_label: [u8; 20],
    /// Estado de la imagen (`OtaImgState`).
    pub ota_state: u32,
    /// CRC-32 del campo `ota_seq` (ver `compute_crc`).
    pub crc: u32,
}

impl OtaSelectEntry {
    /// Entrada "vacía" tal como se leería de flash borrada (todo 0xFF).
    pub const fn empty() -> Self {
        Self {
            ota_seq: OTA_SEQ_EMPTY,
            seq_label: [0xFF; 20],
            ota_state: OTA_SEQ_EMPTY, // == Undefined
            crc: OTA_SEQ_EMPTY,
        }
    }

    /// Crea una entrada con la secuencia y estado dados, con el CRC ya calculado.
    pub fn new(ota_seq: u32, state: OtaImgState) -> Self {
        let mut e = Self {
            ota_seq,
            seq_label: [0x00; 20],
            ota_state: state.as_raw(),
            crc: 0,
        };
        e.crc = e.compute_crc();
        e
    }

    /// CRC-32 esperado del campo `ota_seq`.
    ///
    /// esp-idf calcula `crc32_le(0xFFFFFFFF, &ota_seq, 4)` (variante de la ROM,
    /// polinomio reflejado 0xEDB88320). Aquí se replica esa convención. La lógica
    /// del kernel es autoconsistente (escribe y lee con el mismo CRC); la paridad
    /// bit-a-bit con el bootloader de fábrica es un riesgo marcado en el contrato.
    pub fn compute_crc(&self) -> u32 {
        crc32_le(0xFFFF_FFFF, &self.ota_seq.to_le_bytes())
    }

    /// Entrada válida: secuencia utilizable y CRC correcto.
    ///
    /// Criterio esp-idf: `ota_seq != 0xFFFFFFFF && crc == compute_crc()`. Se añade
    /// la guarda `ota_seq != 0` para no subdesbordar en el mapeo de slot
    /// (`(seq - 1) % SLOT_COUNT`); en operación normal las secuencias empiezan en 1.
    pub fn is_valid(&self) -> bool {
        self.ota_seq != OTA_SEQ_EMPTY && self.ota_seq != 0 && self.crc == self.compute_crc()
    }

    /// Serializa a los 32 bytes exactos del formato en flash.
    pub fn to_bytes(&self) -> [u8; OTA_SELECT_ENTRY_SIZE] {
        let mut b = [0u8; OTA_SELECT_ENTRY_SIZE];
        // Todas las longitudes de origen/destino coinciden -> `copy_from_slice`
        // no puede entrar en pánico.
        b[0..4].copy_from_slice(&self.ota_seq.to_le_bytes());
        b[4..24].copy_from_slice(&self.seq_label);
        b[24..28].copy_from_slice(&self.ota_state.to_le_bytes());
        b[28..32].copy_from_slice(&self.crc.to_le_bytes());
        b
    }

    /// Deserializa desde un búfer (>= 32 bytes) leído de flash.
    pub fn from_bytes(buf: &[u8]) -> KResult<Self> {
        if buf.len() < OTA_SELECT_ENTRY_SIZE {
            return Err(KError::InvalidArgument);
        }
        // `buf` tiene al menos 32 bytes; todos los slices siguientes están en rango.
        let ota_seq = u32_le(&buf[0..4])?;
        let mut seq_label = [0u8; 20];
        seq_label.copy_from_slice(&buf[4..24]);
        let ota_state = u32_le(&buf[24..28])?;
        let crc = u32_le(&buf[28..32])?;
        Ok(Self {
            ota_seq,
            seq_label,
            ota_state,
            crc,
        })
    }
}

// ---------------------------------------------------------------------------
// LÓGICA PURA DE SELECCIÓN (testeable sin hardware).
// ---------------------------------------------------------------------------

/// Elige el índice de copia (0/1) ACTIVA: la entrada válida con mayor `ota_seq`.
/// Devuelve `None` si ninguna de las dos copias es válida. Lógica PURA.
///
/// Empate por secuencia (no debería ocurrir en operación normal): se prefiere la
/// copia 1, replicando `(seq0 > seq1) ? 0 : 1` del bootloader de esp-idf.
pub fn select_active_index(entries: &[OtaSelectEntry; 2]) -> Option<usize> {
    let v0 = entries[0].is_valid();
    let v1 = entries[1].is_valid();
    match (v0, v1) {
        (true, true) => {
            if entries[0].ota_seq > entries[1].ota_seq {
                Some(0)
            } else {
                Some(1)
            }
        }
        (true, false) => Some(0),
        (false, true) => Some(1),
        (false, false) => None,
    }
}

/// Mapea una `ota_seq` válida (>= 1) a su slot de arranque. Lógica PURA.
pub const fn slot_from_seq(seq: u32) -> Slot {
    // En rutas normales `is_valid` garantiza seq >= 1. Se usa `wrapping_sub`
    // para que la función sea panic-free incluso si se invocara con seq == 0.
    Slot::from_index(seq.wrapping_sub(1) % SLOT_COUNT)
}

/// Calcula la próxima `ota_seq` que hará arrancar el slot de índice `target_index`,
/// estrictamente mayor que `current_max` (0 = sin secuencia previa). Lógica PURA.
///
/// Busca la menor secuencia `seq` tal que `seq > current_max`,
/// `seq >= target_index + 1` y `(seq - 1) % SLOT_COUNT == target_index`. Con
/// `SLOT_COUNT == 2` el bucle itera a lo sumo `SLOT_COUNT` veces.
pub fn next_seq_for_index(current_max: u32, target_index: u32) -> KResult<u32> {
    if target_index >= SLOT_COUNT {
        return Err(KError::InvalidArgument);
    }
    // Secuencia mínima que mapea a `target_index` es `target_index + 1`.
    let base = target_index.checked_add(1).ok_or(KError::InvalidArgument)?;
    let mut seq = current_max.checked_add(1).ok_or(KError::NoSpace)?;
    if seq < base {
        seq = base;
    }
    for _ in 0..SLOT_COUNT {
        // seq >= base >= 1; `wrapping_sub` mantiene la función panic-free.
        if seq.wrapping_sub(1) % SLOT_COUNT == target_index {
            return Ok(seq);
        }
        seq = seq.checked_add(1).ok_or(KError::NoSpace)?;
    }
    // Inalcanzable con SLOT_COUNT slots consecutivos.
    Err(KError::InvalidArgument)
}

// ---------------------------------------------------------------------------
// CRC-32 y helpers de bytes (puros).
// ---------------------------------------------------------------------------

/// CRC-32 reflejado (polinomio 0xEDB88320), convención de `esp_rom_crc32_le`:
/// invierte la semilla al entrar y el resultado al salir. Con `seed = 0xFFFFFFFF`
/// reproduce el cálculo que esp-idf hace sobre `ota_seq`. Función PURA.
pub fn crc32_le(seed: u32, data: &[u8]) -> u32 {
    let mut crc = !seed;
    for &byte in data {
        crc ^= byte as u32;
        let mut bit = 0;
        while bit < 8 {
            // `mask` es 0xFFFFFFFF si el bit bajo es 1, si no 0.
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
            bit += 1;
        }
    }
    !crc
}

/// Lee un `u32` little-endian de un slice de al menos 4 bytes. Puro.
fn u32_le(b: &[u8]) -> KResult<u32> {
    let arr: [u8; 4] = b
        .get(0..4)
        .ok_or(KError::InvalidArgument)?
        .try_into()
        .map_err(|_| KError::InvalidArgument)?;
    Ok(u32::from_le_bytes(arr))
}

// ---------------------------------------------------------------------------
// E/S de flash (frontera con `drivers::flash`).
// ---------------------------------------------------------------------------

/// Offset en flash de la copia `copy` (0 o 1) de otadata.
fn otadata_copy_offset(copy: usize) -> KResult<u32> {
    let sector = flash::SECTOR_SIZE as u32;
    match copy {
        0 => Ok(layout::OTADATA_OFFSET),
        1 => layout::OTADATA_OFFSET
            .checked_add(sector)
            .ok_or(KError::InvalidArgument),
        _ => Err(KError::InvalidArgument),
    }
}

/// Lee las dos copias de otadata desde flash. Copias ilegibles se devuelven
/// como `empty()` (se tratarán como inválidas por `is_valid`).
fn read_otadata() -> KResult<[OtaSelectEntry; 2]> {
    let mut entries = [OtaSelectEntry::empty(); 2];
    for (i, slot) in entries.iter_mut().enumerate() {
        let off = otadata_copy_offset(i)?;
        let mut raw = [0u8; OTA_SELECT_ENTRY_SIZE];
        flash::read(off, &mut raw)?;
        *slot = OtaSelectEntry::from_bytes(&raw)?;
    }
    Ok(entries)
}

/// Borra y reescribe una copia de otadata con `entry`.
fn write_otadata_copy(copy: usize, entry: &OtaSelectEntry) -> KResult<()> {
    let off = otadata_copy_offset(copy)?;
    // Un sector completo de borrado antes de escribir la entrada.
    flash::erase_sector(off)?;
    flash::write(off, &entry.to_bytes())
}

// ---------------------------------------------------------------------------
// API pública del contrato (§3.8).
// ---------------------------------------------------------------------------

/// Lee `otadata` y devuelve el slot desde el que se arranca / se arrancará.
///
/// Ante otadata ilegible o sin ninguna copia válida, devuelve `Slot::Factory`
/// (comportamiento seguro por defecto).
pub fn active_slot() -> Slot {
    let entries = match read_otadata() {
        Ok(e) => e,
        Err(_) => return Slot::Factory,
    };
    match select_active_index(&entries) {
        Some(i) => entries
            .get(i)
            .map(|e| slot_from_seq(e.ota_seq))
            .unwrap_or(Slot::Factory),
        None => Slot::Factory,
    }
}

/// Marca `slot` como el de arranque escribiendo una entrada nueva en `otadata`.
///
/// Escribe en la copia INACTIVA con una secuencia mayor que la activa actual, de
/// modo que en el siguiente arranque `active_slot` la seleccione. Un fallo de
/// energía a mitad conserva la copia previa intacta.
pub fn set_boot_slot(slot: Slot) -> KResult<()> {
    // Si otadata no es legible, partimos de dos copias vacías (ambas inválidas).
    let entries = read_otadata().unwrap_or([OtaSelectEntry::empty(); 2]);
    let active = select_active_index(&entries);

    // Secuencia máxima válida en uso (0 si no hay ninguna).
    let current_max = match active {
        Some(i) => entries.get(i).map(|e| e.ota_seq).unwrap_or(0),
        None => 0,
    };

    let target_index = slot.index();
    let new_seq = next_seq_for_index(current_max, target_index)?;

    // Escribir en la copia que NO es la activa (si no hay activa, la copia 0).
    let write_copy = match active {
        Some(0) => 1usize,
        Some(_) => 0usize,
        None => 0usize,
    };

    let entry = OtaSelectEntry::new(new_seq, OtaImgState::New);
    write_otadata_copy(write_copy, &entry)
}
