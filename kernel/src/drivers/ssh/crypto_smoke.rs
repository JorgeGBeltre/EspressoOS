//! Prueba de humo de la criptografía SSH (PASO 1 de-riesgo).
// COMPILE-STATUS: borrador
//!
//! NO forma parte del protocolo SSH. Su ÚNICA misión es EJERCITAR cada crate de
//! criptografía (`sha2`, `x25519-dalek`, `ed25519-dalek`, `chacha20`, `poly1305`,
//! `subtle`) para:
//!   1. Confirmar que compilan y enlazan en `xtensa-esp32s3-none-elf` / no_std.
//!   2. Fijar en el código las firmas reales que usarán `kex`/`crypt`/`auth`.
//!
//! Toda la entropía sale del TRNG vía `super::crypto_rng::HwRng`. NO se usa
//! `getrandom`/`OsRng` (no existe backend xtensa). `smoke()` roba el periférico
//! RNG: es una verificación puntual, NO debe llamarse concurrentemente con la
//! radio en `net_task`.
#![allow(dead_code)]

use crate::prelude::*;

use super::crypto_rng::HwRng;
use rand_core::RngCore; // `fill_bytes` sobre HwRng

// --- SHA-256 (hash de intercambio H y KDF) ---------------------------------
use sha2::{Digest, Sha256};

// --- X25519 (ECDH efímero del kex) -----------------------------------------
use x25519_dalek::{EphemeralSecret, PublicKey};

// --- Ed25519 (host key: firma de H y verificación publickey) ---------------
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};

// --- ChaCha20 (cifrado de sesión, variante djb de openssh) -----------------
use chacha20::cipher::{KeyIvInit, StreamCipher, StreamCipherSeek};
use chacha20::ChaCha20Legacy;

// --- Poly1305 (etiqueta AEAD) ----------------------------------------------
use poly1305::universal_hash::{KeyInit, UniversalHash};
use poly1305::Poly1305;

// --- subtle (comparación en tiempo constante) ------------------------------
use subtle::ConstantTimeEq;

/// Ejercita todas las crates de cripto y devuelve `true` si cada primitiva pasa
/// su comprobación interna (firma verifica, tag coincide, ECDH simétrico).
///
/// Devuelve un `bool` observable para que el optimizador no elimine el cuerpo.
/// `#[inline(never)]` fuerza que quede un símbolo que enlaza cada crate.
#[inline(never)]
pub fn smoke() -> bool {
    // Fuente de entropía: TRNG del ESP32-S3 (radio activa asumida).
    // SAFETY: verificación puntual; robamos el singleton del periférico RNG.
    let rng = esp_hal::rng::Rng::new(unsafe { esp_hal::peripherals::RNG::steal() });
    let mut hw = HwRng::new(rng);

    let mut ok = true;

    // 1) SHA-256 one-shot: debe producir 32 bytes.
    let h: [u8; 32] = Sha256::digest(b"EspressoOS ssh smoke").into();
    ok &= h.iter().any(|&b| b != 0);

    // 2) X25519 ECDH efímero por AMBOS lados con nuestro HwRng; el secreto
    //    compartido debe coincidir (propiedad de Diffie-Hellman).
    let sk_a = EphemeralSecret::random_from_rng(&mut hw);
    let pk_a = PublicKey::from(&sk_a);
    let sk_b = EphemeralSecret::random_from_rng(&mut hw);
    let pk_b = PublicKey::from(&sk_b);
    let shared_a = sk_a.diffie_hellman(&pk_b);
    let shared_b = sk_b.diffie_hellman(&pk_a);
    ok &= shared_a.as_bytes().ct_eq(shared_b.as_bytes()).unwrap_u8() == 1;

    // 3) Ed25519: semilla del TRNG -> firma de H -> verificación estricta.
    let mut seed = [0u8; 32];
    hw.fill_bytes(&mut seed);
    let signing = SigningKey::from_bytes(&seed);
    let verifying: VerifyingKey = signing.verifying_key();
    let sig: Signature = signing.sign(&h);
    // Round-trip por bytes (como en el cable RFC 8709) + verificación.
    let sig2 = Signature::from_bytes(&sig.to_bytes());
    ok &= verifying.verify_strict(&h, &sig2).is_ok();

    // 4) ChaCha20Legacy (nonce 8B) + Poly1305: cifrar y autenticar como en la
    //    construcción `chacha20-poly1305@openssh.com`.
    let mut key = [0u8; 32];
    hw.fill_bytes(&mut key);
    let nonce: [u8; 8] = 1u64.to_be_bytes();

    // Clave Poly1305 = primeros 32 bytes del keystream de ChaCha (contador 0).
    let mut c = ChaCha20Legacy::new_from_slices(&key, &nonce).unwrap();
    let mut poly_key = [0u8; 32];
    c.apply_keystream(&mut poly_key);
    // Payload cifrado desde el bloque 1 (offset de byte 64).
    c.seek(64u64);
    let mut buf = *b"remote shell payload";
    c.apply_keystream(&mut buf);

    // Etiqueta Poly1305 sobre el ciphertext.
    let mut mac = Poly1305::new_from_slice(&poly_key).unwrap();
    mac.update_padded(&buf);
    let tag = mac.finalize();

    // Recalcular la etiqueta y comparar en tiempo constante (subtle).
    let mut mac2 = Poly1305::new_from_slice(&poly_key).unwrap();
    mac2.update_padded(&buf);
    let tag2 = mac2.finalize();
    ok &= tag.as_slice().ct_eq(tag2.as_slice()).unwrap_u8() == 1;

    ok
}
