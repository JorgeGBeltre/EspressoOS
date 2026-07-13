//! Autenticación de usuario SSH (ESQUELETO — RFC 4252).
// COMPILE-STATUS: borrador (sin compilar)
//!
//! Métodos soportados en el MVP:
//!   - `password`: compara contra una credencial del sistema (en tiempo constante).
//!   - `publickey`: verifica una firma `ssh-ed25519` con `ed25519-dalek` contra
//!     las claves autorizadas (equivalente a `authorized_keys`).
//!
//! La verificación de firma la hace la crate auditada; aquí sólo se arma el blob
//! que se firma (RFC 4252 §7) usando `proto::Writer` (ya probado).
#![allow(dead_code)]

use super::proto::Writer;
use crate::prelude::*;

// use ed25519_dalek::{Signature, VerifyingKey};  // (?) confirmar al compilar
// use subtle::ConstantTimeEq;

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

/// Verifica una contraseña contra la credencial configurada, en tiempo constante.
/// ESQUELETO: la credencial debe venir de config del sistema, nunca hardcodeada.
pub fn check_password(_user: &str, _password: &[u8]) -> AuthResult {
    // TODO(fase-red): comparar hash de contraseña con subtle::ConstantTimeEq.
    // NUNCA comparar con == (fuga por timing). NUNCA credenciales en el binario.
    AuthResult::Failure
}

/// Datos que el cliente firma en publickey-auth (RFC 4252 §7):
///   string session_id; byte SSH_MSG_USERAUTH_REQUEST; string user;
///   string "ssh-connection"; string "publickey"; boolean TRUE; string algo; string key.
/// Se reconstruye para verificar la firma. Usa el Writer ya probado.
pub fn signed_blob(
    session_id: &[u8],
    user: &str,
    algo: &[u8],
    key_blob: &[u8],
) -> Vec<u8> {
    let mut w = Writer::new();
    w.put_string(session_id)
        .put_u8(super::proto::SSH_MSG_USERAUTH_REQUEST)
        .put_string(user.as_bytes())
        .put_string(b"ssh-connection")
        .put_string(b"publickey")
        .put_bool(true)
        .put_string(algo)
        .put_string(key_blob);
    w.into_bytes()
}

/// Verifica una firma ssh-ed25519 sobre `signed_blob`. ESQUELETO.
pub fn verify_publickey(
    _user: &str,
    _key_blob: &[u8],
    _signature: &[u8],
    _session_id: &[u8],
) -> AuthResult {
    // TODO(fase-red):
    //  1. Comprobar que key_blob está en la lista de claves autorizadas del user.
    //  2. VerifyingKey::from_bytes(pk).verify(signed_blob(...), Signature::from(sig)).
    AuthResult::Failure
}
