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
pub const DEV_USER: &str = "root";

/// Contraseña de DESARROLLO. PLACEHOLDER: cámbiala/quítala antes de producción.
/// (Comparada en tiempo constante en `auth::check_password`.)
pub const DEV_PASSWORD: &[u8] = b"CHANGE_ME_dev_only";

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
