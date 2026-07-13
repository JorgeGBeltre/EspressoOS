#![allow(dead_code)]

use crate::prelude::*;

use super::crypto_rng::HwRng;
use rand_core::RngCore;

use sha2::{Digest, Sha256};

use x25519_dalek::{EphemeralSecret, PublicKey};

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};

use chacha20::cipher::{KeyIvInit, StreamCipher, StreamCipherSeek};
use chacha20::ChaCha20Legacy;

use poly1305::universal_hash::{KeyInit, UniversalHash};
use poly1305::Poly1305;

use subtle::ConstantTimeEq;

#[inline(never)]
pub fn smoke() -> bool {

    let rng = esp_hal::rng::Rng::new(unsafe { esp_hal::peripherals::RNG::steal() });
    let mut hw = HwRng::new(rng);

    let mut ok = true;

    let h: [u8; 32] = Sha256::digest(b"EspressoOS ssh smoke").into();
    ok &= h.iter().any(|&b| b != 0);

    let sk_a = EphemeralSecret::random_from_rng(&mut hw);
    let pk_a = PublicKey::from(&sk_a);
    let sk_b = EphemeralSecret::random_from_rng(&mut hw);
    let pk_b = PublicKey::from(&sk_b);
    let shared_a = sk_a.diffie_hellman(&pk_b);
    let shared_b = sk_b.diffie_hellman(&pk_a);
    ok &= shared_a.as_bytes().ct_eq(shared_b.as_bytes()).unwrap_u8() == 1;

    let mut seed = [0u8; 32];
    hw.fill_bytes(&mut seed);
    let signing = SigningKey::from_bytes(&seed);
    let verifying: VerifyingKey = signing.verifying_key();
    let sig: Signature = signing.sign(&h);

    let sig2 = Signature::from_bytes(&sig.to_bytes());
    ok &= verifying.verify_strict(&h, &sig2).is_ok();

    let mut key = [0u8; 32];
    hw.fill_bytes(&mut key);
    let nonce: [u8; 8] = 1u64.to_be_bytes();

    let mut c = ChaCha20Legacy::new_from_slices(&key, &nonce).unwrap();
    let mut poly_key = [0u8; 32];
    c.apply_keystream(&mut poly_key);

    c.seek(64u64);
    let mut buf = *b"remote shell payload";
    c.apply_keystream(&mut buf);

    let mut mac = Poly1305::new_from_slice(&poly_key).unwrap();
    mac.update_padded(&buf);
    let tag = mac.finalize();

    let mut mac2 = Poly1305::new_from_slice(&poly_key).unwrap();
    mac2.update_padded(&buf);
    let tag2 = mac2.finalize();
    ok &= tag.as_slice().ct_eq(tag2.as_slice()).unwrap_u8() == 1;

    ok
}
