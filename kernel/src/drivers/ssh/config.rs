//! Configuración de autenticación del servidor SSH (credenciales de DESARROLLO).
// COMPILE-STATUS: borrador
//!
//! =========================  AVISO DE SEGURIDAD  =============================
//! Este módulo contiene credenciales de DESARROLLO en claro para el MVP. NO son
//! secretos de producción y DEBEN sustituirse antes de cualquier despliegue:
//!   - `DEV_USER` / `DEV_PASSWORD`: cuenta fija para probar `password`-auth.
//!   - `AUTHORIZED_KEYS`: claves públicas ssh-ed25519 aceptadas (publickey-auth).
//!
//! En un sistema real estas credenciales vendrían del FS (LittleFS: `/etc/passwd`
//! con hash de contraseña, `~/.ssh/authorized_keys`), NO del binario. Se dejan
//! aquí, explícitas y con placeholder evidente, para no "esconder" un secreto en
//! el código (mejor un placeholder ruidoso que una credencial silenciosa).
//! ===========================================================================
#![allow(dead_code)]

use crate::prelude::*;

/// Usuario de desarrollo aceptado por `password`-auth.
pub const DEV_USER: &str = "youareme";

/// Contraseña de DESARROLLO. PLACEHOLDER: cámbiala/quítala antes de producción.
/// (Comparada en tiempo constante en `auth::check_password`.)
pub const DEV_PASSWORD: &[u8] = b"851963Y@#";

/// Semilla FIJA de la clave de host `ssh-ed25519` (32 bytes). PLACEHOLDER de DEV.
///
/// Fijarla hace que la HUELLA del servidor sea ESTABLE entre reinicios y
/// flasheos, para no tener que borrar `known_hosts` (`ssh-keygen -R`) en cada
/// conexión. En producción, la clave de host se GENERA una vez con el TRNG y se
/// PERSISTE en el FS (LittleFS); esto es el sustituto mientras no haya FS.
/// Cámbiala por 32 bytes aleatorios tuyos si quieres una huella propia.
pub const HOST_KEY_SEED: [u8; 32] = [
    0x45, 0x73, 0x70, 0x72, 0x65, 0x73, 0x73, 0x6f, // Espresso
    0x4f, 0x53, 0x2d, 0x64, 0x65, 0x76, 0x2d, 0x68, // OS-dev-h
    0x6f, 0x73, 0x74, 0x6b, 0x65, 0x79, 0x2d, 0x73, // ostkey-s
    0x65, 0x65, 0x64, 0x2d, 0x76, 0x31, 0x21, 0x21, // eed-v1!!
];

/// Clave(s) pública(s) ssh-ed25519 autorizadas para `publickey`-auth.
///
/// Cada entrada es el BLOB de cable de la clave (RFC 8709):
///   `string "ssh-ed25519" || string pub(32)`  (lo que va tras `ssh-ed25519 ` en
///   un `authorized_keys`, decodificado de base64).
///
/// Vacío por defecto en el MVP: rellenar con la(s) clave(s) del operador. Cuando
/// esté el FS, esto se cargará de `authorized_keys` en vez de estar embebido.
pub fn authorized_key_blobs() -> Vec<Vec<u8>> {
    // TODO(fase-fs): cargar de `/etc/ssh/authorized_keys` (LittleFS).
    Vec::new()
}
