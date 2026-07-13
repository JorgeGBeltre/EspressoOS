#![allow(dead_code)]

use super::crypto_rng::HwRng;
use super::proto::Writer;
use crate::prelude::*;

use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use sha2::{Digest, Sha256};
use x25519_dalek::{EphemeralSecret, PublicKey};

pub const KEX_NAME: &str = "curve25519-sha256";

pub const KEX_NAME_ALIAS: &str = "curve25519-sha256@libssh.org";

pub const HOSTKEY_NAME: &str = "ssh-ed25519";

pub const X25519_POINT_LEN: usize = 32;

pub fn host_key_blob(vk: &VerifyingKey) -> Vec<u8> {
    let mut w = Writer::new();
    w.put_string(HOSTKEY_NAME.as_bytes())
        .put_string(vk.as_bytes());
    w.into_bytes()
}

pub fn signature_blob(sig: &ed25519_dalek::Signature) -> Vec<u8> {
    let mut w = Writer::new();
    w.put_string(HOSTKEY_NAME.as_bytes())
        .put_string(&sig.to_bytes());
    w.into_bytes()
}

pub fn exchange_hash(
    v_c: &[u8],
    v_s: &[u8],
    i_c: &[u8],
    i_s: &[u8],
    k_s: &[u8],
    q_c: &[u8],
    q_s: &[u8],
    k: &[u8],
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

pub fn k_as_mpint(k_raw: &[u8]) -> Vec<u8> {
    let mut w = Writer::new();
    w.put_mpint_uint(k_raw);
    w.into_bytes()
}

pub fn derive_key(k_mpint: &[u8], h: &[u8; 32], x: u8, session_id: &[u8; 32], out: &mut [u8]) {

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

        let mut hasher = Sha256::new();
        hasher.update(k_mpint);
        hasher.update(h);
        hasher.update(prev);
        prev = hasher.finalize().into();
    }
}

pub struct KexResult {

    pub h: [u8; 32],

    pub k_mpint: Vec<u8>,

    pub q_s: [u8; 32],

    pub k_s: Vec<u8>,

    pub sig_blob: Vec<u8>,
}

pub fn run_server(
    rng: &mut HwRng,
    host_sk: &SigningKey,
    v_c: &[u8],
    v_s: &[u8],
    i_c: &[u8],
    i_s: &[u8],
    q_c: &[u8],
) -> KResult<KexResult> {

    if q_c.len() != X25519_POINT_LEN {
        return Err(KError::InvalidArgument);
    }
    let mut q_c_arr = [0u8; 32];
    q_c_arr.copy_from_slice(q_c);

    if q_c_arr.iter().all(|&b| b == 0) {
        return Err(KError::InvalidArgument);
    }

    let secret = EphemeralSecret::random_from_rng(&mut *rng);
    let q_s_pub = PublicKey::from(&secret);
    let q_s = *q_s_pub.as_bytes();

    let their = PublicKey::from(q_c_arr);
    let shared = secret.diffie_hellman(&their);
    let k_raw = shared.as_bytes();

    if k_raw.iter().all(|&b| b == 0) {
        return Err(KError::InvalidArgument);
    }

    let vk: VerifyingKey = host_sk.verifying_key();
    let k_s = host_key_blob(&vk);
    let h = exchange_hash(v_c, v_s, i_c, i_s, &k_s, q_c, &q_s, k_raw);

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
