//! Intercambio de claves SSH: `curve25519-sha256` (RFC 8731/5656).
// COMPILE-STATUS: borrador (implementado, sin compilar contra HW)
//!
//! Delega TODA la criptografía a crates auditadas:
//!   - X25519 (ECDH efímero): `x25519-dalek` v2 (`EphemeralSecret`).
//!   - SHA-256 (hash de intercambio H y KDF): `sha2`.
//!   - Firma de la clave de host: `ed25519-dalek` v2 (`SigningKey`).
//!   - Aleatoriedad: TRNG del ESP32-S3 vía `super::crypto_rng::HwRng`.
//!
//! Este archivo SÓLO orquesta: NO implementa curvas ni hashes a mano.
#![allow(dead_code)]

use super::crypto_rng::HwRng;
use super::proto::Writer;
use crate::prelude::*;

use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use sha2::{Digest, Sha256};
use x25519_dalek::{EphemeralSecret, PublicKey};

/// Nombre del algoritmo de kex negociado.
pub const KEX_NAME: &str = "curve25519-sha256";
/// Alias idéntico (libssh) que también anunciamos por compatibilidad.
pub const KEX_NAME_ALIAS: &str = "curve25519-sha256@libssh.org";
/// Nombre del algoritmo de clave de host.
pub const HOSTKEY_NAME: &str = "ssh-ed25519";

/// Longitud de una coordenada pública X25519 (Q_C / Q_S).
pub const X25519_POINT_LEN: usize = 32;

/// Construye el blob público de la clave de host `K_S` (RFC 8709):
///   `string "ssh-ed25519" || string pub(32)`.
pub fn host_key_blob(vk: &VerifyingKey) -> Vec<u8> {
    let mut w = Writer::new();
    w.put_string(HOSTKEY_NAME.as_bytes())
        .put_string(vk.as_bytes());
    w.into_bytes()
}

/// Empaqueta una firma ed25519 como blob de cable (RFC 8709):
///   `string "ssh-ed25519" || string sig(64)`.
pub fn signature_blob(sig: &ed25519_dalek::Signature) -> Vec<u8> {
    let mut w = Writer::new();
    w.put_string(HOSTKEY_NAME.as_bytes())
        .put_string(&sig.to_bytes());
    w.into_bytes()
}

/// Calcula el hash de intercambio H (RFC 4253 §8, adaptado a curve25519 por
/// RFC 5656 §4 / RFC 8731):
///
/// `H = SHA256( string V_C || string V_S || string I_C || string I_S ||
///              string K_S || string Q_C || string Q_S || mpint K )`
///
/// Cada término se serializa según RFC 4251 con `proto::Writer` (ya probado); el
/// hash lo hace `sha2`.
pub fn exchange_hash(
    v_c: &[u8], // ident del cliente (sin CRLF)
    v_s: &[u8], // ident del servidor (sin CRLF)
    i_c: &[u8], // payload íntegro del KEXINIT del cliente (empieza por el byte 20)
    i_s: &[u8], // payload íntegro del KEXINIT del servidor
    k_s: &[u8], // blob de clave de host pública (ssh-ed25519)
    q_c: &[u8], // pública efímera del cliente (32 bytes)
    q_s: &[u8], // pública efímera del servidor (32 bytes)
    k: &[u8],   // secreto compartido (big-endian, se codifica como mpint)
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
    Sha256::digest(w.as_slice()).into()
}

/// Codifica el secreto compartido K como `mpint` (RFC 4251) para el KDF.
///
/// En el KDF (RFC 4253 §7.2) K entra con su codificación `mpint` COMPLETA
/// (longitud de 4 bytes incluida), igual que en el hash de intercambio.
pub fn k_as_mpint(k_raw: &[u8]) -> Vec<u8> {
    let mut w = Writer::new();
    w.put_mpint_uint(k_raw);
    w.into_bytes()
}

