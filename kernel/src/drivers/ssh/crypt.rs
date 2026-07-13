//! Capa de cifrado de sesión: `chacha20-poly1305@openssh.com`.
// COMPILE-STATUS: borrador (implementado, sin compilar contra HW)
//!
//! AEAD del transporte SSH tras NEWKEYS. Se compone a partir de primitivas
//! auditadas de RustCrypto (`chacha20` + `poly1305`), NO del crate
//! `chacha20poly1305` (que implementa el AEAD RFC 8439, distinto del de openssh).
//!
//! Construcción openssh (verificada contra OpenSSH `PROTOCOL.chacha20poly1305`):
//!   - Dos claves de 256 bits: `K_1` (cifra el `packet_length`) y `K_2` (cifra el
//!     resto + deriva la clave Poly1305).
//!   - Nonce = número de secuencia del paquete (`u64` big-endian).
//!   - Clave Poly1305 = primeros 32 bytes del keystream de `K_2` con contador 0.
//!   - El payload (`padlen || payload || padding`) se cifra con `K_2`, contador 1
//!     (offset de byte 64 -> `seek(64)`).
//!   - La longitud (4 bytes) se cifra aparte con `K_1`, contador 0.
//!   - Tag Poly1305 sobre `enc_length(4) || enc_payload` (los dos ciphertexts).
//!
//! DECISIÓN DE DISEÑO (número de secuencia): a diferencia del borrador previo,
//! el `seq` NO vive dentro de `Aead`. Lo mantiene `super::Connection` como dos
//! contadores globales (`send_seq`/`recv_seq`) que NUNCA se reinician con NEWKEYS
//! (RFC 4253 §6.4: el contador cuenta TODOS los paquetes binarios desde el primero
//! tras el intercambio de versión, cifrados o no). `seal`/`open` reciben el `seq`
//! explícito -> se elimina de raíz el bug clásico de reiniciar el contador al
//! activar el cifrado.
#![allow(dead_code)]

use crate::prelude::*;

use chacha20::cipher::{KeyIvInit, StreamCipher, StreamCipherSeek};
use chacha20::ChaCha20Legacy; // variante djb (nonce de 8 bytes) de openssh
use poly1305::universal_hash::{KeyInit, UniversalHash};
use poly1305::Poly1305;
use subtle::ConstantTimeEq; // comparación de etiquetas en tiempo constante

/// Nombre del cifrado negociado.
pub const CIPHER_NAME: &str = "chacha20-poly1305@openssh.com";
/// Longitud de la etiqueta Poly1305.
pub const TAG_LEN: usize = 16;
/// Longitud del campo de longitud cifrado.
pub const LEN_LEN: usize = 4;

/// Estado AEAD de un sentido (cliente→servidor o servidor→cliente).
///
/// Contiene sólo las dos claves; el número de secuencia lo aporta el llamador.
/// `Clone` es barato (64 bytes) y permite sacar una copia para sellar/abrir sin
/// mantener un préstamo de `Connection` mientras se muta su buffer.
#[derive(Clone)]
pub struct Aead {
    k1: [u8; 32], // clave para el `packet_length`
    k2: [u8; 32], // clave para el payload + la etiqueta
}

impl Aead {
    /// Construye el estado a partir de los 64 bytes de material de clave derivado
    /// en el kex. En la construcción openssh el bloque es `K_2 || K_1`
    /// (RFC/PROTOCOL: "The first 256 bits constitute K_2 and the second 256 bits
    /// become K_1").
    pub fn new(key_material: &[u8; 64]) -> Self {
        let mut k2 = [0u8; 32];
        let mut k1 = [0u8; 32];
        k2.copy_from_slice(&key_material[..32]);
        k1.copy_from_slice(&key_material[32..]);
        Self { k1, k2 }
    }

    /// Nonce de 8 bytes = número de secuencia como `uint64` big-endian.
    #[inline]
    fn nonce(seq: u32) -> [u8; 8] {
        (seq as u64).to_be_bytes()
    }

