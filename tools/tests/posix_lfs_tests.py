#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
posix_lfs_tests.py -- Verificacion de la logica pura POSIX / LFS 13.1 de EspressoOS.

Porta fielmente los algoritmos de kernel y userland agregados para compatibilidad
POSIX / Linux From Scratch 13.1 sobre ESP32-S3 NOMMU:
  - Confinamiento `chroot` (`vfs/mount.rs::normalize_rooted`) y resolucion de
    enlaces simbolicos con deteccion de bucles (`MAX_SYMLINKS = 8`)
  - Endurecimiento de autenticacion SSH (`drivers/ssh/auth.rs::verify_stored`)
    y upsert multiusuario no destructivo de 7 campos (`drivers/passwd.rs`)
  - Bloqueos consultivos `flock` (`vfs/mod.rs`: LOCK_SH / LOCK_EX / LOCK_UN / release)
  - Orden topologico DAG de Micro-Systemd (`userland/apps/src/bin/init.rs`)
  - Operadores POSIX de `/bin/sh` (`&&`, `||`, `;`, `$VAR`/`${VAR}`/`$?`/`$$`,
    comillas simples y redirecciones `<`, `>`, `>>`, `2>`, `2>&1`)
  - Imagen persistente NOR `ELFS` + CRC32 de `LittleFs` (`fs/littlefs/mod.rs`)

