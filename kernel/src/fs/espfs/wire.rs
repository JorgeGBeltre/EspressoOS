#![allow(dead_code)]

use crate::prelude::*;

pub const REC_MAGIC: u16 = 0xE5F5;
pub const SB_MAGIC: u32 = 0x4573_4653;
pub const VERSION: u32 = 1;

pub const HEADER_LEN: usize = 16;
pub const SB_LEN: usize = 20;

pub fn crc32_update(mut crc: u32, data: &[u8]) -> u32 {
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    crc
}

pub fn crc32_init() -> u32 {
    0xFFFF_FFFF
}

pub fn crc32_final(crc: u32) -> u32 {
    crc ^ 0xFFFF_FFFF
}

pub fn crc32(data: &[u8]) -> u32 {
    crc32_final(crc32_update(crc32_init(), data))
}

#[inline]
pub fn pad4(n: usize) -> usize {
    (n + 3) & !3
}

#[inline]
fn rd_u16(b: &[u8], o: usize) -> u16 {
    (b[o] as u16) | ((b[o + 1] as u16) << 8)
}

#[inline]
fn rd_u32(b: &[u8], o: usize) -> u32 {
    (b[o] as u32) | ((b[o + 1] as u32) << 8) | ((b[o + 2] as u32) << 16) | ((b[o + 3] as u32) << 24)
}

