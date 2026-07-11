//! Capa de cifrado de sesión: `chacha20-poly1305@openssh.com` (ESQUELETO).
// COMPILE-STATUS: borrador (sin compilar)
//!
//! AEAD del transporte SSH tras NEWKEYS. Se compone a partir de primitivas
//! auditadas de RustCrypto (`chacha20` + `poly1305`), NO del crate
//! `chacha20poly1305` (que implementa el AEAD RFC 8439, distinto del de openssh).
//!
//! Construcción openssh (resumen):
//!   - Dos claves de 256 bits: K_1 (cifra el `packet_length`) y K_2 (cifra el resto).
//!   - Nonce = número de secuencia del paquete (u64 big-endian).
//!   - La etiqueta Poly1305 usa una clave derivada del primer bloque del keystream
//!     de K_2 con contador 0; el payload se cifra con contador 1.
//!   - La longitud va cifrada aparte con K_1 para poder saber cuánto leer.
//!
//! Se implementa esa envoltura a mano (es *encuadre*, no criptografía nueva);
//! ChaCha20 y Poly1305 en sí los provee la crate.
#![allow(dead_code)]

use crate::prelude::*;

// use chacha20::ChaCha20;      // (?) confirmar API/versión al compilar
// use poly1305::Poly1305;      // (?)
// use subtle::ConstantTimeEq;  // comparación de etiquetas en tiempo constante

/// Nombre del cifrado negociado.
pub const CIPHER_NAME: &str = "chacha20-poly1305@openssh.com";
/// Longitud de la etiqueta Poly1305.
pub const TAG_LEN: usize = 16;
/// Longitud del campo de longitud cifrado.
pub const LEN_LEN: usize = 4;

/// Estado AEAD de un sentido (cliente→servidor o servidor→cliente).
pub struct Aead {
    k1: [u8; 32], // clave para la longitud
    k2: [u8; 32], // clave para el payload + etiqueta
    seq: u32,     // número de secuencia del paquete (nonce)
}

impl Aead {
    /// Construye el estado a partir de las 64 bytes de material de clave
    /// derivado en el kex (K_2 || K_1, orden de openssh).
    pub fn new(key_material: &[u8; 64]) -> Self {
        let mut k2 = [0u8; 32];
        let mut k1 = [0u8; 32];
        k2.copy_from_slice(&key_material[..32]);
        k1.copy_from_slice(&key_material[32..]);
        Self { k1, k2, seq: 0 }
    }

    /// Cifra `payload` ya enmarcado (proto::frame_packet) produciendo el registro
    /// de cable: [len cifrada (4)] [payload cifrado] [tag (16)]. ESQUELETO.
    pub fn seal(&mut self, _framed: &[u8]) -> KResult<Vec<u8>> {
        // TODO(fase-red): ChaCha20(k1, nonce=seq).apply(len); Poly1305 key = ChaCha20(k2,seq,ctr0);
        // ciphertext = ChaCha20(k2,seq,ctr1).apply(rest); tag = Poly1305(len||ct).
        self.seq = self.seq.wrapping_add(1);
        Err(KError::NotSupported)
    }

    /// Descifra la longitud a partir de los primeros 4 bytes (necesario para saber
    /// cuántos leer del socket). ESQUELETO.
    pub fn open_length(&self, _enc_len: &[u8; 4]) -> u32 {
        // TODO(fase-red): ChaCha20(k1, nonce=seq).apply_keystream sobre enc_len.
        0
    }

    /// Verifica la etiqueta y descifra el registro completo → payload en claro.
    /// Devuelve `Err(InvalidArgument)` si la etiqueta no valida (en tiempo constante).
    pub fn open(&mut self, _record: &[u8]) -> KResult<Vec<u8>> {
        // TODO(fase-red): recomputar Poly1305 y comparar con subtle::ConstantTimeEq;
        // si ok, descifrar payload y devolver; incrementar seq.
        self.seq = self.seq.wrapping_add(1);
        Err(KError::NotSupported)
    }
}