Ejecutar:  python tools/tests/posix_lfs_tests.py   (o via run_all.py)
"""

import hashlib
import hmac
import unittest
from typing import Dict, List, Optional, Tuple


# ===========================================================================
# 1. VFS: normalize_rooted (chroot) + symlink traversal (MAX_SYMLINKS = 8)
# ===========================================================================

MAX_NAME_LEN = 64
MAX_SYMLINKS = 8


def normalize_against(base: str, path: str) -> str:
    effective_base = "" if path.startswith("/") else base
    comps: List[str] = []
    for part in effective_base.split("/") + path.split("/"):
        if part in ("", "."):
            continue
        if part == "..":
            if comps:
                comps.pop()
        else:
            if len(part) > MAX_NAME_LEN:
                raise ValueError("ENAMETOOLONG")
            comps.append(part)
    return "/" if not comps else "/" + "/".join(comps)


def normalize_rooted(root: str, cwd: str, path: str) -> str:
    if not root or root == "/":
        return normalize_against(cwd, path)
    if cwd == root:
        rel_cwd = "/"
    elif cwd.startswith(root + "/"):
        rel_cwd = cwd[len(root):]
    else:
        rel_cwd = "/"
    vpath = normalize_against(rel_cwd, path)
    if vpath == "/":
        return root
    return root.rstrip("/") + vpath


def resolve_with_symlinks(
    entries: Dict[str, Tuple[str, str]],
    root: str,
    cwd: str,
    path: str,
    follow_last: bool = True,
) -> str:
    cur = normalize_rooted(root, cwd, path)
    hops = 0
    while True:
        # Walk prefixes to resolve intermediate symlinks first
        comps = [c for c in cur.split("/") if c]
        rebuilt = ""
        redirected = False
        for idx, comp in enumerate(comps):
            rebuilt += "/" + comp
            is_last = idx == len(comps) - 1
            node = entries.get(rebuilt)
            if node is None:
                raise FileNotFoundError(rebuilt)
            kind, target = node
            if kind == "symlink" and (not is_last or follow_last):
                hops += 1
                if hops > MAX_SYMLINKS:
                    raise RuntimeError("ELOOP")
                parent = "/" if idx == 0 else "/" + "/".join(comps[:idx])
                resolved_target = normalize_rooted(root, parent, target)
                rest = "/".join(comps[idx + 1:])
                cur = resolved_target if not rest else resolved_target.rstrip("/") + "/" + rest
                cur = normalize_rooted(root, "/", cur[len(root):] if root != "/" and cur.startswith(root) else cur)
                redirected = True
                break
        if not redirected:
            return cur


# ===========================================================================
# 2. SSH Auth + /etc/passwd 7-field non-destructive upsert
# ===========================================================================

def pbkdf2_sha256(password: bytes, salt: bytes, rounds: int) -> bytes:
    return hashlib.pbkdf2_hmac("sha256", password, salt, rounds, dklen=32)


def verify_stored(field: str, password: bytes) -> bool:
    if not field or field in ("x", "*") or field.startswith("!"):
        return False
    if field.startswith("$pbkdf2-sha256$i="):
        rest = field[len("$pbkdf2-sha256$i="):]
        parts = rest.split("$")
        if len(parts) != 3:
            return False
        try:
            iterations = int(parts[0])
            salt = bytes.fromhex(parts[1])
            expected = bytes.fromhex(parts[2])
        except ValueError:
            return False
        if iterations <= 0 or iterations > 100_000 or len(expected) != 32:
            return False
        got = pbkdf2_sha256(password, salt, iterations)
        return hmac.compare_digest(got, expected)
    return hmac.compare_digest(field.encode("utf-8"), password)


def update_passwd_content(existing: str, user: str, hashed_field: str) -> str:
    if not user or ":" in user or "\n" in user:
        raise ValueError("EINVAL")
    out: List[str] = []
    updated = False
    max_uid = 999
    for raw_line in existing.splitlines():
        trimmed = raw_line.strip()
        if not trimmed:
            continue
        if trimmed.startswith("#"):
            out.append(trimmed)
            continue
        parts = trimmed.split(":")
        if len(parts) >= 3:
            try:
                u = int(parts[2])
                if 1000 <= u < 65000 and u > max_uid:
                    max_uid = u
            except ValueError:
                pass
        if parts[0].strip() == user:
            if len(parts) >= 7:
                out.append(f"{user}:{hashed_field}:" + ":".join(parts[2:7]))
            elif user == "root":
                out.append(f"root:{hashed_field}:0:0:root:/root:/bin/sh")
            else:
                max_uid += 1
                out.append(f"{user}:{hashed_field}:{max_uid}:{max_uid}:{user}:/home/{user}:/bin/sh")
            updated = True
        else:
            out.append(trimmed)
    if not updated:
        if user == "root":
            out.append(f"root:{hashed_field}:0:0:root:/root:/bin/sh")
        else:
            max_uid += 1
            out.append(f"{user}:{hashed_field}:{max_uid}:{max_uid}:{user}:/home/{user}:/bin/sh")
    return "\n".join(out) + "\n"


# ===========================================================================
# 3. Advisory flock state machine
# ===========================================================================

LOCK_SH = 1
LOCK_EX = 2
LOCK_NB = 4
LOCK_UN = 8


class FlockTable:
    def __init__(self):
        self.locks: List[Tuple[str, int, bool]] = []  # (key, owner, exclusive)

    def flock(self, key: str, owner: int, op: int) -> bool:
        cmd = op & ~LOCK_NB
        if cmd == LOCK_UN:
            self.locks = [e for e in self.locks if not (e[0] == key and e[1] == owner)]
            return True
        want_excl = cmd == LOCK_EX
        if not want_excl and cmd != LOCK_SH:
            raise ValueError("EINVAL")
        conflict = any(
            k == key and o != owner and (want_excl or excl)
            for (k, o, excl) in self.locks
        )
        if conflict:
            return False
        for idx, (k, o, _) in enumerate(self.locks):
            if k == key and o == owner:
                self.locks[idx] = (key, owner, want_excl)
                return True
        self.locks.append((key, owner, want_excl))
        return True

    def release_owner(self, owner: int):
        self.locks = [e for e in self.locks if e[1] != owner]


# ===========================================================================
# 4. Micro-Systemd Kahn DAG topological ordering
# ===========================================================================

def topo_order(units: List[Tuple[str, List[str], List[str]]]) -> List[str]:
    """units: [(name, after_list, requires_list)] -> ordered unit names."""
    n = len(units)
    name_to_idx = {u[0]: i for i, u in enumerate(units)}
    deps_of: List[List[int]] = [[] for _ in range(n)]
    indeg = [0] * n
    for i, (_, after, req) in enumerate(units):
        seen = set()
        for dep_name in after + req:
            j = name_to_idx.get(dep_name)
            if j is not None and j != i and j not in seen:
                seen.add(j)
                deps_of[i].append(j)
                indeg[i] += 1

    placed = [False] * n
    order: List[str] = []
    for _ in range(n):
        pick = None
        for i in range(n):
            if not placed[i] and indeg[i] == 0:
                pick = i
                break
        if pick is None:
            # Deterministic cycle break: lowest index unplaced unit
            for i in range(n):
                if not placed[i]:
                    pick = i
                    break
        placed[pick] = True
        order.append(units[pick][0])
        for k in range(n):
            if not placed[k] and pick in deps_of[k]:
                indeg[k] = max(0, indeg[k] - 1)
    return order


# ===========================================================================
# 5. /bin/sh conditional chains, variable expansion, redirections
# ===========================================================================

OP_ALWAYS, OP_AND, OP_OR = 0, 1, 2


def split_conditional_chain(line: str) -> List[Tuple[int, str]]:
    out: List[Tuple[int, str]] = []
    cur: List[str] = []
    next_op = OP_ALWAYS
    in_sq = False
    in_dq = False
    i = 0
    while i < len(line):
        c = line[i]
        if c == "'" and not in_dq:
            in_sq = not in_sq
            cur.append(c)
            i += 1
            continue
        if c == '"' and not in_sq:
            in_dq = not in_dq
            cur.append(c)
            i += 1
            continue
        if not in_sq and not in_dq:
            if line[i:i + 2] == "&&":
                seg = "".join(cur).strip()
                if seg:
                    out.append((next_op, seg))
                cur = []
                next_op = OP_AND
                i += 2
                continue
            if line[i:i + 2] == "||":
                seg = "".join(cur).strip()
                if seg:
                    out.append((next_op, seg))
                cur = []
                next_op = OP_OR
                i += 2
                continue
            if c == ";":
                seg = "".join(cur).strip()
                if seg:
                    out.append((next_op, seg))
                cur = []
                next_op = OP_ALWAYS
                i += 1
                continue
        cur.append(c)
        i += 1
    seg = "".join(cur).strip()
    if seg:
        out.append((next_op, seg))
    return out


def expand_vars(src: str, env: Dict[str, str], last_status: int, shell_pid: int) -> str:
    out: List[str] = []
    in_sq = False
    i = 0
    while i < len(src):
        c = src[i]
        if c == "'":
            in_sq = not in_sq
            out.append(c)
            i += 1
            continue
        if c == "$" and not in_sq and i + 1 < len(src):
            nxt = src[i + 1]
            if nxt == "?":
                out.append(str(last_status))
                i += 2
                continue
            if nxt == "$":
                out.append(str(shell_pid))
                i += 2
                continue
            if nxt == "{":
                end = src.find("}", i + 2)
                if end != -1:
                    key = src[i + 2:end]
                    out.append(env.get(key, ""))
                    i = end + 1
                    continue
            if nxt.isalpha() or nxt == "_":
                j = i + 1
                while j < len(src) and (src[j].isalnum() or src[j] == "_"):
                    j += 1
                out.append(env.get(src[i + 1:j], ""))
                i = j
                continue
        out.append(c)
        i += 1
    return "".join(out)


# ===========================================================================
# 6. LittleFs ELFS NOR Flash persistence image + CRC32
# ===========================================================================

LFS_MAGIC = 0x53464C45
LFS_VERSION = 1


def crc32_ieee(data: bytes) -> int:
    crc = 0xFFFFFFFF
    for b in data:
        crc ^= b
        for _ in range(8):
            mask = -(crc & 1) & 0xFFFFFFFF
            crc = (crc >> 1) ^ (0xEDB88320 & mask)
    return crc ^ 0xFFFFFFFF


def encode_elfs_image(entries: List[Tuple[int, int, int, int, int, int, str, bytes]]) -> bytes:
    payload = bytearray()
    for kind, ino, parent, mode, uid, gid, name, data in entries:
        nb = name.encode("utf-8")
        payload.append(kind)
        payload += ino.to_bytes(4, "little")
        payload += parent.to_bytes(4, "little")
        payload += mode.to_bytes(2, "little")
        payload += uid.to_bytes(2, "little")
        payload += gid.to_bytes(2, "little")
        payload += len(nb).to_bytes(2, "little")
        payload += len(data).to_bytes(4, "little")
        payload += nb
        payload += data
    hdr = (
        LFS_MAGIC.to_bytes(4, "little")
        + LFS_VERSION.to_bytes(4, "little")
        + len(payload).to_bytes(4, "little")
        + crc32_ieee(bytes(payload)).to_bytes(4, "little")
    )
    img = bytearray(hdr + payload)
    while len(img) % 4 != 0:
        img.append(0)
    return bytes(img)


def decode_elfs_image(img: bytes) -> Optional[List[Tuple[int, int, int, int, int, int, str, bytes]]]:
    if len(img) < 16:
        return None
    magic = int.from_bytes(img[0:4], "little")
    ver = int.from_bytes(img[4:8], "little")
    plen = int.from_bytes(img[8:12], "little")
    expected_crc = int.from_bytes(img[12:16], "little")
    if magic != LFS_MAGIC or ver != LFS_VERSION or 16 + plen > len(img):
        return None
    payload = img[16:16 + plen]
    if crc32_ieee(payload) != expected_crc:
        return None
    out = []
    pos = 0
    while pos + 21 <= len(payload):
        kind = payload[pos]
        ino = int.from_bytes(payload[pos + 1:pos + 5], "little")
        parent = int.from_bytes(payload[pos + 5:pos + 9], "little")
        mode = int.from_bytes(payload[pos + 9:pos + 11], "little")
        uid = int.from_bytes(payload[pos + 11:pos + 13], "little")
        gid = int.from_bytes(payload[pos + 13:pos + 15], "little")
        nlen = int.from_bytes(payload[pos + 15:pos + 17], "little")
        dlen = int.from_bytes(payload[pos + 17:pos + 21], "little")
        pos += 21
        name = payload[pos:pos + nlen].decode("utf-8")
        pos += nlen
        data = payload[pos:pos + dlen]
        pos += dlen
        out.append((kind, ino, parent, mode, uid, gid, name, data))
    return out


# ===========================================================================
# Test Suites
# ===========================================================================

class ChrootAndSymlinkTests(unittest.TestCase):
    def test_chroot_prevents_dotdot_escape(self):
        self.assertEqual(normalize_rooted("/tmp/jail", "/tmp/jail/sub", "../../../../etc/passwd"), "/tmp/jail/etc/passwd")
        self.assertEqual(normalize_rooted("/tmp/jail", "/tmp/jail", "/"), "/tmp/jail")
        self.assertEqual(normalize_rooted("/tmp/jail", "/tmp/jail/work", "notes.txt"), "/tmp/jail/work/notes.txt")

    def test_cross_mount_symlinks_and_loop_limit(self):
        tree = {
            "/var": ("dir", ""),
            "/var/run": ("symlink", "/run"),
            "/run": ("dir", ""),
            "/run/sshd.pid": ("file", ""),
            "/etc": ("dir", ""),
            "/etc/a": ("symlink", "/etc/b"),
            "/etc/b": ("symlink", "/etc/a"),
        }
        self.assertEqual(resolve_with_symlinks(tree, "/", "/", "/var/run/sshd.pid"), "/run/sshd.pid")
        self.assertEqual(resolve_with_symlinks(tree, "/", "/", "/var/run", follow_last=False), "/var/run")
        with self.assertRaises(RuntimeError):
            resolve_with_symlinks(tree, "/", "/", "/etc/a")


class PasswdAndSshAuthTests(unittest.TestCase):
    def test_locked_and_shadow_markers_never_match_plaintext(self):
        self.assertFalse(verify_stored("", b""))
        self.assertFalse(verify_stored("x", b"x"))
        self.assertFalse(verify_stored("*", b"*"))
        self.assertFalse(verify_stored("!", b"!"))
        self.assertFalse(verify_stored("!locked", b"!locked"))

    def test_pbkdf2_field_matches_only_valid_secret(self):
        salt = bytes.fromhex("00112233445566778899aabbccddeeff")
        digest = pbkdf2_sha256(b"s3cr3t!", salt, 1000).hex()
        field = f"$pbkdf2-sha256$i=1000${salt.hex()}${digest}"
        self.assertTrue(verify_stored(field, b"s3cr3t!"))
        self.assertFalse(verify_stored(field, b"wrong"))

    def test_update_passwd_preserves_other_accounts_and_7_fields(self):
        initial = (
            "root:$pbkdf2-sha256$i=1000$aa$bb:0:0:root:/root:/bin/sh\n"
            "alice:!:1000:1000:Alice:/home/alice:/bin/sh\n"
        )
        updated = update_passwd_content(initial, "alice", "$pbkdf2-sha256$i=1000$cc$dd")
        lines = updated.strip().splitlines()
        self.assertEqual(len(lines), 2)
        self.assertTrue(lines[0].startswith("root:$pbkdf2-sha256$i=1000$aa$bb:0:0:root:/root:/bin/sh"))
        self.assertEqual(lines[1], "alice:$pbkdf2-sha256$i=1000$cc$dd:1000:1000:Alice:/home/alice:/bin/sh")


class FlockAdvisoryTests(unittest.TestCase):
    def test_shared_vs_exclusive_and_release(self):
        ft = FlockTable()
        self.assertTrue(ft.flock("F:2", owner=10, op=LOCK_SH | LOCK_NB))
        self.assertTrue(ft.flock("F:2", owner=11, op=LOCK_SH | LOCK_NB))
        self.assertFalse(ft.flock("F:2", owner=12, op=LOCK_EX | LOCK_NB))
        ft.release_owner(10)
        ft.release_owner(11)
        self.assertTrue(ft.flock("F:2", owner=12, op=LOCK_EX | LOCK_NB))
        self.assertFalse(ft.flock("F:2", owner=10, op=LOCK_SH | LOCK_NB))
        self.assertTrue(ft.flock("F:2", owner=12, op=LOCK_UN))
        self.assertTrue(ft.flock("F:2", owner=10, op=LOCK_SH | LOCK_NB))


class MicroSystemdDagTests(unittest.TestCase):
    def test_topological_sort_respects_after_and_requires(self):
        units = [
            ("httpd", ["syslog"], []),
            ("cron", ["syslog"], []),
            ("syslog", [], []),
            ("app", ["httpd"], ["cron"]),
        ]
        ordered = topo_order(units)
        self.assertEqual(ordered[0], "syslog")
        self.assertLess(ordered.index("httpd"), ordered.index("app"))
        self.assertLess(ordered.index("cron"), ordered.index("app"))

    def test_cycle_breaks_deterministically_without_hang(self):
        units = [
            ("alpha", ["beta"], []),
            ("beta", ["alpha"], []),
        ]
        ordered = topo_order(units)
        self.assertEqual(sorted(ordered), ["alpha", "beta"])


class PosixShellTests(unittest.TestCase):
    def test_conditional_splitting_respects_quotes(self):
        chain = split_conditional_chain("mkdir /tmp/x && echo 'a && b' || echo fail; ls /tmp/x")
        self.assertEqual(
            chain,
            [
                (OP_ALWAYS, "mkdir /tmp/x"),
                (OP_AND, "echo 'a && b'"),
                (OP_OR, "echo fail"),
                (OP_ALWAYS, "ls /tmp/x"),
            ],
        )

    def test_variable_expansion_and_single_quotes(self):
        env = {"USER": "root", "HOME": "/root"}
        s = expand_vars("echo $USER ${HOME}/bin '$USER' $? $$", env, last_status=42, shell_pid=7)
        self.assertEqual(s, "echo root /root/bin '$USER' 42 7")


class LittleFsNorImageTests(unittest.TestCase):
    def test_elfs_roundtrip_and_crc_tamper_detection(self):
        entries = [
            (2, 2, 1, 0o755, 0, 0, "cfg", b""),
            (1, 3, 2, 0o600, 0, 0, "wifi.key", b"psk=espresso"),
            (3, 4, 1, 0o777, 0, 0, "current", b"/mnt/lfs/cfg/wifi.key"),
        ]
        img = encode_elfs_image(entries)
        self.assertEqual(decode_elfs_image(img), entries)
        corrupted = bytearray(img)
        corrupted[20] ^= 0x5A
        self.assertIsNone(decode_elfs_image(bytes(corrupted)))


if __name__ == "__main__":
    unittest.main()
