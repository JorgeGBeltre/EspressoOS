#!/usr/bin/env python3
"""Tests de la capa de wire-format SSH (espeja kernel/src/drivers/ssh/proto.rs).

Verifica, sin toolchain de Rust, la LÓGICA PURA del protocolo: los codecs de tipos
RFC 4251 (uint32/string/mpint/name-list) y el binary packet protocol (RFC 4253 §6:
longitudes, padding mínimo 4, alineación a bloque). NO cubre criptografía (esa se
delega a crates auditadas y se valida por interoperabilidad en hardware).

Ejecutar:  python tools/tests/ssh_proto_tests.py
"""
from __future__ import annotations

import struct
import unittest

MAX_PACKET = 35_000
MIN_PADDING = 4
MIN_BLOCK = 8


# --- Codecs de tipos RFC 4251 (espejo de proto::Writer/Reader) ---------------

def put_u32(v: int) -> bytes:
    return struct.pack(">I", v)


def put_string(s: bytes) -> bytes:
    return put_u32(len(s)) + s


def put_name_list(names: list[str]) -> bytes:
    return put_string(",".join(names).encode())


def put_mpint_uint(be: bytes) -> bytes:
    # Quitar ceros a la izquierda.
    i = 0
    while i < len(be) and be[i] == 0:
        i += 1
    t = be[i:]
    if not t:
        return put_u32(0)
    if t[0] & 0x80:
        return put_u32(len(t) + 1) + b"\x00" + t
    return put_string(t)


class Reader:
    def __init__(self, buf: bytes):
        self.buf = buf
        self.pos = 0

    def take(self, n: int) -> bytes:
        if len(self.buf) - self.pos < n:
            raise ValueError("truncado")
        s = self.buf[self.pos:self.pos + n]
        self.pos += n
        return s

    def get_u32(self) -> int:
        return struct.unpack(">I", self.take(4))[0]

    def get_string(self) -> bytes:
        n = self.get_u32()
        if n > MAX_PACKET:
            raise ValueError("string enorme")
        return self.take(n)

    def get_name_list(self) -> list[str]:
        t = self.get_string().decode()
        return t.split(",") if t else []


# --- Binary packet protocol (espejo de proto::frame_packet/parse_packet) -----

def frame_packet(payload: bytes, block: int, pad_fill: int) -> bytes:
    block = max(block, MIN_BLOCK)
    base = 1 + len(payload)
    pad = block - ((4 + base) % block)
    if pad < MIN_PADDING:
        pad += block
    packet_length = base + pad
    return put_u32(packet_length) + bytes([pad]) + payload + bytes([pad_fill]) * pad


def parse_packet(buf: bytes):
    if len(buf) < 5:
        raise ValueError("corto")
    packet_length = struct.unpack(">I", buf[0:4])[0]
    if packet_length < 1 + MIN_PADDING or packet_length > MAX_PACKET:
        raise ValueError("packet_length inválido")
    total = 4 + packet_length
    if len(buf) < total:
        raise ValueError("incompleto")
    pad_len = buf[4]
    if pad_len < MIN_PADDING or pad_len + 1 > packet_length:
        raise ValueError("padding inválido")
    payload_len = packet_length - 1 - pad_len
    return buf[5:5 + payload_len], total


# --- Tests -------------------------------------------------------------------

class TestTypes(unittest.TestCase):
    def test_u32_roundtrip(self):
        for v in (0, 1, 255, 256, 0xDEADBEEF, 0xFFFFFFFF):
            self.assertEqual(Reader(put_u32(v)).get_u32(), v)

    def test_string_roundtrip(self):
        for s in (b"", b"a", b"hola mundo", bytes(range(256))):
            self.assertEqual(Reader(put_string(s)).get_string(), s)

    def test_string_prefijo_longitud(self):
        self.assertEqual(put_string(b"abc"), b"\x00\x00\x00\x03abc")

    def test_name_list_roundtrip(self):
        for names in ([], ["ssh-ed25519"], ["curve25519-sha256", "ecdh-sha2-nistp256"]):
            self.assertEqual(Reader(put_name_list(names)).get_name_list(), names)

    def test_name_list_vacia_es_string_vacio(self):
        self.assertEqual(put_name_list([]), b"\x00\x00\x00\x00")

    def test_reader_truncado_lanza(self):
        with self.assertRaises(ValueError):
            Reader(b"\x00\x00").get_u32()
        with self.assertRaises(ValueError):
            Reader(b"\x00\x00\x00\x05ab").get_string()


class TestMpint(unittest.TestCase):
    # Vectores del RFC 4251 §5 y casos límite del bit alto.
    def test_cero_es_string_vacio(self):
        self.assertEqual(put_mpint_uint(b"\x00\x00"), b"\x00\x00\x00\x00")
        self.assertEqual(put_mpint_uint(b""), b"\x00\x00\x00\x00")

    def test_positivo_pequeno(self):
        self.assertEqual(put_mpint_uint(b"\x09"), b"\x00\x00\x00\x01\x09")

    def test_bit_alto_antepone_cero(self):
        self.assertEqual(put_mpint_uint(b"\x80"), b"\x00\x00\x00\x02\x00\x80")

    def test_quita_ceros_y_luego_bit_alto(self):
        self.assertEqual(put_mpint_uint(b"\x00\x00\x80\x00"),
                         b"\x00\x00\x00\x03\x00\x80\x00")

    def test_valor_multibyte_normal(self):
        self.assertEqual(put_mpint_uint(b"\x12\x34"), b"\x00\x00\x00\x02\x12\x34")

    def test_vector_rfc_0x9a378f9b2e332a7(self):
        be = bytes.fromhex("09a378f9b2e332a7")
        # bit alto de 0x09 no está puesto -> string directo, 8 bytes.
        self.assertEqual(put_mpint_uint(be), b"\x00\x00\x00\x08" + be)


class TestBinaryPacket(unittest.TestCase):
    def test_roundtrip_varias_longitudes(self):
        for n in range(0, 300):
            payload = bytes((i * 7) & 0xFF for i in range(n))
            pkt = frame_packet(payload, block=8, pad_fill=0)
            out, consumed = parse_packet(pkt)
            self.assertEqual(out, payload)
            self.assertEqual(consumed, len(pkt))

    def test_alineacion_a_bloque(self):
        for block in (8, 16):
            for n in range(0, 100):
                pkt = frame_packet(bytes(n), block=block, pad_fill=0)
                self.assertEqual(len(pkt) % block, 0, f"n={n} block={block}")

    def test_padding_minimo_4(self):
        for n in range(0, 100):
            pkt = frame_packet(bytes(n), block=8, pad_fill=0)
            pad_len = pkt[4]
            self.assertGreaterEqual(pad_len, MIN_PADDING)

    def test_packet_incompleto_lanza(self):
        pkt = frame_packet(b"hola", block=8, pad_fill=0)
        with self.assertRaises(ValueError):
            parse_packet(pkt[:-1])  # falta el último byte

    def test_padding_invalido_lanza(self):
        pkt = bytearray(frame_packet(b"hola", block=8, pad_fill=0))
        pkt[4] = 1  # padding_length < 4
        with self.assertRaises(ValueError):
            parse_packet(bytes(pkt))

    def test_packet_length_fuera_de_rango(self):
        bad = put_u32(MAX_PACKET + 1) + bytes(10)
        with self.assertRaises(ValueError):
            parse_packet(bad)


if __name__ == "__main__":
    unittest.main(verbosity=2)
