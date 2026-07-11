//! Lectura de la tabla de particiones desde flash (ESQUELETO).
//!
//! La tabla vive en el offset 0x8000. Cada entrada son 32 bytes con magic
//! 0xAA50. Ver `tools/partition-gen/partition_gen.py` para el generador y el
//! formato exacto. Aquí se define la vista de solo-lectura que usará el
//! bootloader para elegir el slot de arranque.
#![allow(dead_code)]

/// Offset por defecto de la tabla de particiones en flash.
pub const PARTITION_TABLE_OFFSET: u32 = 0x8000;
/// Magic de una entrada de partición normal.
pub const PARTITION_MAGIC: u16 = 0xAA50;

#[derive(Clone, Copy)]
#[repr(u8)]
pub enum PartitionType {
    App = 0,
    Data = 1,
}

/// Entrada de 32 bytes de la tabla de particiones.
#[repr(C, packed)]
pub struct PartitionEntry {
    pub magic: u16,
    pub ptype: u8,
    pub subtype: u8,
    pub offset: u32,
    pub size: u32,
    pub label: [u8; 16],
    pub flags: u32,
}

// TODO(fase-bootloader): iterar entradas hasta la de MD5 (magic 0xEBEB).
