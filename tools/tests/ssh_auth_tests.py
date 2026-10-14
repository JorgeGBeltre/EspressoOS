#!/usr/bin/env python3
"""Tests de la capa de autenticación SSH, base64 y persistencia de host key NVS.

Espeja la lógica de:
- kernel/src/drivers/ssh/base64.rs (decode/encode)
- kernel/src/drivers/ssh/config.rs (authorized_key_blobs)
- kernel/src/drivers/ssh/host_key_store.rs (checksum y estructura ESHK)
"""
from __future__ import annotations

import base64
import struct
import unittest

MAGIC = b"ESHK"
RECORD_LEN = 64


def checksum(seed: bytes) -> int:
    s = 0x1234_5678
    for b in seed:
        s = (((s << 3) & 0xFFFF_FFFF) | (s >> (32 - 3))) ^ b
        s &= 0xFFFF_FFFF
    return s


def parse_authorized_keys(content: str) -> list[bytes]:
    keys = []
    for line in content.splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        parts = line.split()
        if len(parts) < 2:
            continue
        key_type, b64_blob = parts[0], parts[1]
        if key_type != "ssh-ed25519":
            continue
        try:
            blob = base64.b64decode(b64_blob)
        except Exception:
            continue
        # El blob SSH-2.0 de ssh-ed25519 tiene 51 bytes:
        # uint32 len(11) + "ssh-ed25519" + uint32 len(32) + 32-byte pubkey
        if len(blob) == 51 and blob[4:15] == b"ssh-ed25519":
            keys.append(blob)
    return keys


class SshAuthTests(unittest.TestCase):
    def test_nvs_host_key_checksum(self):
        seed = b"\x42" * 32
        cs1 = checksum(seed)
        cs2 = checksum(seed)
        self.assertEqual(cs1, cs2)
        self.assertNotEqual(cs1, 0)

        # Mutar un byte altera el checksum
        seed_mutated = bytearray(seed)
        seed_mutated[0] ^= 0x01
        self.assertNotEqual(checksum(seed), checksum(seed_mutated))

    def test_nvs_record_packing(self):
        seed = bytes(range(32))
        rec = bytearray(RECORD_LEN)
        rec[0:4] = MAGIC
        rec[4] = 1  # version
        rec[8:40] = seed
        cs = checksum(seed)
        rec[40:44] = struct.pack("<I", cs)

        self.assertEqual(rec[0:4], b"ESHK")
        self.assertEqual(rec[4], 1)
        self.assertEqual(rec[8:40], seed)
        unpacked_cs = struct.unpack("<I", rec[40:44])[0]
        self.assertEqual(cs, unpacked_cs)

    def test_parse_authorized_keys_valid(self):
        # Generar un blob OpenSSH ed25519 sintético válido (51 bytes)
        algo = b"ssh-ed25519"
        pubkey = b"\xaa" * 32
        blob = struct.pack(">I", len(algo)) + algo + struct.pack(">I", len(pubkey)) + pubkey
        self.assertEqual(len(blob), 51)
        b64 = base64.b64encode(blob).decode()

        content = f"""
# Clave del administrador
ssh-ed25519 {b64} admin@espressoos

# Línea vacía siguiente

"""
        keys = parse_authorized_keys(content)
        self.assertEqual(len(keys), 1)
        self.assertEqual(keys[0], blob)

    def test_parse_authorized_keys_ignores_rsa(self):
        rsa_line = "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQC3... user@host"
        keys = parse_authorized_keys(rsa_line)
        self.assertEqual(len(keys), 0)

    def test_parse_authorized_keys_ignores_corrupt_b64(self):
        bad_line = "ssh-ed25519 !@#$%^&*() admin@espressoos"
        keys = parse_authorized_keys(bad_line)
        self.assertEqual(len(keys), 0)

    def test_parse_authorized_keys_rejects_wrong_length(self):
        algo = b"ssh-ed25519"
        # Pubkey demasiado corta (16 bytes en vez de 32)
        short_blob = struct.pack(">I", len(algo)) + algo + struct.pack(">I", 16) + (b"\xaa" * 16)
        b64 = base64.b64encode(short_blob).decode()
        bad_line = f"ssh-ed25519 {b64} admin@espressoos"
        keys = parse_authorized_keys(bad_line)
        self.assertEqual(len(keys), 0)


if __name__ == "__main__":
    unittest.main()
