#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
espfs_tests.py -- Verificacion de la LOGICA PURA de EspFs (Fase 4).

Porta a Python el formato en disco y el algoritmo de replay del log-structured
filesystem `kernel/src/fs/espfs/{wire.rs,mod.rs}`, para verificarlo sin flash ni
toolchain de Rust. Cubre:
  - CRC-32 y (de)serializado de registros y superbloque (wire.rs)
  - reproduccion del log -> arbol de inodos (mod.rs::replay)
  - splice de extents y resolucion de lecturas (mod.rs::splice_extent/read_file)
  - cola rota (CRC invalido) = fin del log

Ejecutar:  python tools/tests/espfs_tests.py   (o via run_all.py)
"""

import unittest
from typing import Dict, List, Optional, Tuple

# ===========================================================================
# PORT de wire.rs
# ===========================================================================

REC_MAGIC = 0xE5F5
SB_MAGIC = 0x45734653
VERSION = 1
HEADER_LEN = 16
SB_LEN = 20
WRITE_DATA_OFF = 8

MK_FILE, MK_DIR, WRITE, TRUNCATE, UNLINK, MK_SYMLINK, SET_ATTR, RENAME = 1, 2, 3, 4, 5, 6, 7, 8


def crc32(data: bytes) -> int:
    crc = 0xFFFFFFFF
    for b in data:
        crc ^= b
        for _ in range(8):
            mask = -(crc & 1) & 0xFFFFFFFF
            crc = (crc >> 1) ^ (0xEDB88320 & mask)
    return crc ^ 0xFFFFFFFF


def pad4(n: int) -> int:
    return (n + 3) & ~3


def record_total_len(plen: int) -> int:
    return HEADER_LEN + pad4(plen)


def encode_record(rtype: int, seq: int, payload: bytes) -> bytes:
    h = bytearray(HEADER_LEN)
    h[0] = REC_MAGIC & 0xFF
    h[1] = (REC_MAGIC >> 8) & 0xFF
    h[2] = rtype
    h[3] = 0
    h[4:8] = seq.to_bytes(4, "little")
    h[8:12] = len(payload).to_bytes(4, "little")
    crc = crc32(bytes(h[0:12]) + payload)
    h[12:16] = crc.to_bytes(4, "little")
    out = bytearray(h)
    out += payload
    out += b"\x00" * (record_total_len(len(payload)) - HEADER_LEN - len(payload))
    return bytes(out)


def parse_header(buf: bytes) -> Optional[Tuple[int, int, int, int]]:
    """(rtype, seq, plen, crc) o None si el magic no coincide."""
    if len(buf) < HEADER_LEN:
        return None
    if (buf[0] | (buf[1] << 8)) != REC_MAGIC:
        return None
    rtype = buf[2]
    if rtype not in (MK_FILE, MK_DIR, WRITE, TRUNCATE, UNLINK, MK_SYMLINK, SET_ATTR, RENAME):
        return None
    seq = int.from_bytes(buf[4:8], "little")
    plen = int.from_bytes(buf[8:12], "little")
    crc = int.from_bytes(buf[12:16], "little")
    return (rtype, seq, plen, crc)


def verify_crc(header16: bytes, payload: bytes, expected: int) -> bool:
    return crc32(bytes(header16[0:12]) + payload) == expected


def enc_mk(ino: int, parent: int, name: bytes) -> bytes:
    return ino.to_bytes(4, "little") + parent.to_bytes(4, "little") + name


def enc_write(ino: int, offset: int, data: bytes) -> bytes:
    return ino.to_bytes(4, "little") + offset.to_bytes(4, "little") + data


def enc_trunc(ino: int, length: int) -> bytes:
    return ino.to_bytes(4, "little") + length.to_bytes(4, "little")


def enc_unlink(parent: int, name: bytes) -> bytes:
    return parent.to_bytes(4, "little") + name


def enc_symlink(ino: int, parent: int, name: bytes, target: bytes) -> bytes:
    return (
        ino.to_bytes(4, "little")
        + parent.to_bytes(4, "little")
        + len(name).to_bytes(2, "little")
        + len(target).to_bytes(2, "little")
        + name
        + target
    )


def enc_setattr(ino: int, mode: int, uid: int, gid: int) -> bytes:
    return (
        ino.to_bytes(4, "little")
        + mode.to_bytes(2, "little")
        + uid.to_bytes(2, "little")
        + gid.to_bytes(2, "little")
        + (0).to_bytes(2, "little")
    )


def enc_rename(old_parent: int, new_parent: int, old_name: bytes, new_name: bytes) -> bytes:
    return (
        old_parent.to_bytes(4, "little")
        + new_parent.to_bytes(4, "little")
        + len(old_name).to_bytes(2, "little")
        + len(new_name).to_bytes(2, "little")
        + old_name
        + new_name
    )


def encode_super(generation: int, active_half: int) -> bytes:
    b = bytearray(SB_LEN)
    b[0:4] = SB_MAGIC.to_bytes(4, "little")
    b[4:8] = VERSION.to_bytes(4, "little")
    b[8:12] = generation.to_bytes(4, "little")
    b[12:16] = active_half.to_bytes(4, "little")
    b[16:20] = crc32(bytes(b[0:16])).to_bytes(4, "little")
    return bytes(b)


def decode_super(b: bytes) -> Optional[Tuple[int, int]]:
    if len(b) < SB_LEN:
        return None
    if int.from_bytes(b[0:4], "little") != SB_MAGIC:
        return None
    if int.from_bytes(b[4:8], "little") != VERSION:
        return None
    if crc32(bytes(b[0:16])) != int.from_bytes(b[16:20], "little"):
        return None
    return (int.from_bytes(b[8:12], "little"), int.from_bytes(b[12:16], "little"))


# ===========================================================================
# PORT de mod.rs: indice en RAM + splice + replay sobre un buffer de "flash".
# ===========================================================================

ROOT_INO = 1


class Node:
    def __init__(self, kind: str, target: str = ""):
        self.kind = kind  # "dir" | "file" | "symlink"
        self.children: Dict[str, int] = {}
        self.size = len(target.encode("utf-8")) if kind == "symlink" else 0
        self.extents: List[Tuple[int, int, int]] = []  # (off, len, flash)
        self.target = target
        self.mode = 0o755 if kind == "dir" else (0o777 if kind == "symlink" else 0o644)
        self.uid = 0
        self.gid = 0


class FsModel:
    """Reproduce mod.rs sobre un buffer `flash` (bytes de la mitad de log)."""

    def __init__(self, flash: bytes, half_base: int = 0):
        self.flash = flash
        self.half_base = half_base
        self.nodes: Dict[int, Node] = {ROOT_INO: Node("dir")}
        self.next_ino = 2
        self.next_seq = 1

    # -- aplicacion de registros (espeja apply_*) --
    def apply_mk(self, kind: str, ino: int, parent: int, name: str):
        self.nodes[ino] = Node(kind)
        p = self.nodes.get(parent)
        if p and p.kind == "dir":
            p.children[name] = ino
        if ino >= self.next_ino:
            self.next_ino = ino + 1

    def splice_extent(self, ino: int, off: int, length: int, flash: int):
        n = self.nodes.get(ino)
        if not n or n.kind != "file" or length == 0:
            return
        new_end = off + length
        out = []
        for (eo, el, ef) in n.extents:
            ee = eo + el
            if ee <= off or eo >= new_end:
                out.append((eo, el, ef))
            else:
                if eo < off:
                    out.append((eo, off - eo, ef))
                if ee > new_end:
                    out.append((new_end, ee - new_end, ef + (new_end - eo)))
        out.append((off, length, flash))
        out.sort(key=lambda e: e[0])
        n.extents = out
        if new_end > n.size:
            n.size = new_end

    def apply_trunc(self, ino: int, length: int):
        n = self.nodes.get(ino)
        if not n or n.kind != "file":
            return
        out = []
        for (eo, el, ef) in n.extents:
            ee = eo + el
            if eo >= length:
                continue
            if ee <= length:
                out.append((eo, el, ef))
            else:
                out.append((eo, length - eo, ef))
        n.extents = out
        n.size = length

    def apply_unlink(self, parent: int, name: str):
        p = self.nodes.get(parent)
        if not p or p.kind != "dir":
            return
        ci = p.children.pop(name, None)
        if ci is not None:
            self.nodes.pop(ci, None)

    def unlink_checked(self, parent: int, name: str, caller_uid: int) -> bool:
        p = self.nodes.get(parent)
        if not p or p.kind != "dir":
            return False
        ci = p.children.get(name)
        if ci is None:
            return False
        child = self.nodes.get(ci)
        if (p.mode & 0o1000) != 0 and caller_uid != 0:
            if caller_uid != p.uid and (child is None or caller_uid != child.uid):
                return False
        self.apply_unlink(parent, name)
        return True

    def apply_symlink(self, ino: int, parent: int, name: str, target: str):
        self.nodes[ino] = Node("symlink", target)
        p = self.nodes.get(parent)
        if p and p.kind == "dir":
            p.children[name] = ino
        if ino >= self.next_ino:
            self.next_ino = ino + 1

    def apply_setattr(self, ino: int, mode: int, uid: int, gid: int):
        n = self.nodes.get(ino)
        if n is not None:
            n.mode = mode
            n.uid = uid
            n.gid = gid

    def apply_rename(self, old_parent: int, new_parent: int, old_name: str, new_name: str):
        op = self.nodes.get(old_parent)
        np = self.nodes.get(new_parent)
        if not op or op.kind != "dir" or not np or np.kind != "dir":
            return
        moved = op.children.pop(old_name, None)
        if moved is not None:
            replaced = np.children.get(new_name)
            np.children[new_name] = moved
            if replaced is not None and replaced != moved:
                self.nodes.pop(replaced, None)

    # -- lectura resolviendo extents (espeja read_file) --
    def read(self, ino: int, off: int, length: int) -> bytes:
        n = self.nodes[ino]
        if off >= n.size:
            return b""
        cnt = min(length, n.size - off)
        buf = bytearray(cnt)
        read_end = off + cnt
        for (eo, el, ef) in n.extents:
            ee = eo + el
            s = max(eo, off)
            e = min(ee, read_end)
            if s < e:
                fo = ef + (s - eo)
                buf[s - off:e - off] = self.flash[fo:fo + (e - s)]
        return bytes(buf)

    # -- replay (espeja mod.rs::replay) --
    def replay(self, base: int, end: int):
        cur = base
        while cur + HEADER_LEN <= end:
            hbuf = self.flash[cur:cur + HEADER_LEN]
            h = parse_header(hbuf)
            if h is None:
                break
            rtype, seq, plen, crc = h
            total = record_total_len(plen)
            if cur + total > end:
                break
            payload = self.flash[cur + HEADER_LEN:cur + HEADER_LEN + plen]
            if not verify_crc(hbuf, payload, crc):
                break
            if rtype == WRITE:
                ino = int.from_bytes(payload[0:4], "little")
                off = int.from_bytes(payload[4:8], "little")
                dlen = plen - WRITE_DATA_OFF
                dflash = cur + HEADER_LEN + WRITE_DATA_OFF
                self.splice_extent(ino, off, dlen, dflash)
            elif rtype in (MK_FILE, MK_DIR):
                ino = int.from_bytes(payload[0:4], "little")
                parent = int.from_bytes(payload[4:8], "little")
                name = payload[8:].decode("utf-8")
                self.apply_mk("dir" if rtype == MK_DIR else "file", ino, parent, name)
            elif rtype == TRUNCATE:
                ino = int.from_bytes(payload[0:4], "little")
                length = int.from_bytes(payload[4:8], "little")
                self.apply_trunc(ino, length)
            elif rtype == UNLINK:
                parent = int.from_bytes(payload[0:4], "little")
                name = payload[4:].decode("utf-8")
                self.apply_unlink(parent, name)
            elif rtype == MK_SYMLINK:
                ino = int.from_bytes(payload[0:4], "little")
                parent = int.from_bytes(payload[4:8], "little")
                nlen = int.from_bytes(payload[8:10], "little")
                tlen = int.from_bytes(payload[10:12], "little")
                name = payload[12:12 + nlen].decode("utf-8")
                target = payload[12 + nlen:12 + nlen + tlen].decode("utf-8")
                self.apply_symlink(ino, parent, name, target)
            elif rtype == SET_ATTR:
                ino = int.from_bytes(payload[0:4], "little")
                mode = int.from_bytes(payload[4:6], "little")
                uid = int.from_bytes(payload[6:8], "little")
                gid = int.from_bytes(payload[8:10], "little")
                self.apply_setattr(ino, mode, uid, gid)
            elif rtype == RENAME:
                old_p = int.from_bytes(payload[0:4], "little")
                new_p = int.from_bytes(payload[4:8], "little")
                olen = int.from_bytes(payload[8:10], "little")
                nlen = int.from_bytes(payload[10:12], "little")
                old_name = payload[12:12 + olen].decode("utf-8")
                new_name = payload[12 + olen:12 + olen + nlen].decode("utf-8")
                self.apply_rename(old_p, new_p, old_name, new_name)
            if seq >= self.next_seq:
                self.next_seq = seq + 1
            cur += total
        return cur


class LogBuilder:
    """Construye un buffer de log emitiendo registros como lo hace `append`."""

    def __init__(self):
        self.buf = bytearray()
        self.seq = 1

    def emit(self, rtype: int, payload: bytes) -> int:
        off = len(self.buf)
        self.buf += encode_record(rtype, self.seq, payload)
        self.seq += 1
        return off  # offset del registro dentro del buffer


# ===========================================================================
# Tests
# ===========================================================================


class WireTests(unittest.TestCase):
    def test_record_roundtrip(self):
        payload = enc_write(7, 100, b"hello world")
        rec = encode_record(WRITE, 42, payload)
        self.assertEqual(len(rec) % 4, 0)
        h = parse_header(rec)
        self.assertIsNotNone(h)
        rtype, seq, plen, crc = h
        self.assertEqual((rtype, seq, plen), (WRITE, 42, len(payload)))
        self.assertTrue(verify_crc(rec[0:16], rec[16:16 + plen], crc))

    def test_torn_record_fails_crc(self):
        payload = enc_mk(3, 1, b"foo")
        rec = bytearray(encode_record(MK_FILE, 1, payload))
        _, _, plen, crc = parse_header(rec)
        rec[16] ^= 0xFF
        self.assertFalse(verify_crc(rec[0:16], rec[16:16 + plen], crc))

    def test_super_roundtrip(self):
        enc = encode_super(5, 1)
        self.assertEqual(decode_super(enc), (5, 1))
        bad = bytearray(enc)
        bad[8] ^= 0xFF
        self.assertIsNone(decode_super(bad))


class ReplayTests(unittest.TestCase):
    def test_tree_and_data(self):
        lb = LogBuilder()
        lb.emit(MK_DIR, enc_mk(2, ROOT_INO, b"etc"))
        lb.emit(MK_FILE, enc_mk(3, 2, b"motd"))
        lb.emit(WRITE, enc_write(3, 0, b"welcome"))
        m = FsModel(bytes(lb.buf))
        m.replay(0, len(lb.buf))
        # arbol
        self.assertEqual(m.nodes[ROOT_INO].children, {"etc": 2})
        self.assertEqual(m.nodes[2].kind, "dir")
        self.assertEqual(m.nodes[2].children, {"motd": 3})
        self.assertEqual(m.read(3, 0, 100), b"welcome")

    def test_overwrite_supersedes(self):
        lb = LogBuilder()
        lb.emit(MK_FILE, enc_mk(2, ROOT_INO, b"f"))
        lb.emit(WRITE, enc_write(2, 0, b"AAAAAAAA"))
        lb.emit(WRITE, enc_write(2, 2, b"xy"))  # solapa en [2,4)
        m = FsModel(bytes(lb.buf))
        m.replay(0, len(lb.buf))
        self.assertEqual(m.read(2, 0, 100), b"AAxyAAAA")

    def test_truncate_shrinks(self):
        lb = LogBuilder()
        lb.emit(MK_FILE, enc_mk(2, ROOT_INO, b"f"))
        lb.emit(WRITE, enc_write(2, 0, b"0123456789"))
        lb.emit(TRUNCATE, enc_trunc(2, 4))
        m = FsModel(bytes(lb.buf))
        m.replay(0, len(lb.buf))
        self.assertEqual(m.nodes[2].size, 4)
        self.assertEqual(m.read(2, 0, 100), b"0123")

    def test_truncate_extends_with_holes(self):
        lb = LogBuilder()
        lb.emit(MK_FILE, enc_mk(2, ROOT_INO, b"f"))
        lb.emit(WRITE, enc_write(2, 0, b"ab"))
        lb.emit(TRUNCATE, enc_trunc(2, 5))
        m = FsModel(bytes(lb.buf))
        m.replay(0, len(lb.buf))
        self.assertEqual(m.read(2, 0, 100), b"ab\x00\x00\x00")

    def test_unlink_removes(self):
        lb = LogBuilder()
        lb.emit(MK_FILE, enc_mk(2, ROOT_INO, b"f"))
        lb.emit(WRITE, enc_write(2, 0, b"data"))
        lb.emit(UNLINK, enc_unlink(ROOT_INO, b"f"))
        m = FsModel(bytes(lb.buf))
        m.replay(0, len(lb.buf))
        self.assertEqual(m.nodes[ROOT_INO].children, {})
        self.assertNotIn(2, m.nodes)

    def test_torn_tail_stops_replay(self):
        lb = LogBuilder()
        lb.emit(MK_FILE, enc_mk(2, ROOT_INO, b"keep"))
        lb.emit(WRITE, enc_write(2, 0, b"safe"))
        good_len = len(lb.buf)
        # Registro extra corrupto (payload alterado tras calcular CRC).
        rec = bytearray(encode_record(WRITE, lb.seq, enc_write(2, 4, b"LOST")))
        rec[16] ^= 0xFF
        buf = bytes(lb.buf) + bytes(rec) + b"\xff" * 64
        m = FsModel(buf)
        end_cur = m.replay(0, len(buf))
        # El replay se detiene justo donde acaba el log valido.
        self.assertEqual(end_cur, good_len)
        self.assertEqual(m.read(2, 0, 100), b"safe")

    def test_erased_region_is_empty_log(self):
        buf = b"\xff" * 256
        m = FsModel(buf)
        cur = m.replay(0, len(buf))
        self.assertEqual(cur, 0)
        self.assertEqual(m.nodes[ROOT_INO].children, {})

    def test_symlink_setattr_and_atomic_rename_replay(self):
        lb = LogBuilder()
        lb.emit(MK_DIR, enc_mk(2, ROOT_INO, b"etc"))
        lb.emit(MK_DIR, enc_mk(3, ROOT_INO, b"tmp"))
        lb.emit(SET_ATTR, enc_setattr(3, 0o1777, 0, 0))
        lb.emit(MK_SYMLINK, enc_symlink(4, 2, b"mtab", b"/proc/self/mounts"))
        lb.emit(MK_FILE, enc_mk(5, 3, b"draft.txt"))
        lb.emit(WRITE, enc_write(5, 0, b"v2"))
        lb.emit(SET_ATTR, enc_setattr(5, 0o600, 1000, 1000))
        lb.emit(MK_FILE, enc_mk(6, 2, b"final.txt"))
        lb.emit(WRITE, enc_write(6, 0, b"v1"))
        lb.emit(RENAME, enc_rename(3, 2, b"draft.txt", b"final.txt"))

        m = FsModel(bytes(lb.buf))
        m.replay(0, len(lb.buf))

        self.assertEqual(m.nodes[4].kind, "symlink")
        self.assertEqual(m.nodes[4].target, "/proc/self/mounts")
        self.assertEqual(m.nodes[3].mode, 0o1777)
        self.assertNotIn("draft.txt", m.nodes[3].children)
        self.assertEqual(m.nodes[2].children["final.txt"], 5)
        self.assertNotIn(6, m.nodes)
        self.assertEqual(m.read(5, 0, 16), b"v2")
        self.assertEqual((m.nodes[5].mode, m.nodes[5].uid, m.nodes[5].gid), (0o600, 1000, 1000))

    def test_sticky_bit_unlink_protection(self):
        lb = LogBuilder()
        lb.emit(MK_DIR, enc_mk(2, ROOT_INO, b"tmp"))
        lb.emit(SET_ATTR, enc_setattr(2, 0o1777, 0, 0))
        lb.emit(MK_FILE, enc_mk(3, 2, b"alice.txt"))
        lb.emit(SET_ATTR, enc_setattr(3, 0o644, 1000, 1000))
        m = FsModel(bytes(lb.buf))
        m.replay(0, len(lb.buf))
        self.assertFalse(m.unlink_checked(2, "alice.txt", caller_uid=1001))
        self.assertIn("alice.txt", m.nodes[2].children)
        self.assertTrue(m.unlink_checked(2, "alice.txt", caller_uid=1000))
        self.assertNotIn("alice.txt", m.nodes[2].children)


class SpliceTests(unittest.TestCase):
    def test_write_inside_existing(self):
        # Un extent grande y una escritura interior lo parte en tres.
        m = FsModel(b"")
        m.nodes[2] = Node("file")
        m.splice_extent(2, 0, 10, 1000)  # flash 1000..1010
        m.splice_extent(2, 3, 2, 2000)  # flash 2000..2002
        exts = m.nodes[2].extents
        # Esperado: [0,3)@1000, [3,5)@2000, [5,10)@1005
        self.assertEqual(exts, [(0, 3, 1000), (3, 2, 2000), (5, 5, 1005)])


if __name__ == "__main__":
    unittest.main()
