//! Intercambio de claves SSH: `curve25519-sha256` (ESQUELETO — RFC 8731/5656).
// COMPILE-STATUS: borrador (sin compilar)
//!
//! Delega TODA la criptografía a crates auditadas:
//!   - X25519 (ECDH efímero): `x25519-dalek` v2.
//!   - SHA-256 (hash de intercambio H y KDF): `sha2`.
//!   - Firma de la clave de host: `ed25519-dalek` v2 (en `super::auth`/`mod`).
//!   - Aleatoriedad: TRNG del ESP32-S3 (`esp_hal::rng::Rng`) vía `rand_core`.
//!
//! Este archivo SÓLO orquesta: NO implementa curvas ni hashes a mano.
#![allow(dead_code)]

use super::proto::Writer;
use crate::prelude::*;

// use x25519_dalek::{EphemeralSecret, PublicKey};   // (?) confirmar al compilar
// use sha2::{Digest, Sha256};                        // (?)

/// Nombre del algoritmo de kex negociado.
pub const KEX_NAME: &str = "curve25519-sha256";
/// Nombre del algoritmo de clave de host.
pub const HOSTKEY_NAME: &str = "ssh-ed25519";

/// Resultado de un kex correcto: el hash de intercambio (y session_id) y el
/// secreto compartido, de los que se derivan todas las claves.
pub struct KexOutput {
    /// Hash de intercambio H (también session_id en el primer kex). 32 bytes.
    pub h: [u8; 32],
    /// Secreto compartido K (big-endian, para codificar como mpint).
    pub k: Vec<u8>,
}

/// Calcula el hash de intercambio H (RFC 4253 §8, adaptado a curve25519).
///
/// H = SHA256( V_C || V_S || I_C || I_S || K_S || Q_C || Q_S || K )
/// donde cada término se serializa como `string`/`mpint` según RFC 4251. La
/// serialización usa `proto::Writer` (ya probado); el hash lo hace `sha2`.
pub fn exchange_hash(
    v_c: &[u8], // ident del cliente (sin CRLF)
    v_s: &[u8], // ident del servidor
    i_c: &[u8], // KEXINIT del cliente (payload)
    i_s: &[u8], // KEXINIT del servidor
    k_s: &[u8], // clave de host pública (blob ssh-ed25519)
    q_c: &[u8], // pública efímera del cliente (32 bytes)
    q_s: &[u8], // pública efímera del servidor (32 bytes)
    k: &[u8],   // secreto compartido (big-endian)
) -> [u8; 32] {
    let mut w = Writer::new();
    w.put_string(v_c)
        .put_string(v_s)
        .put_string(i_c)
        .put_string(i_s)
        .put_string(k_s)
        .put_string(q_c)
        .put_string(q_s)
        .put_mpint_uint(k);
    // TODO(fase-red): Sha256::digest(w.as_slice()) -> [u8;32]
    let _bytes = w.into_bytes();
    [0u8; 32] // marcador hasta enlazar sha2
}

/// Deriva una clave de sesión (RFC 4253 §7.2): K1 = HASH(K || H || X || session_id),
/// extendida con K2 = HASH(K || H || K1)... hasta `out.len()`. `x` es la letra
/// A..F que identifica cada clave (IV/clave/MAC de cada sentido). ESQUELETO.
pub fn derive_key(_k: &[u8], _h: &[u8; 32], _x: u8, _session_id: &[u8; 32], out: &mut [u8]) {
    // TODO(fase-red): bucle de SHA-256 encadenado como en RFC 4253 §7.2.
    for b in out.iter_mut() {
        *b = 0;
    }
}

/// Ejecuta el kex del lado servidor: recibe Q_C, genera par efímero, calcula K,
/// H y firma H con la clave de host. ESQUELETO.
pub fn run_server(_q_c: &[u8]) -> KResult<KexOutput> {
    // TODO(fase-red):
    //  1. EphemeralSecret::random_from_rng(hw_rng) -> secreto; PublicKey -> Q_S.
    //  2. K = secreto.diffie_hellman(&PublicKey::from(q_c)).
    //  3. H = exchange_hash(...); firmar H con ed25519 (host key).
    //  4. Enviar SSH_MSG_KEX_ECDH_REPLY { K_S, Q_S, sig(H) }.
    Err(KError::NotSupported)
}
