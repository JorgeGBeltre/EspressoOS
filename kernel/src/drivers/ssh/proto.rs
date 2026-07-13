//! Protocolo SSH: constantes, *binary packet protocol* y tipos de datos RFC 4251.
// COMPILE-STATUS: borrador (sin compilar)
//!
//! Esta es la ÚNICA capa del subsistema SSH que es lógica pura (no criptografía):
//! serialización/deserialización del formato de cable. Por eso se prueba de forma
//! aislada con `tools/tests/ssh_proto_tests.py`, que replica exactamente estas
//! reglas. La cripto vive en `kex`/`crypt`/`auth` y se delega a crates auditadas.
//!
//! Referencias: RFC 4251 §5 (tipos), RFC 4253 §6 (binary packet protocol).
#![allow(dead_code)]

use crate::prelude::*;

// ---------------------------------------------------------------------------
// Identificación de versión y límites.
// ---------------------------------------------------------------------------

/// Cadena de identificación del servidor (sin el CRLF, que se añade al enviar).
pub const IDENT: &str = "SSH-2.0-EspressoOS_0.1";

/// Tamaño máximo aceptado de `packet_length` (RFC 4253 recomienda >= 35000).
pub const MAX_PACKET: usize = 35_000;

/// Padding mínimo del binary packet protocol (RFC 4253 §6).
pub const MIN_PADDING: usize = 4;

/// Bloque mínimo de alineación cuando el cifrado es de flujo o aún no hay cifrado.
pub const MIN_BLOCK: usize = 8;

// ---------------------------------------------------------------------------
// Números de mensaje SSH (RFC 4253 §12, 4252 §6, 4254 §9, 5656).
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Escritor de tipos RFC 4251 sobre un Vec<u8>.
// ---------------------------------------------------------------------------

/// Acumula campos SSH en orden de cable (big-endian). Nunca falla en memoria
/// salvo OOM del allocator global.
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

    /// `byte`.
    pub fn put_u8(&mut self, v: u8) -> &mut Self {
        self.buf.push(v);
        self
    }

    /// `boolean` (0 o 1).
    pub fn put_bool(&mut self, v: bool) -> &mut Self {
        self.buf.push(v as u8);
        self
    }

    /// `uint32` big-endian.
    pub fn put_u32(&mut self, v: u32) -> &mut Self {
        self.buf.extend_from_slice(&v.to_be_bytes());
        self
    }

    /// `uint64` big-endian.
    pub fn put_u64(&mut self, v: u64) -> &mut Self {
        self.buf.extend_from_slice(&v.to_be_bytes());
        self
    }

    /// `string`: uint32 de longitud + bytes crudos.
    pub fn put_string(&mut self, s: &[u8]) -> &mut Self {
        self.put_u32(s.len() as u32);
        self.buf.extend_from_slice(s);
        self
    }

    /// `name-list`: lista separada por comas, codificada como `string`.
    pub fn put_name_list(&mut self, names: &[&str]) -> &mut Self {
        let joined = names.join(",");
        self.put_string(joined.as_bytes())
    }

    /// `mpint` para un entero **no negativo** dado por sus bytes big-endian.
    ///
    /// Reglas RFC 4251: se quitan los ceros a la izquierda; si el bit más alto del
    /// primer byte queda a 1, se antepone un `0x00` (para que no se interprete como
    /// negativo); el cero se codifica como `string` vacío. En SSH los mpint de DH y
    /// claves son siempre no negativos, así que no se maneja el caso negativo.
    pub fn put_mpint_uint(&mut self, be_bytes: &[u8]) -> &mut Self {
        // Saltar ceros a la izquierda.
        let mut start = 0;
        while start < be_bytes.len() && be_bytes[start] == 0 {
            start += 1;
        }
        let trimmed = &be_bytes[start..];
        if trimmed.is_empty() {
            return self.put_u32(0); // cero -> string vacío
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

// ---------------------------------------------------------------------------
// Lector de tipos RFC 4251 sobre un &[u8] con comprobación de límites.
// ---------------------------------------------------------------------------

/// Lee campos SSH en orden de cable. Todo método devuelve `Err(InvalidArgument)`
/// si el buffer se agota (paquete truncado/malformado): nunca hace panic.
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

    /// `string` como slice prestado (sin copiar).
    pub fn get_string(&mut self) -> KResult<&'a [u8]> {
        let len = self.get_u32()? as usize;
        if len > MAX_PACKET {
            return Err(KError::InvalidArgument);
        }
        self.take(len)
    }

    /// `name-list` → Vec de nombres.
    pub fn get_name_list(&mut self) -> KResult<Vec<String>> {
        let s = self.get_string()?;
        let text = core::str::from_utf8(s).map_err(|_| KError::InvalidArgument)?;
        if text.is_empty() {
            return Ok(Vec::new());
        }
        Ok(text.split(',').map(String::from).collect())
    }
}

// ---------------------------------------------------------------------------
// Binary packet protocol (RFC 4253 §6) — SIN la capa MAC/AEAD.
//
//   uint32   packet_length   = 1 + payload_len + padding_len
//   byte     padding_length  (>= 4)
//   byte[]   payload
//   byte[]   random padding
//
// La regla de alineación es: (4 + packet_length) % block == 0. Con cifrado AEAD
// (crypt.rs) esto se envuelve; aquí sólo se produce/consume el marco en claro.
// ---------------------------------------------------------------------------

/// Enmarca `payload` en un paquete binario, rellenando con `pad_fill`.
///
/// En producción el padding DEBE ser aleatorio (`crypt` inyecta bytes del RNG);
/// `pad_fill` se parametriza sólo para poder probar el marco de forma determinista.
pub fn frame_packet(payload: &[u8], block: usize, pad_fill: u8) -> Vec<u8> {
    let block = block.max(MIN_BLOCK);
    let base = 1 + payload.len(); // padding_length (1 byte) + payload
    // Padding para que (4 + base + pad) sea múltiplo de `block`, con pad >= 4.
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

/// Igual que [`frame_packet`] pero para el AEAD `chacha20-poly1305@openssh.com`:
/// el campo de longitud (4 bytes) se cifra APARTE y NO cuenta para el alineado,
/// así que es `packet_length` (= `1 + payload + padding`) el que debe ser múltiplo
/// de `block`, no `4 + packet_length`. (OpenSSH rechaza con "padding error" si
/// `packet_length % block != 0`.)
pub fn frame_packet_aead(payload: &[u8], block: usize, pad_fill: u8) -> Vec<u8> {
    let block = block.max(MIN_BLOCK);
    let base = 1 + payload.len(); // padding_length (1 byte) + payload
    // Padding para que `base + pad` (= packet_length) sea múltiplo de `block`, con
    // pad >= 4. NO se incluyen los 4 bytes de longitud (van cifrados aparte).
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

/// Extrae el `payload` de un paquete binario en claro.
///
/// Devuelve `(payload, bytes_consumidos)`. `Err(InvalidArgument)` si el buffer no
/// contiene aún el paquete completo o si viola las reglas de longitud/padding.
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
        return Err(KError::InvalidArgument); // faltan bytes; reintentar al recibir más
    }
    let pad_len = buf[4] as usize;
    if pad_len < MIN_PADDING || pad_len + 1 > packet_length {
        return Err(KError::InvalidArgument);
    }
    let payload_len = packet_length - 1 - pad_len;
    let payload = buf[5..5 + payload_len].to_vec();
    Ok((payload, total))
}
