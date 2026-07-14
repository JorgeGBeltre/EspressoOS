#![allow(dead_code)]

use crate::prelude::*;

use chacha20::cipher::{KeyIvInit, StreamCipher, StreamCipherSeek};
use chacha20::ChaCha20Legacy;
use poly1305::universal_hash::{KeyInit, UniversalHash};
use poly1305::Poly1305;
use subtle::ConstantTimeEq;

pub const CIPHER_NAME: &str = "chacha20-poly1305@openssh.com";

pub const TAG_LEN: usize = 16;

pub const LEN_LEN: usize = 4;

#[derive(Clone)]
pub struct Aead {
    k1: [u8; 32],
    k2: [u8; 32],
}

impl Aead {
    pub fn new(key_material: &[u8; 64]) -> Self {
        let mut k2 = [0u8; 32];
        let mut k1 = [0u8; 32];
        k2.copy_from_slice(&key_material[..32]);
        k1.copy_from_slice(&key_material[32..]);
        Self { k1, k2 }
    }

    #[inline]
    fn nonce(seq: u32) -> [u8; 8] {
        (seq as u64).to_be_bytes()
    }

    pub fn seal(&self, framed: &[u8], seq: u32) -> KResult<Vec<u8>> {
        if framed.len() < 4 {
            return Err(KError::InvalidArgument);
        }
        let nonce = Self::nonce(seq);
        let (len_pt, rest_pt) = framed.split_at(4);

        let mut enc_len = [0u8; 4];
        enc_len.copy_from_slice(len_pt);
        let mut c1 = ChaCha20Legacy::new_from_slices(&self.k1, &nonce)
            .map_err(|_| KError::InvalidArgument)?;
        c1.apply_keystream(&mut enc_len);

        let mut c2 = ChaCha20Legacy::new_from_slices(&self.k2, &nonce)
            .map_err(|_| KError::InvalidArgument)?;
        let mut poly_key = [0u8; 32];
        c2.apply_keystream(&mut poly_key);
        c2.seek(64u64);
        let mut enc_rest = rest_pt.to_vec();
        c2.apply_keystream(&mut enc_rest);

        let mac = Poly1305::new_from_slice(&poly_key).map_err(|_| KError::InvalidArgument)?;
        let mut aad = Vec::with_capacity(4 + enc_rest.len());
        aad.extend_from_slice(&enc_len);
        aad.extend_from_slice(&enc_rest);

        let tag = mac.compute_unpadded(&aad);

        let mut out = Vec::with_capacity(4 + enc_rest.len() + TAG_LEN);
        out.extend_from_slice(&enc_len);
        out.extend_from_slice(&enc_rest);
        out.extend_from_slice(tag.as_slice());
        Ok(out)
    }

    pub fn open_length(&self, enc_len: &[u8; 4], seq: u32) -> KResult<u32> {
        let nonce = Self::nonce(seq);
        let mut buf = *enc_len;
        let mut c1 = ChaCha20Legacy::new_from_slices(&self.k1, &nonce)
            .map_err(|_| KError::InvalidArgument)?;
        c1.apply_keystream(&mut buf);
        Ok(u32::from_be_bytes(buf))
    }

    pub fn open(&self, record: &[u8], seq: u32) -> KResult<Vec<u8>> {
        if record.len() < 4 + TAG_LEN {
            return Err(KError::InvalidArgument);
        }
        let nonce = Self::nonce(seq);
        let (enc_len, tail) = record.split_at(4);
        let (enc_ct, tag_rx) = tail.split_at(tail.len() - TAG_LEN);

        let mut c2 = ChaCha20Legacy::new_from_slices(&self.k2, &nonce)
            .map_err(|_| KError::InvalidArgument)?;
        let mut poly_key = [0u8; 32];
        c2.apply_keystream(&mut poly_key);
        let mac = Poly1305::new_from_slice(&poly_key).map_err(|_| KError::InvalidArgument)?;
        let mut aad = Vec::with_capacity(4 + enc_ct.len());
        aad.extend_from_slice(enc_len);
        aad.extend_from_slice(enc_ct);

        let tag = mac.compute_unpadded(&aad);
        if tag.as_slice().ct_eq(tag_rx).unwrap_u8() != 1 {
            return Err(KError::InvalidArgument);
        }

        c2.seek(64u64);
        let mut pt = enc_ct.to_vec();
        c2.apply_keystream(&mut pt);
        if pt.is_empty() {
            return Err(KError::InvalidArgument);
        }
        let pad_len = pt[0] as usize;
        if 1 + pad_len > pt.len() {
            return Err(KError::InvalidArgument);
        }
        let payload = pt[1..pt.len() - pad_len].to_vec();
        Ok(payload)
    }
}
