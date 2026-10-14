//! Persistencia de clave de host SSH en NVS Flash (sector 1, 0xA000).
//!
//! Garantiza que la clave Ed25519 del host SSH sea persistente y única por placa:
//! - Si existe un registro válido con magic "ESHK" en NVS, se carga la semilla.
//! - Si no existe, se genera una semilla aleatoria mediante el TRNG de hardware y
//!   se almacena en el sector 1 de NVS (0x0000_A000).
//! - Si falla el acceso a flash, el sistema continúa con una clave efímera segura.

use crate::drivers::flash;
use crate::prelude::*;

const MAGIC: &[u8; 4] = b"ESHK";
const OFFSET: u32 = layout::NVS_OFFSET + 0x1000; // 0x0000_A000 (sector 1 de NVS)
const RECORD_LEN: usize = 64;

fn checksum(seed: &[u8; 32]) -> u32 {
    let mut sum: u32 = 0x1234_5678;
    for &b in seed {
        sum = sum.rotate_left(3) ^ (b as u32);
    }
    sum
}

/// Lee la semilla de 32 bytes de la clave de host guardada en NVS flash.
pub fn load_seed() -> Option<[u8; 32]> {
    let mut rec = [0u8; RECORD_LEN];
    flash::read(OFFSET, &mut rec).ok()?;
    if &rec[0..4] != MAGIC || rec[4] != 1 {
        return None;
    }
    let mut seed = [0u8; 32];
    seed.copy_from_slice(&rec[8..40]);
    let stored_sum = u32::from_le_bytes(rec[40..44].try_into().ok()?);
    if checksum(&seed) != stored_sum {
        return None;
    }
    Some(seed)
}

/// Guarda la semilla de 32 bytes en el sector 1 de NVS flash.
pub fn save_seed(seed: &[u8; 32]) -> KResult<()> {
    let mut rec = [0u8; RECORD_LEN];
    rec[0..4].copy_from_slice(MAGIC);
    rec[4] = 1; // version 1
    rec[8..40].copy_from_slice(seed);
    let sum = checksum(seed);
    rec[40..44].copy_from_slice(&sum.to_le_bytes());

    flash::erase_sector(OFFSET)?;
    flash::write(OFFSET, &rec)?;
    Ok(())
}

/// Borra la clave de host de NVS flash (forzará regeneración en el siguiente arranque).
pub fn clear_seed() -> KResult<()> {
    flash::erase_sector(OFFSET)
}