    /// Cifra `framed` (ya producido por `proto::frame_packet(payload, 8, pad)`,
    /// esto es `packet_length(4) || padlen(1) || payload || padding`) y devuelve el
    /// registro de cable: `[len cifrada (4)] [payload cifrado] [tag (16)]`.
    pub fn seal(&self, framed: &[u8], seq: u32) -> KResult<Vec<u8>> {
        if framed.len() < 4 {
            return Err(KError::InvalidArgument);
        }
        let nonce = Self::nonce(seq);
        let (len_pt, rest_pt) = framed.split_at(4);

        // 1) Longitud con K_1, contador 0.
        let mut enc_len = [0u8; 4];
        enc_len.copy_from_slice(len_pt);
        let mut c1 = ChaCha20Legacy::new_from_slices(&self.k1, &nonce)
            .map_err(|_| KError::InvalidArgument)?;
        c1.apply_keystream(&mut enc_len);

        // 2) Clave Poly1305 desde K_2 contador 0; payload desde contador 1.
        let mut c2 = ChaCha20Legacy::new_from_slices(&self.k2, &nonce)
            .map_err(|_| KError::InvalidArgument)?;
        let mut poly_key = [0u8; 32];
        c2.apply_keystream(&mut poly_key); // block 0 -> clave Poly1305
        c2.seek(64u64); // saltar al block 1
        let mut enc_rest = rest_pt.to_vec();
        c2.apply_keystream(&mut enc_rest);

        // 3) Tag Poly1305 sobre enc_len || enc_rest (un solo `update_padded`).
        let mut mac =
            Poly1305::new_from_slice(&poly_key).map_err(|_| KError::InvalidArgument)?;
        let mut aad = Vec::with_capacity(4 + enc_rest.len());
        aad.extend_from_slice(&enc_len);
        aad.extend_from_slice(&enc_rest);
        mac.update_padded(&aad);
        let tag = mac.finalize();

        let mut out = Vec::with_capacity(4 + enc_rest.len() + TAG_LEN);
        out.extend_from_slice(&enc_len);
        out.extend_from_slice(&enc_rest);
        out.extend_from_slice(tag.as_slice());
        Ok(out)
    }

    /// Descifra sólo la longitud a partir de los 4 primeros bytes del registro
    /// (necesario para saber cuántos bytes leer del socket antes de tener el
    /// paquete completo). NO consume ni verifica nada.
    pub fn open_length(&self, enc_len: &[u8; 4], seq: u32) -> KResult<u32> {
        let nonce = Self::nonce(seq);
        let mut buf = *enc_len;
        let mut c1 = ChaCha20Legacy::new_from_slices(&self.k1, &nonce)
            .map_err(|_| KError::InvalidArgument)?;
        c1.apply_keystream(&mut buf);
        Ok(u32::from_be_bytes(buf))
    }

    /// Verifica la etiqueta (en tiempo constante) y descifra el registro completo.
    ///
    /// `record = enc_len(4) || enc_ct(packet_length) || tag(16)`. Devuelve el
    /// `payload` en claro (sin `padding_length` ni padding). `Err(InvalidArgument)`
    /// si la etiqueta no valida — en cuyo caso NO se descifra el payload.
    pub fn open(&self, record: &[u8], seq: u32) -> KResult<Vec<u8>> {
        if record.len() < 4 + TAG_LEN {
            return Err(KError::InvalidArgument);
        }
        let nonce = Self::nonce(seq);
        let (enc_len, tail) = record.split_at(4);
        let (enc_ct, tag_rx) = tail.split_at(tail.len() - TAG_LEN);

        // 1) Recomputar la clave Poly1305 (K_2 contador 0) y verificar el tag
        //    ANTES de descifrar (encrypt-then-MAC / verificar-antes-de-descifrar).
        let mut c2 = ChaCha20Legacy::new_from_slices(&self.k2, &nonce)
            .map_err(|_| KError::InvalidArgument)?;
        let mut poly_key = [0u8; 32];
        c2.apply_keystream(&mut poly_key);
        let mut mac =
            Poly1305::new_from_slice(&poly_key).map_err(|_| KError::InvalidArgument)?;
        let mut aad = Vec::with_capacity(4 + enc_ct.len());
        aad.extend_from_slice(enc_len);
        aad.extend_from_slice(enc_ct);
        mac.update_padded(&aad);
        let tag = mac.finalize();
        if tag.as_slice().ct_eq(tag_rx).unwrap_u8() != 1 {
            return Err(KError::InvalidArgument);
        }

        // 2) Descifrar el payload con K_2 contador 1.
        c2.seek(64u64);
        let mut pt = enc_ct.to_vec();
        c2.apply_keystream(&mut pt); // = padlen || payload || padding
        if pt.is_empty() {
            return Err(KError::InvalidArgument);
        }
        let pad_len = pt[0] as usize;
        if 1 + pad_len > pt.len() {
            return Err(KError::InvalidArgument);
        }
        let payload = pt[1..pt.len() - pad_len].to_vec();
        Ok(payload)
    }
}
