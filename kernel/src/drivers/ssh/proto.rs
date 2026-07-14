#![allow(dead_code)]

use crate::prelude::*;

pub const IDENT: &str = "SSH-2.0-EspressoOS_0.1";

pub const MAX_PACKET: usize = 35_000;

pub const MIN_PADDING: usize = 4;

pub const MIN_BLOCK: usize = 8;

pub const SSH_MSG_DISCONNECT: u8 = 1;
pub const SSH_MSG_IGNORE: u8 = 2;
pub const SSH_MSG_UNIMPLEMENTED: u8 = 3;
pub const SSH_MSG_DEBUG: u8 = 4;
pub const SSH_MSG_SERVICE_REQUEST: u8 = 5;
pub const SSH_MSG_SERVICE_ACCEPT: u8 = 6;
pub const SSH_MSG_KEXINIT: u8 = 20;
pub const SSH_MSG_NEWKEYS: u8 = 21;
pub const SSH_MSG_KEX_ECDH_INIT: u8 = 30;
pub const SSH_MSG_KEX_ECDH_REPLY: u8 = 31;
pub const SSH_MSG_USERAUTH_REQUEST: u8 = 50;
pub const SSH_MSG_USERAUTH_FAILURE: u8 = 51;
pub const SSH_MSG_USERAUTH_SUCCESS: u8 = 52;
pub const SSH_MSG_USERAUTH_BANNER: u8 = 53;
pub const SSH_MSG_GLOBAL_REQUEST: u8 = 80;
pub const SSH_MSG_REQUEST_SUCCESS: u8 = 81;
pub const SSH_MSG_REQUEST_FAILURE: u8 = 82;
pub const SSH_MSG_CHANNEL_OPEN: u8 = 90;
pub const SSH_MSG_CHANNEL_OPEN_CONFIRMATION: u8 = 91;
pub const SSH_MSG_CHANNEL_OPEN_FAILURE: u8 = 92;
pub const SSH_MSG_CHANNEL_WINDOW_ADJUST: u8 = 93;
pub const SSH_MSG_CHANNEL_DATA: u8 = 94;
pub const SSH_MSG_CHANNEL_EXTENDED_DATA: u8 = 95;
pub const SSH_MSG_CHANNEL_EOF: u8 = 96;
pub const SSH_MSG_CHANNEL_CLOSE: u8 = 97;
pub const SSH_MSG_CHANNEL_REQUEST: u8 = 98;
pub const SSH_MSG_CHANNEL_SUCCESS: u8 = 99;
pub const SSH_MSG_CHANNEL_FAILURE: u8 = 100;

#[derive(Default)]
pub struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.buf
    }

    pub fn put_u8(&mut self, v: u8) -> &mut Self {
        self.buf.push(v);
        self
    }

    pub fn put_bool(&mut self, v: bool) -> &mut Self {
        self.buf.push(v as u8);
        self
    }

    pub fn put_u32(&mut self, v: u32) -> &mut Self {
        self.buf.extend_from_slice(&v.to_be_bytes());
        self
    }

    pub fn put_u64(&mut self, v: u64) -> &mut Self {
        self.buf.extend_from_slice(&v.to_be_bytes());
        self
    }

    pub fn put_string(&mut self, s: &[u8]) -> &mut Self {
        self.put_u32(s.len() as u32);
        self.buf.extend_from_slice(s);
        self
    }

    pub fn put_name_list(&mut self, names: &[&str]) -> &mut Self {
        let joined = names.join(",");
        self.put_string(joined.as_bytes())
    }

    pub fn put_mpint_uint(&mut self, be_bytes: &[u8]) -> &mut Self {
        let mut start = 0;
        while start < be_bytes.len() && be_bytes[start] == 0 {
            start += 1;
        }
        let trimmed = &be_bytes[start..];
        if trimmed.is_empty() {
            return self.put_u32(0);
        }
        if trimmed[0] & 0x80 != 0 {
            self.put_u32((trimmed.len() + 1) as u32);
            self.buf.push(0x00);
            self.buf.extend_from_slice(trimmed);
        } else {
            self.put_string(trimmed);
        }
        self
    }
}

pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    fn take(&mut self, n: usize) -> KResult<&'a [u8]> {
        if self.remaining() < n {
            return Err(KError::InvalidArgument);
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    pub fn get_u8(&mut self) -> KResult<u8> {
        Ok(self.take(1)?[0])
    }

    pub fn get_bool(&mut self) -> KResult<bool> {
        Ok(self.get_u8()? != 0)
    }

    pub fn get_u32(&mut self) -> KResult<u32> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn get_string(&mut self) -> KResult<&'a [u8]> {
        let len = self.get_u32()? as usize;
        if len > MAX_PACKET {
            return Err(KError::InvalidArgument);
        }
        self.take(len)
    }

    pub fn get_name_list(&mut self) -> KResult<Vec<String>> {
        let s = self.get_string()?;
        let text = core::str::from_utf8(s).map_err(|_| KError::InvalidArgument)?;
        if text.is_empty() {
            return Ok(Vec::new());
        }
        Ok(text.split(',').map(String::from).collect())
    }
}

pub fn frame_packet(payload: &[u8], block: usize, pad_fill: u8) -> Vec<u8> {
    let block = block.max(MIN_BLOCK);
    let base = 1 + payload.len();

    let mut pad = block - ((4 + base) % block);
    if pad < MIN_PADDING {
        pad += block;
    }
    let packet_length = (base + pad) as u32;
    let mut out = Vec::with_capacity(4 + packet_length as usize);
    out.extend_from_slice(&packet_length.to_be_bytes());
    out.push(pad as u8);
    out.extend_from_slice(payload);
    out.extend(core::iter::repeat(pad_fill).take(pad));
    out
}

pub fn frame_packet_aead(payload: &[u8], block: usize, pad_fill: u8) -> Vec<u8> {
    let block = block.max(MIN_BLOCK);
    let base = 1 + payload.len();

    let mut pad = block - (base % block);
    if pad < MIN_PADDING {
        pad += block;
    }
    let packet_length = (base + pad) as u32;
    let mut out = Vec::with_capacity(4 + packet_length as usize);
    out.extend_from_slice(&packet_length.to_be_bytes());
    out.push(pad as u8);
    out.extend_from_slice(payload);
    out.extend(core::iter::repeat(pad_fill).take(pad));
    out
}

pub fn parse_packet(buf: &[u8]) -> KResult<(Vec<u8>, usize)> {
    if buf.len() < 5 {
        return Err(KError::InvalidArgument);
    }
    let packet_length = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
    if packet_length < 1 + MIN_PADDING || packet_length > MAX_PACKET {
        return Err(KError::InvalidArgument);
    }
    let total = 4 + packet_length;
    if buf.len() < total {
        return Err(KError::InvalidArgument);
    }
    let pad_len = buf[4] as usize;
    if pad_len < MIN_PADDING || pad_len + 1 > packet_length {
        return Err(KError::InvalidArgument);
    }
    let payload_len = packet_length - 1 - pad_len;
    let payload = buf[5..5 + payload_len].to_vec();
    Ok((payload, total))
}
