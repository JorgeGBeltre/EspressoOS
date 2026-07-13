//! Autenticación de usuario SSH (RFC 4252): `password` + `publickey`.
// COMPILE-STATUS: borrador (implementado, sin compilar contra HW)
//!
//! Métodos soportados en el MVP:
//!   - `password`: compara contra la credencial de desarrollo (`super::config`)
//!     en tiempo constante (`subtle`).
//!   - `publickey`: verifica una firma `ssh-ed25519` con `ed25519-dalek`
//!     (`verify_strict`) contra las claves autorizadas (`super::config`).
//!
//! La verificación de firma la hace la crate auditada; aquí sólo se arma el blob
//! que se firma (RFC 4252 §7) usando `proto::Writer` (ya probado) y se aplican las
//! comprobaciones de política (usuario/clave autorizados).
#![allow(dead_code)]

use super::config;
use super::proto::{Reader, Writer, SSH_MSG_USERAUTH_REQUEST};
use crate::prelude::*;

use ed25519_dalek::{Signature, VerifyingKey};
use subtle::ConstantTimeEq;

/// Nombre del algoritmo de clave pública soportado.
pub const PUBLICKEY_ALGO: &str = "ssh-ed25519";

/// Resultado de un intento de autenticación.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AuthResult {
    Success,
    Failure,
    PartialSuccess,
}

/// Método de autenticación de una petición USERAUTH_REQUEST.
pub enum AuthMethod<'a> {
    None,
    Password(&'a [u8]),
    /// (algoritmo, blob de clave pública, firma opcional).
    PublicKey {
        algo: &'a [u8],
        key_blob: &'a [u8],
        signature: Option<&'a [u8]>,
    },
}

/// Compara dos secretos en tiempo constante. Compara primero la longitud (fuga
/// aceptada: la longitud no es el secreto) y luego el contenido con `ct_eq`.
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.ct_eq(b).unwrap_u8() == 1
}

/// Verifica una contraseña contra la credencial de desarrollo, en tiempo
/// constante. La credencial vive en `super::config` (placeholder documentado);
/// en producción vendría del FS con hash, nunca en claro en el binario.
pub fn check_password(user: &str, password: &[u8]) -> AuthResult {
    // El usuario no es secreto: comparación normal. La contraseña sí: `ct_eq`.
    let user_ok = user.as_bytes() == config::DEV_USER.as_bytes();
    let pass_ok = ct_eq(password, config::DEV_PASSWORD);
    // Evaluamos ambos SIEMPRE (sin cortocircuito por timing) antes de decidir.
    if user_ok && pass_ok {
        AuthResult::Success
    } else {
        AuthResult::Failure
    }
}

/// Datos que el cliente firma en publickey-auth (RFC 4252 §7):
///   string session_id; byte SSH_MSG_USERAUTH_REQUEST; string user;
///   string "ssh-connection"; string "publickey"; boolean TRUE; string algo; string key.
pub fn signed_blob(session_id: &[u8], user: &str, algo: &[u8], key_blob: &[u8]) -> Vec<u8> {
    let mut w = Writer::new();
    w.put_string(session_id)
        .put_u8(SSH_MSG_USERAUTH_REQUEST)
        .put_string(user.as_bytes())
        .put_string(b"ssh-connection")
        .put_string(b"publickey")
        .put_bool(true)
        .put_string(algo)
        .put_string(key_blob);
    w.into_bytes()
}

/// ¿Está `key_blob` en la lista de claves autorizadas del usuario?
///
/// En el MVP la lista es global (`super::config`), no por-usuario. Comparación en
/// tiempo constante para no filtrar qué clave coincide por timing.
pub fn is_authorized_key(_user: &str, key_blob: &[u8]) -> bool {
    let mut authorized = false;
    for k in config::authorized_key_blobs() {
        // No cortocircuitar: recorrer todas para tiempo ~constante en el número de
        // claves (el contenido ya se compara con ct_eq).
        authorized |= ct_eq(&k, key_blob);
    }
    authorized
}

/// Extrae la clave pública ed25519 (32 bytes) de un blob `ssh-ed25519`:
///   `string "ssh-ed25519" || string pub(32)`.
fn parse_ed25519_key(key_blob: &[u8]) -> KResult<[u8; 32]> {
    let mut r = Reader::new(key_blob);
    if r.get_string()? != PUBLICKEY_ALGO.as_bytes() {
        return Err(KError::InvalidArgument);
    }
    let pk = r.get_string()?;
    pk.try_into().map_err(|_| KError::InvalidArgument)
}

/// Extrae la firma cruda (64 bytes) de un blob de firma `ssh-ed25519`:
///   `string "ssh-ed25519" || string sig(64)`.
fn parse_ed25519_sig(sig_blob: &[u8]) -> KResult<[u8; 64]> {
    let mut r = Reader::new(sig_blob);
    if r.get_string()? != PUBLICKEY_ALGO.as_bytes() {
        return Err(KError::InvalidArgument);
    }
    let sig = r.get_string()?;
    sig.try_into().map_err(|_| KError::InvalidArgument)
}

/// Verifica la fase de SONDEO de publickey (sin firma): comprueba sólo que la
/// clave esté autorizada y sea un `ssh-ed25519` válido. Si pasa, el servidor
/// responde `SSH_MSG_USERAUTH_PK_OK`.
pub fn probe_publickey(user: &str, algo: &[u8], key_blob: &[u8]) -> bool {
    if algo != PUBLICKEY_ALGO.as_bytes() {
        return false;
    }
    if parse_ed25519_key(key_blob).is_err() {
        return false;
    }
    is_authorized_key(user, key_blob)
}

/// Verifica una firma ssh-ed25519 sobre `signed_blob` (fase de autenticación real,
/// `boolean TRUE`). Comprueba autorización de la clave y validez de la firma con
/// `verify_strict` (rechaza claves débiles / small-order).
pub fn verify_publickey(
    user: &str,
    algo: &[u8],
    key_blob: &[u8],
    signature: &[u8],
    session_id: &[u8],
) -> AuthResult {
    if algo != PUBLICKEY_ALGO.as_bytes() {
        return AuthResult::Failure;
    }
    // 1) La clave debe estar autorizada ANTES de gastar CPU verificando la firma.
    if !is_authorized_key(user, key_blob) {
        return AuthResult::Failure;
    }
    // 2) Parsear clave y firma del formato de cable.
    let pk = match parse_ed25519_key(key_blob) {
        Ok(p) => p,
        Err(_) => return AuthResult::Failure,
    };
    let sig_bytes = match parse_ed25519_sig(signature) {
        Ok(s) => s,
        Err(_) => return AuthResult::Failure,
    };
    let vk = match VerifyingKey::from_bytes(&pk) {
        Ok(v) => v,
        Err(_) => return AuthResult::Failure,
    };
    let sig = Signature::from_bytes(&sig_bytes);

    // 3) El mensaje firmado se reconstruye exactamente (RFC 4252 §7).
    let msg = signed_blob(session_id, user, algo, key_blob);
    match vk.verify_strict(&msg, &sig) {
        Ok(()) => AuthResult::Success,
        Err(_) => AuthResult::Failure,
    }
}
