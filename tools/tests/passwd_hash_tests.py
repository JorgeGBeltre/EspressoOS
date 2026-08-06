#!/usr/bin/env python3
"""Tests del formato de hash de /etc/passwd (espeja drivers::passwd::hash_line y
ssh::auth::verify_stored en kernel/src/drivers/passwd.rs y kernel/src/drivers/ssh/auth.rs).

Verifica, sin toolchain de Rust, la LÓGICA PURA del formato `$s5$<salt-hex>$<hash-hex>`
(SHA-256(salt || password)): codificación hex, parseo del campo almacenado y el criterio
de aceptación/rechazo que usaría `check_password`. NO cubre el generador de salt (TRNG de
hardware) ni las syscalls/ioctl reales -- esas se validan en hardware.

Ejecutar:  python tools/tests/passwd_hash_tests.py
"""
from __future__ import annotations

import hashlib
import os
import unittest

SALT_LEN = 16


def hex_encode(b: bytes) -> str:
    return b.hex()


def hex_decode(s: str) -> bytes | None:
    if not s or len(s) % 2 != 0:
        return None
    try:
        return bytes.fromhex(s)
    except ValueError:
        return None


def make_line(user: str, password: bytes, salt: bytes) -> str:
    """Espejo de drivers::passwd::hash_line."""
    assert len(salt) == SALT_LEN
    digest = hashlib.sha256(salt + password).digest()
    return f"{user}:$s5${hex_encode(salt)}${hex_encode(digest)}\n"


def verify_stored(field: str, password: bytes) -> bool:
    """Espejo de ssh::auth::verify_stored: entiende `$s5$<salt-hex>$<hash-hex>` y hace
    fallback a comparación en texto plano para entradas legacy."""
    if field.startswith("$s5$"):
        rest = field[len("$s5$"):]
        parts = rest.split("$", 1)
        if len(parts) != 2:
            return False
        salt = hex_decode(parts[0])
        expected = hex_decode(parts[1])
        if salt is None or expected is None or len(expected) != 32:
            return False
        got = hashlib.sha256(salt + password).digest()
        return got == expected
    return field.encode() == password


class PasswdHashFormatTests(unittest.TestCase):
    def test_roundtrip_correct_password(self):
        salt = os.urandom(SALT_LEN)
        line = make_line("youareme", b"hunter2", salt)
        user, field = line.strip().split(":", 1)
        self.assertEqual(user, "youareme")
        self.assertTrue(verify_stored(field, b"hunter2"))

    def test_roundtrip_wrong_password_rejected(self):
        salt = os.urandom(SALT_LEN)
        line = make_line("youareme", b"hunter2", salt)
        _, field = line.strip().split(":", 1)
        self.assertFalse(verify_stored(field, b"wrong-password"))

    def test_different_salts_produce_different_hashes(self):
        salt_a = b"\x00" * SALT_LEN
        salt_b = b"\x01" * SALT_LEN
        line_a = make_line("u", b"samepass", salt_a)
        line_b = make_line("u", b"samepass", salt_b)
        self.assertNotEqual(line_a, line_b)

    def test_legacy_plaintext_field_still_verifies(self):
        # Entradas escritas antes de este cambio (o a mano) siguen aceptándose en
        # lectura -- passwd(1) siempre escribe el formato nuevo al reemplazarlas.
        self.assertTrue(verify_stored("hunter2", b"hunter2"))
        self.assertFalse(verify_stored("hunter2", b"wrong"))

    def test_malformed_s5_field_rejected_not_crashed(self):
        self.assertFalse(verify_stored("$s5$", b"x"))
        self.assertFalse(verify_stored("$s5$zz$aa", b"x"))  # hex inválido
        self.assertFalse(verify_stored("$s5$aa$bb", b"x"))  # hash truncado (< 32 bytes)
        self.assertFalse(verify_stored("$s5$onlyonefield", b"x"))

    def test_known_vector(self):
        salt = bytes(range(16))  # 000102...0f
        password = b"correct horse battery staple"
        expected = hashlib.sha256(salt + password).hexdigest()
        line = make_line("u", password, salt)
        self.assertIn(f"$s5$000102030405060708090a0b0c0d0e0f${expected}", line)


if __name__ == "__main__":
    unittest.main()