#[inline]
fn wr_u32(b: &mut [u8], o: usize, v: u32) {
    b[o] = v as u8;
    b[o + 1] = (v >> 8) as u8;
    b[o + 2] = (v >> 16) as u8;
    b[o + 3] = (v >> 24) as u8;
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RecType {
    MkFile = 1,
    MkDir = 2,
    Write = 3,
    Truncate = 4,
    Unlink = 5,
}

impl RecType {
    pub fn from_u8(v: u8) -> Option<RecType> {
        match v {
            1 => Some(RecType::MkFile),
            2 => Some(RecType::MkDir),
            3 => Some(RecType::Write),
            4 => Some(RecType::Truncate),
            5 => Some(RecType::Unlink),
            _ => None,
        }
    }
    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Header {
    pub rtype: RecType,
    pub seq: u32,
    pub plen: u32,
    pub crc: u32,
}

pub fn record_total_len(plen: usize) -> usize {
    HEADER_LEN + pad4(plen)
}

pub fn build_header(rtype: RecType, seq: u32, payload: &[u8]) -> [u8; HEADER_LEN] {
    let mut h = [0u8; HEADER_LEN];
    h[0] = REC_MAGIC as u8;
    h[1] = (REC_MAGIC >> 8) as u8;
    h[2] = rtype.as_u8();
    h[3] = 0;
    wr_u32(&mut h, 4, seq);
    wr_u32(&mut h, 8, payload.len() as u32);
    let mut crc = crc32_update(crc32_init(), &h[0..12]);
    crc = crc32_final(crc32_update(crc, payload));
    wr_u32(&mut h, 12, crc);
    h
}

pub fn encode_record(rtype: RecType, seq: u32, payload: &[u8]) -> Vec<u8> {
    let h = build_header(rtype, seq, payload);
    let total = record_total_len(payload.len());
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&h);
    out.extend_from_slice(payload);
    out.resize(total, 0);
    out
}

pub fn parse_header(buf: &[u8]) -> Option<Header> {
    if buf.len() < HEADER_LEN {
        return None;
    }
    if rd_u16(buf, 0) != REC_MAGIC {
        return None;
    }
    let rtype = RecType::from_u8(buf[2])?;
    Some(Header {
        rtype,
        seq: rd_u32(buf, 4),
        plen: rd_u32(buf, 8),
        crc: rd_u32(buf, 12),
    })
}

pub fn verify_crc(header16: &[u8], payload: &[u8], expected: u32) -> bool {
    let mut crc = crc32_update(crc32_init(), &header16[0..12]);
    crc = crc32_final(crc32_update(crc, payload));
    crc == expected
}

pub fn enc_mk(ino: u32, parent: u32, name: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(8 + name.len());
    v.extend_from_slice(&ino.to_le_bytes());
    v.extend_from_slice(&parent.to_le_bytes());
    v.extend_from_slice(name);
    v
}

pub fn dec_mk(p: &[u8]) -> Option<(u32, u32, &[u8])> {
    if p.len() < 8 {
        return None;
    }
    Some((rd_u32(p, 0), rd_u32(p, 4), &p[8..]))
}

pub fn enc_write(ino: u32, offset: u32, data: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(8 + data.len());
    v.extend_from_slice(&ino.to_le_bytes());
    v.extend_from_slice(&offset.to_le_bytes());
    v.extend_from_slice(data);
    v
}

pub fn dec_write_head(p8: &[u8]) -> Option<(u32, u32)> {
    if p8.len() < 8 {
        return None;
    }
    Some((rd_u32(p8, 0), rd_u32(p8, 4)))
}

pub const WRITE_DATA_OFF: usize = 8;

pub fn enc_trunc(ino: u32, len: u32) -> Vec<u8> {
    let mut v = Vec::with_capacity(8);
    v.extend_from_slice(&ino.to_le_bytes());
    v.extend_from_slice(&len.to_le_bytes());
    v
}

pub fn dec_trunc(p: &[u8]) -> Option<(u32, u32)> {
    if p.len() < 8 {
        return None;
    }
    Some((rd_u32(p, 0), rd_u32(p, 4)))
}

pub fn enc_unlink(parent: u32, name: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(4 + name.len());
    v.extend_from_slice(&parent.to_le_bytes());
    v.extend_from_slice(name);
    v
}

pub fn dec_unlink(p: &[u8]) -> Option<(u32, &[u8])> {
    if p.len() < 4 {
        return None;
    }
    Some((rd_u32(p, 0), &p[4..]))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SuperBlock {
    pub generation: u32,
    pub active_half: u32,
}

pub fn encode_super(sb: SuperBlock) -> Vec<u8> {
    let mut b = [0u8; SB_LEN];
    wr_u32(&mut b, 0, SB_MAGIC);
    wr_u32(&mut b, 4, VERSION);
    wr_u32(&mut b, 8, sb.generation);
    wr_u32(&mut b, 12, sb.active_half);
    let crc = crc32(&b[0..16]);
    wr_u32(&mut b, 16, crc);
    b.to_vec()
}

pub fn decode_super(b: &[u8]) -> Option<SuperBlock> {
    if b.len() < SB_LEN {
        return None;
    }
    if rd_u32(b, 0) != SB_MAGIC || rd_u32(b, 4) != VERSION {
        return None;
    }
    if crc32(&b[0..16]) != rd_u32(b, 16) {
        return None;
    }
    Some(SuperBlock {
        generation: rd_u32(b, 8),
        active_half: rd_u32(b, 12),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_roundtrip() {
        let payload = enc_write(7, 100, b"hello world");
        let rec = encode_record(RecType::Write, 42, &payload);
        assert_eq!(rec.len() % 4, 0);
        let h = parse_header(&rec).unwrap();
        assert_eq!(h.rtype, RecType::Write);
        assert_eq!(h.seq, 42);
        assert_eq!(h.plen as usize, payload.len());
        assert!(verify_crc(&rec[0..16], &rec[16..16 + payload.len()], h.crc));
        let (ino, off) = dec_write_head(&rec[16..24]).unwrap();
        assert_eq!((ino, off), (7, 100));
    }

    #[test]
    fn torn_record_fails_crc() {
        let payload = enc_mk(3, 1, b"foo");
        let mut rec = encode_record(RecType::MkFile, 1, &payload);
        let h = parse_header(&rec).unwrap();
        let plen = h.plen as usize;
        rec[16] ^= 0xFF;
        assert!(!verify_crc(&rec[0..16], &rec[16..16 + plen], h.crc));
    }

    #[test]
    fn super_roundtrip() {
        let sb = SuperBlock {
            generation: 5,
            active_half: 1,
        };
        let enc = encode_super(sb);
        assert_eq!(decode_super(&enc), Some(sb));
        let mut bad = enc.clone();
        bad[8] ^= 0xFF;
        assert_eq!(decode_super(&bad), None);
    }
}
