#![allow(dead_code)]

use super::config;
use super::proto::{Reader, Writer, SSH_MSG_USERAUTH_REQUEST};
use crate::prelude::*;

use ed25519_dalek::{Signature, VerifyingKey};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

pub const PUBLICKEY_ALGO: &str = "ssh-ed25519";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AuthResult {
    Success,
    Failure,
    PartialSuccess,
}

pub enum AuthMethod<'a> {
    None,
    Password(&'a [u8]),

    PublicKey {
        algo: &'a [u8],
        key_blob: &'a [u8],
        signature: Option<&'a [u8]>,
    },
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.ct_eq(b).unwrap_u8() == 1
}

fn hex_val(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    if b.is_empty() || b.len() % 2 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(b.len() / 2);
    let mut i = 0;
    while i < b.len() {
        let hi = hex_val(b[i])?;
        let lo = hex_val(b[i + 1])?;
        out.push((hi << 4) | lo);
        i += 2;
    }
    Some(out)
}

/// Checks `password` against a stored `/etc/passwd` field. Understands two shapes:
/// `$s5$<salt-hex>$<hash-hex>` (SHA-256(salt||password), written by `passwd(1)` via
/// `drivers::passwd` since SP2-R6) and bare plaintext (whatever `/etc/passwd` held
/// before that -- kept working on READ so an existing file is not silently locked out;
/// running `passwd` again replaces it with the salted form).
fn verify_stored(field: &str, password: &[u8]) -> bool {
    if let Some(rest) = field.strip_prefix("$s5$") {
        let mut parts = rest.splitn(2, '$');
        let (salt_hex, hash_hex) = match (parts.next(), parts.next()) {
            (Some(s), Some(h)) => (s, h),
            _ => return false,
        };
        let (salt, expected) = match (hex_decode(salt_hex), hex_decode(hash_hex)) {
            (Some(s), Some(h)) if h.len() == 32 => (s, h),
            _ => return false,
        };

        let mut hasher = Sha256::new();
        hasher.update(&salt);
        hasher.update(password);
        let got = hasher.finalize();
        return ct_eq(&got, &expected);
    }

    ct_eq(password, field.as_bytes())
}

pub fn check_password(user: &str, password: &[u8]) -> AuthResult {
    if let Ok(inode) = crate::vfs::mount::resolve("/etc/passwd") {
        let mut buf = [0u8; 512];
        if let Ok(n) = inode.read_at(0, &mut buf) {
            if let Ok(content) = core::str::from_utf8(&buf[..n]) {
                for line in content.lines() {
                    let line = line.trim();
                    if line.is_empty() || line.starts_with('#') {
                        continue;
                    }
                    let mut parts = line.split(':');
                    if let (Some(u), Some(field)) = (parts.next(), parts.next()) {
                        let u_trimmed = u.trim();
                        let f_trimmed = field.trim();
                        if ct_eq(user.as_bytes(), u_trimmed.as_bytes()) {
                            return if verify_stored(f_trimmed, password) {
                                AuthResult::Success
                            } else {
                                AuthResult::Failure
                            };
                        }
                    }
                }
            }
        }
    }

    let user_ok = ct_eq(user.as_bytes(), config::DEV_USER.as_bytes());
    let pass_ok = ct_eq(password, config::DEV_PASSWORD);

    if user_ok && pass_ok {
        AuthResult::Success
    } else {
        AuthResult::Failure
    }
}

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

pub fn is_authorized_key(_user: &str, key_blob: &[u8]) -> bool {
    let mut authorized = false;
    for k in config::authorized_key_blobs() {
        authorized |= ct_eq(&k, key_blob);
    }
    authorized
}

fn parse_ed25519_key(key_blob: &[u8]) -> KResult<[u8; 32]> {
    let mut r = Reader::new(key_blob);
    if r.get_string()? != PUBLICKEY_ALGO.as_bytes() {
        return Err(KError::InvalidArgument);
    }
    let pk = r.get_string()?;
    pk.try_into().map_err(|_| KError::InvalidArgument)
}

fn parse_ed25519_sig(sig_blob: &[u8]) -> KResult<[u8; 64]> {
    let mut r = Reader::new(sig_blob);
    if r.get_string()? != PUBLICKEY_ALGO.as_bytes() {
        return Err(KError::InvalidArgument);
    }
    let sig = r.get_string()?;
    sig.try_into().map_err(|_| KError::InvalidArgument)
}

pub fn probe_publickey(user: &str, algo: &[u8], key_blob: &[u8]) -> bool {
    if algo != PUBLICKEY_ALGO.as_bytes() {
        return false;
    }
    if parse_ed25519_key(key_blob).is_err() {
        return false;
    }
    is_authorized_key(user, key_blob)
}

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

    if !is_authorized_key(user, key_blob) {
        return AuthResult::Failure;
    }

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

    let msg = signed_blob(session_id, user, algo, key_blob);
    match vk.verify_strict(&msg, &sig) {
        Ok(()) => AuthResult::Success,
        Err(_) => AuthResult::Failure,
    }
}
