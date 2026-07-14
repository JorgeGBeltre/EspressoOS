#![allow(dead_code)]
//! Almacén PERSISTENTE de credenciales WiFi en flash.
//!
//! Guarda SSID/contraseña en el primer sector de la región NVS (`0x9000`), que en
//! este montaje bare-metal está libre (esp-wifi no usa el NVS de ESP-IDF, y no se
//! toca `phy_init` en `0xF000`). Sobrevive a reinicios y re-flasheos del kernel
//! (la región de datos no se reescribe al flashear la app). Se escribe al hacer
//! `wifi connect` y se lee al arrancar el `net_task`.
//!
//! Formato del registro (128 bytes, alineado a 4):
//! ```text
//!   0..4    magic  "EWC1"
//!   4       ssid_len (u8, <= 32)
//!   5       pass_len (u8, <= 64)
//!   6..8    reservado
//!   8..40   ssid (relleno con ceros)
//!  40..104  password (relleno con ceros)
//! ```

use crate::drivers::flash;
use crate::prelude::*;

const MAGIC: &[u8; 4] = b"EWC1";
const OFFSET: u32 = layout::NVS_OFFSET; // 0x9000, primer sector (4 KB)
const RECORD_LEN: usize = 128;
const SSID_OFF: usize = 8;
const SSID_MAX: usize = 32;
const PASS_OFF: usize = 40;
const PASS_MAX: usize = 64;

/// Guarda SSID/contraseña en flash (borra el sector y escribe el registro).
pub fn save(ssid: &str, password: &str) -> KResult<()> {
    let sb = ssid.as_bytes();
    let pb = password.as_bytes();
    if sb.len() > SSID_MAX || pb.len() > PASS_MAX {
        return Err(KError::InvalidArgument);
    }
    let mut rec = [0u8; RECORD_LEN];
    rec[0..4].copy_from_slice(MAGIC);
    rec[4] = sb.len() as u8;
    rec[5] = pb.len() as u8;
    rec[SSID_OFF..SSID_OFF + sb.len()].copy_from_slice(sb);
    rec[PASS_OFF..PASS_OFF + pb.len()].copy_from_slice(pb);
    flash::erase_sector(OFFSET)?;
    flash::write(OFFSET, &rec)?;
    Ok(())
}

/// Lee las credenciales guardadas, si el registro es válido (magic correcto).
pub fn load() -> Option<(String, String)> {
    let mut rec = [0u8; RECORD_LEN];
    flash::read(OFFSET, &mut rec).ok()?;
    if &rec[0..4] != MAGIC {
        return None;
    }
    let sl = rec[4] as usize;
    let pl = rec[5] as usize;
    if sl > SSID_MAX || pl > PASS_MAX {
        return None;
    }
    let ssid = core::str::from_utf8(&rec[SSID_OFF..SSID_OFF + sl]).ok()?;
    let pass = core::str::from_utf8(&rec[PASS_OFF..PASS_OFF + pl]).ok()?;
    Some((String::from(ssid), String::from(pass)))
}

/// Borra las credenciales guardadas (el próximo arranque usa las de compilación).
pub fn clear() -> KResult<()> {
    flash::erase_sector(OFFSET)
}
