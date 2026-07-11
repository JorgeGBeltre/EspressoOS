//! Lectura de flash del bootloader (ESQUELETO).
//!
//! Lee regiones de la SPI flash (vía mapeo a caché o comandos SPI) para cargar
//! la tabla de particiones y los segmentos de la imagen del kernel.
#![allow(dead_code)]

/// Lee `buf.len()` bytes desde `offset` de la flash. ESQUELETO.
pub fn read(_offset: u32, _buf: &mut [u8]) -> Result<(), ()> {
    // TODO(fase-bootloader): mapear a caché o usar el periférico SPI0/1.
    Err(())
}