/// Deriva material de clave de sesión (RFC 4253 §7.2):
///
/// `K1 = HASH(K || H || "X" || session_id)`
/// `K2 = HASH(K || H || K1)`, `K3 = HASH(K || H || K1 || K2)` ...
/// hasta rellenar `out`. `x` es la letra ASCII A..F que identifica la clave.
///
/// `k_mpint` es K ya codificado como `mpint` (usar [`k_as_mpint`]).
pub fn derive_key(k_mpint: &[u8], h: &[u8; 32], x: u8, session_id: &[u8; 32], out: &mut [u8]) {
    // Primer bloque: K1 = HASH(K || H || x || session_id).
    let mut hasher = Sha256::new();
    hasher.update(k_mpint);
    hasher.update(h);
    hasher.update([x]);
    hasher.update(session_id);
    let mut prev: [u8; 32] = hasher.finalize().into();

    let mut filled = 0usize;
    loop {
        let take = core::cmp::min(prev.len(), out.len() - filled);
        out[filled..filled + take].copy_from_slice(&prev[..take]);
        filled += take;
        if filled >= out.len() {
            break;
        }
        // Bloque siguiente: HASH(K || H || <todo lo derivado hasta ahora>).
        // Como cada bloque es de 32 bytes, encadenar el último basta cuando
        // `out.len() <= 64` (nuestro caso: 64 bytes por clave). Para longitudes
        // mayores habría que concatenar TODOS los bloques previos; aquí basta con
        // el último porque nunca pedimos más de 2 bloques.
        let mut hasher = Sha256::new();
        hasher.update(k_mpint);
        hasher.update(h);
        hasher.update(prev);
        prev = hasher.finalize().into();
    }
}

/// Resultado de un kex correcto del lado servidor. Todo lo que `Connection`
/// necesita para armar el KEX_ECDH_REPLY y derivar las claves.
pub struct KexResult {
    /// Hash de intercambio H (y session_id en el primer kex). 32 bytes.
    pub h: [u8; 32],
    /// Secreto compartido K ya codificado como `mpint` (para el KDF).
    pub k_mpint: Vec<u8>,
    /// Pública efímera del servidor Q_S (32 bytes, va como `string` en el REPLY).
    pub q_s: [u8; 32],
    /// Blob de clave de host `K_S` (`string`).
    pub k_s: Vec<u8>,
    /// Blob de firma de H (`string "ssh-ed25519" || string sig`).
    pub sig_blob: Vec<u8>,
}

/// Ejecuta el kex del lado servidor: valida Q_C, genera el par efímero, calcula K
/// y H, y firma H con la clave de host ed25519.
///
/// El llamador (`Connection`) provee la transcripción (V_*/I_*), la clave de host
/// y el RNG hardware. Devuelve todo lo necesario para el REPLY y el KDF.
pub fn run_server(
    rng: &mut HwRng,
    host_sk: &SigningKey,
    v_c: &[u8],
    v_s: &[u8],
    i_c: &[u8],
    i_s: &[u8],
    q_c: &[u8],
) -> KResult<KexResult> {
    // Q_C debe ser exactamente una coordenada de 32 bytes.
    if q_c.len() != X25519_POINT_LEN {
        return Err(KError::InvalidArgument);
    }
    let mut q_c_arr = [0u8; 32];
    q_c_arr.copy_from_slice(q_c);
    // Rechazar la pública all-zeros (clave de contribución nula / orden bajo).
    if q_c_arr.iter().all(|&b| b == 0) {
        return Err(KError::InvalidArgument);
    }

    // 1) Par efímero del servidor con el TRNG hardware.
    let secret = EphemeralSecret::random_from_rng(&mut *rng);
    let q_s_pub = PublicKey::from(&secret);
    let q_s = *q_s_pub.as_bytes();

    // 2) ECDH: consume el secreto (se borra con zeroize).
    let their = PublicKey::from(q_c_arr);
    let shared = secret.diffie_hellman(&their);
    let k_raw = shared.as_bytes();
    // Rechazar el secreto de orden bajo (all-zeros): aborta el kex.
    if k_raw.iter().all(|&b| b == 0) {
        return Err(KError::InvalidArgument);
    }

    // 3) Blob de clave de host y hash de intercambio.
    let vk: VerifyingKey = host_sk.verifying_key();
    let k_s = host_key_blob(&vk);
    let h = exchange_hash(v_c, v_s, i_c, i_s, &k_s, q_c, &q_s, k_raw);

    // 4) Firma de H con la clave de host y empaquetado del blob de firma.
    let sig = host_sk.sign(&h);
    let sig_blob = signature_blob(&sig);

    Ok(KexResult {
        h,
        k_mpint: k_as_mpint(k_raw),
        q_s,
        k_s,
        sig_blob,
    })
}
