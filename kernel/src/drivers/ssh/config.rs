#![allow(dead_code)]

use crate::prelude::*;
use super::base64;

pub const DEV_USER: &str = "youareme";

pub const DEV_PASSWORD: &[u8] = b"851963Y@#";

pub const HOST_KEY_SEED: [u8; 32] = [
    0x45, 0x73, 0x70, 0x72, 0x65, 0x73, 0x73, 0x6f, 0x4f, 0x53, 0x2d, 0x64, 0x65, 0x76, 0x2d, 0x68,
    0x6f, 0x73, 0x74, 0x6b, 0x65, 0x79, 0x2d, 0x73, 0x65, 0x65, 0x64, 0x2d, 0x76, 0x31, 0x21, 0x21,
];

/// Lee las claves públicas autorizadas desde `/etc/authorized_keys` en el VFS.
/// Soporta formato estándar OpenSSH (`ssh-ed25519 <base64-blob> [comentario]`).
pub fn authorized_key_blobs() -> Vec<Vec<u8>> {
    let mut keys = Vec::new();
    if let Ok(inode) = crate::vfs::mount::resolve("/etc/authorized_keys") {
        let mut buf = alloc::vec![0u8; 2048];
        if let Ok(n) = inode.read_at(0, &mut buf) {
            if let Ok(content) = core::str::from_utf8(&buf[..n]) {
                for line in content.lines() {
                    let line = line.trim();
                    if line.is_empty() || line.starts_with('#') {
                        continue;
                    }
                    let mut parts = line.split_whitespace();
                    let key_type = match parts.next() {
                        Some(t) => t,
                        None => continue,
                    };
                    if key_type != "ssh-ed25519" {
                        continue;
                    }
                    let b64_blob = match parts.next() {
                        Some(b) => b,
                        None => continue,
                    };
                    if let Some(blob) = base64::decode(b64_blob) {
                        // El blob SSH-2.0 de ssh-ed25519 tiene exactamente 51 bytes:
                        // 4 bytes len(11) + "ssh-ed25519" (11) + 4 bytes len(32) + 32-byte pubkey
                        if blob.len() == 51 && &blob[4..15] == b"ssh-ed25519" {
                            keys.push(blob);
                        }
                    }
                }
            }
        }
    }
    keys
}

