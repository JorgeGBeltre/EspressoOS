//! Base64 encode and decode (RFC 4648) for no_std.

use crate::prelude::*;

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Encodes binary data into base64 (without padding).
pub fn encode_nopad(data: &[u8]) -> String {
    let mut out = String::new();
    let mut chunks = data.chunks(3);
    for c in &mut chunks {
        let b0 = c[0] as u32;
        let b1 = *c.get(1).unwrap_or(&0) as u32;
        let b2 = *c.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((n >> 18) & 63) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 63) as usize] as char);
        if c.len() > 1 {
            out.push(ALPHABET[((n >> 6) & 63) as usize] as char);
        }
        if c.len() > 2 {
            out.push(ALPHABET[(n & 63) as usize] as char);
        }
    }
    out
}

/// Encodes binary data into standard base64 (with '=' padding).
pub fn encode(data: &[u8]) -> String {
    let mut s = encode_nopad(data);
    while s.len() % 4 != 0 {
        s.push('=');
    }
    s
}

/// Decodes standard or unpadded base64 string into bytes.
pub fn decode(s: &str) -> Option<Vec<u8>> {
    let clean = s.trim();
    if clean.is_empty() {
        return Some(Vec::new());
    }

    fn decode_char(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }

    let bytes = clean.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0u32;

    for &b in bytes {
        if b == b'=' {
            break;
        }
        if b.is_ascii_whitespace() {
            continue;
        }
        let val = decode_char(b)?;
        buf = (buf << 6) | (val as u32);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }

    Some(out)
}
