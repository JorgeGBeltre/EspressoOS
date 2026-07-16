#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
logic_tests.py -- Arnes de verificacion de la LOGICA PURA del kernel.

Este archivo PORTA a Python los algoritmos deterministas del kernel escrito en
Rust (`kernel/src/**`) para poder verificarlos SIN necesidad de una toolchain
de Rust. Cada bloque "PORT" es una traduccion fiel, linea a linea, del Rust
correspondiente; los `mod tests` de Rust no pueden ejecutarse porque el kernel
es `#![no_std] #![no_main]` y esta marcado "COMPILE-STATUS: borrador".

Modulos replicados:
  1. Tokenizador/parser de shell   -> kernel/src/shell/parser.rs
  2. Normalizacion de rutas VFS     -> kernel/src/vfs/mount.rs
  3. Seleccion de slot OTA A/B      -> kernel/src/ota/partition.rs
  4. Semantica de ramfs             -> kernel/src/fs/ramfs.rs

Ejecutar:
    python tools/tests/logic_tests.py         (usa unittest)
    python -m pytest tools/tests/logic_tests.py

Si algun test falla, revela una divergencia entre el comportamiento
implementado en Rust y el comportamiento correcto/documentado: en ese caso hay
que corregir el archivo Rust correspondiente.
"""

import unittest
from dataclasses import dataclass, field
from typing import List, Optional, Tuple

# ===========================================================================
# Error canonico del kernel (espeja prelude::KError). Se modela como excepcion
# portando un codigo de variante; KResult<T> se modela como "devuelve T o
# lanza KError".
# ===========================================================================


class KError(Exception):
    def __init__(self, code: str):
        super().__init__(code)
        self.code = code

    def __eq__(self, other):
        return isinstance(other, KError) and self.code == other.code

    def __hash__(self):
        return hash(self.code)

    def __repr__(self):
        return f"KError::{self.code}"


# Variantes usadas por la logica portada.
INVALID_ARGUMENT = "InvalidArgument"
NAME_TOO_LONG = "NameTooLong"
NOT_FOUND = "NotFound"
ALREADY_EXISTS = "AlreadyExists"
NOT_A_DIRECTORY = "NotADirectory"
IS_A_DIRECTORY = "IsADirectory"
NOT_SUPPORTED = "NotSupported"
BUSY = "Busy"
NO_MEM = "NoMem"
NO_SPACE = "NoSpace"


# ===========================================================================
# 1. PORT de kernel/src/shell/parser.rs  (tokenizador + parser de tuberias)
# ===========================================================================
#
# Representacion de Token:
#   ("W", texto)   -> Token::Word(texto)   (ya sin comillas)
#   (">",)         -> Token::RedirectOut
#   (">>",)        -> Token::RedirectAppend
#   ("|",)         -> Token::Pipe
#   (";",)         -> Token::Semi
#
# Representacion de Redirect:
#   ("none",)          -> Redirect::None
#   ("trunc", nombre)  -> Redirect::Truncate(nombre)
#   ("append", nombre) -> Redirect::Append(nombre)


def W(s: str):
    return ("W", s)


def tokenize(line: str):
    """Espeja parser.rs::tokenize."""
    tokens = []
    current = []          # buffer de la palabra en curso (lista de chars)
    has_word = False      # distingue "sin palabra" de "palabra vacia en curso"
    quote = "no"          # "no" | "single" | "double"

    chars = list(line)
    n = len(chars)
    i = 0
    while i < n:
        c = chars[i]
        i += 1

        if quote == "single":
            if c == "'":
                quote = "no"
            else:
                current.append(c)
            has_word = True

        elif quote == "double":
            if c == '"':
                quote = "no"
            elif c == "\\":
                nxt = chars[i] if i < n else None
                if nxt == '"' or nxt == "\\":
                    current.append(nxt)
                    i += 1
                else:
                    current.append("\\")
            else:
                current.append(c)
            has_word = True

        else:  # quote == "no"
            if c in (" ", "\t", "\r", "\n"):
                if has_word:
                    tokens.append(W("".join(current)))
                    current = []
                    has_word = False
            elif c == "'":
                quote = "single"
                has_word = True
            elif c == '"':
                quote = "double"
                has_word = True
            elif c == "\\":
                if i < n:
                    current.append(chars[i])
                    i += 1
                else:
                    current.append("\\")
                has_word = True
            elif c == "|":
                if has_word:
                    tokens.append(W("".join(current)))
                    current = []
                    has_word = False
                tokens.append(("|",))
            elif c == ";":
                if has_word:
                    tokens.append(W("".join(current)))
                    current = []
                    has_word = False
                tokens.append((";",))
            elif c == ">":
                if has_word:
                    tokens.append(W("".join(current)))
                    current = []
                    has_word = False
                if i < n and chars[i] == ">":
                    i += 1
                    tokens.append((">>",))
                else:
                    tokens.append((">",))
            else:
                current.append(c)
                has_word = True

    if quote != "no":
        raise KError(INVALID_ARGUMENT)
    if has_word:
        tokens.append(W("".join(current)))
    return tokens


@dataclass
class Command:
    name: str
    args: List[str]
    redirect: Tuple  # ("none",) | ("trunc", name) | ("append", name)


def _build_command(words: List[str], redirect_holder: List) -> Command:
    """Espeja parser.rs::build_command. `redirect_holder` es una caja [redir]."""
    if not words:
        raise KError(INVALID_ARGUMENT)
    name = words.pop(0)
    args = words[:]
    del words[:]
    redir = redirect_holder[0]
    redirect_holder[0] = ("none",)
    return Command(name=name, args=args, redirect=redir)


def _pipeline_from_tokens(tokens: List) -> List[Command]:
    """Espeja parser.rs::pipeline_from_tokens."""
    if not tokens:
        return []

    commands: List[Command] = []
    words: List[str] = []
    redirect_holder = [("none",)]
    pending: Optional[bool] = None  # None | False(>) | True(>>)

    for tok in tokens:
        if pending is not None:
            if tok[0] == "W":
                w = tok[1]
                redirect_holder[0] = ("append", w) if pending else ("trunc", w)
                pending = None
                continue
            else:
                raise KError(INVALID_ARGUMENT)

        kind = tok[0]
        if kind == "W":
            words.append(tok[1])
        elif kind == ">":
            pending = False
        elif kind == ">>":
            pending = True
        elif kind == "|":
            commands.append(_build_command(words, redirect_holder))
        elif kind == ";":
            # parse_line los quita antes de llamar aqui.
            raise KError(INVALID_ARGUMENT)

    if pending is not None:
        raise KError(INVALID_ARGUMENT)

    commands.append(_build_command(words, redirect_holder))
    return commands


def parse_line(line: str) -> List[List[Command]]:
    """Espeja parser.rs::parse_line.

    `;` separa tuberias que corren una detras de otra; `|` separa las etapas de
    una. Los dos los reconoce el tokenizador, asi que las comillas protegen a los
    dos: `echo "a;b"` es una sola palabra.
    """
    out: List[List[Command]] = []
    segment: List = []

    for tok in tokenize(line):
        if tok[0] == ";":
            # Un segmento vacio no separa nada: `ls ;` es una orden y `ls ;; cat`
            # son dos. Se salta en vez de ser error de sintaxis, que es lo que hace
            # cualquier shell y lo que vuelve inofensivo un `;` final.
            if segment:
                out.append(_pipeline_from_tokens(segment))
                segment = []
        else:
            segment.append(tok)
    if segment:
        out.append(_pipeline_from_tokens(segment))
    return out


def parse_pipeline(line: str) -> List[Command]:
    """Espeja parser.rs::parse_pipeline. Una sola tuberia; rechaza `;`."""
    return _pipeline_from_tokens(tokenize(line))


def parse(line: str) -> Optional[Command]:
    """Espeja parser.rs::parse. Devuelve la primera etapa, o None si vacia."""
    pipeline = parse_pipeline(line)
    if not pipeline:
        return None
    return pipeline.pop(0)


# ===========================================================================
# 2. PORT de kernel/src/vfs/mount.rs  (normalizacion/particion de rutas)
# ===========================================================================

MAX_NAME_LEN = 255


def normalize_against(base: str, path: str) -> str:
    """Espeja mount.rs::normalize_against.

    La mitad PURA de la normalizacion. La otra mitad, `mount::normalize`, resuelve
    una ruta relativa contra el cwd del proceso llamante consultando la tabla de
    procesos -- eso no se puede espejar aqui y no se intenta. Lo que se espeja es
    todo lo que decide QUE significa una ruta; lo que queda fuera es solo QUIEN
    pregunta, que en el Rust es una sola linea.
    """
    # Primero de todo: "" no es una ruta relativa, no nombra nada. Sin esto caeria
    # al join y saldria como `base`, o sea el cwd del llamante -- y `unlink(x, 0)`
    # llega hasta aqui como "" sin puntero valido ninguno.
    if path == "":
        raise KError(INVALID_ARGUMENT)

    # Una ruta absoluta ignora la base. Comprobado aqui y no como precondicion del
    # llamador, para que la funcion no se pueda llamar mal.
    if path.startswith("/"):
        base = ""

    comps: List[str] = []
    # Los componentes de la base primero, luego los de la ruta: eso es lo que
    # significa "relativa a", y es lo que deja que ".." cruce la union.
    for part in base.split("/") + path.split("/"):
        if part == "" or part == ".":
            pass
        elif part == "..":
            if comps:
                comps.pop()
        else:
            if len(part) > MAX_NAME_LEN:
                raise KError(NAME_TOO_LONG)
            comps.append(part)

    if not comps:
        return "/"
    return "/" + "/".join(comps)


def split_parent(path: str) -> Tuple[str, str]:
    """Espeja mount.rs::split_parent."""
    if path == "" or path == "/":
        raise KError(INVALID_ARGUMENT)
    idx = path.rfind("/")
    if idx == -1:
        raise KError(INVALID_ARGUMENT)
    if idx == 0:
        return ("/", path[1:])
    name = path[idx + 1:]
    if name == "":
        raise KError(INVALID_ARGUMENT)
    return (path[:idx], name)


def split_components(normalized: str) -> List[str]:
    """Espeja mount.rs::split_components."""
    return [s for s in normalized.split("/") if s != ""]


# ===========================================================================
# 3. PORT de kernel/src/ota/partition.rs  (seleccion de slot OTA A/B)
# ===========================================================================

SLOT_COUNT = 2
OTA_SELECT_ENTRY_SIZE = 32
OTA_SEQ_EMPTY = 0xFFFFFFFF
U32_MASK = 0xFFFFFFFF

# Slot A/B modelado como cadenas.
SLOT_FACTORY = "Factory"
SLOT_OTA0 = "Ota0"


def slot_from_index(idx: int) -> str:
    """Espeja Slot::from_index (0 -> Factory, resto -> Ota0)."""
    return SLOT_FACTORY if idx == 0 else SLOT_OTA0


def slot_index(slot: str) -> int:
    """Espeja Slot::index."""
    return 0 if slot == SLOT_FACTORY else 1


def slot_other(slot: str) -> str:
    return SLOT_OTA0 if slot == SLOT_FACTORY else SLOT_FACTORY


def crc32_le(seed: int, data: bytes) -> int:
    """Espeja partition.rs::crc32_le (CRC-32 reflejado, poly 0xEDB88320)."""
    crc = (~seed) & U32_MASK
    for byte in data:
        crc ^= byte
        crc &= U32_MASK
        for _ in range(8):
            mask = U32_MASK if (crc & 1) else 0
            crc = ((crc >> 1) ^ (0xEDB88320 & mask)) & U32_MASK
    return (~crc) & U32_MASK


@dataclass
class OtaSelectEntry:
    ota_seq: int
    seq_label: bytes
    ota_state: int
    crc: int

    @staticmethod
    def empty() -> "OtaSelectEntry":
        return OtaSelectEntry(OTA_SEQ_EMPTY, b"\xff" * 20, OTA_SEQ_EMPTY, OTA_SEQ_EMPTY)

    @staticmethod
    def new(ota_seq: int, state: int) -> "OtaSelectEntry":
        e = OtaSelectEntry(ota_seq & U32_MASK, b"\x00" * 20, state & U32_MASK, 0)
        e.crc = e.compute_crc()
        return e

    def compute_crc(self) -> int:
        return crc32_le(0xFFFFFFFF, (self.ota_seq & U32_MASK).to_bytes(4, "little"))

    def is_valid(self) -> bool:
        return (
            self.ota_seq != OTA_SEQ_EMPTY
            and self.ota_seq != 0
            and self.crc == self.compute_crc()
        )

    def to_bytes(self) -> bytes:
        b = bytearray(OTA_SELECT_ENTRY_SIZE)
        b[0:4] = (self.ota_seq & U32_MASK).to_bytes(4, "little")
        b[4:24] = self.seq_label
        b[24:28] = (self.ota_state & U32_MASK).to_bytes(4, "little")
        b[28:32] = (self.crc & U32_MASK).to_bytes(4, "little")
        return bytes(b)

    @staticmethod
    def from_bytes(buf: bytes) -> "OtaSelectEntry":
        if len(buf) < OTA_SELECT_ENTRY_SIZE:
            raise KError(INVALID_ARGUMENT)
        ota_seq = int.from_bytes(buf[0:4], "little")
        seq_label = bytes(buf[4:24])
        ota_state = int.from_bytes(buf[24:28], "little")
        crc = int.from_bytes(buf[28:32], "little")
        return OtaSelectEntry(ota_seq, seq_label, ota_state, crc)


def select_active_index(entries: List[OtaSelectEntry]) -> Optional[int]:
    """Espeja partition.rs::select_active_index. `entries` = [copia0, copia1]."""
    v0 = entries[0].is_valid()
    v1 = entries[1].is_valid()
    if v0 and v1:
        return 0 if entries[0].ota_seq > entries[1].ota_seq else 1
    if v0 and not v1:
        return 0
    if not v0 and v1:
        return 1
    return None


def slot_from_seq(seq: int) -> str:
    """Espeja partition.rs::slot_from_seq. wrapping_sub replicado con mascara."""
    idx = ((seq - 1) & U32_MASK) % SLOT_COUNT
    return slot_from_index(idx)


def next_seq_for_index(current_max: int, target_index: int) -> int:
    """Espeja partition.rs::next_seq_for_index."""
    if target_index >= SLOT_COUNT:
        raise KError(INVALID_ARGUMENT)
    base = target_index + 1
    if base > U32_MASK:
        raise KError(INVALID_ARGUMENT)
    seq = current_max + 1
    if seq > U32_MASK:
        raise KError(NO_SPACE)
    if seq < base:
        seq = base
    for _ in range(SLOT_COUNT):
        if ((seq - 1) & U32_MASK) % SLOT_COUNT == target_index:
            return seq
        seq += 1
        if seq > U32_MASK:
            raise KError(NO_SPACE)
    raise KError(INVALID_ARGUMENT)


def active_slot(entries: List[OtaSelectEntry]) -> str:
    """Espeja partition.rs::active_slot (parte pura: dado el par de entradas)."""
    i = select_active_index(entries)
    if i is None:
        return SLOT_FACTORY
    return slot_from_seq(entries[i].ota_seq)


def compute_write_copy(entries: List[OtaSelectEntry]) -> int:
    """Copia INACTIVA donde set_boot_slot escribiria (espeja set_boot_slot)."""
    active = select_active_index(entries)
    if active == 0:
        return 1
    if active is None:
        return 0
    return 0


# ===========================================================================
# 4. PORT de kernel/src/fs/ramfs.rs  (crear/leer/escribir/borrar/listar)
# ===========================================================================

_NEXT_INO = [1]


def _alloc_ino() -> int:
    v = _NEXT_INO[0]
    _NEXT_INO[0] += 1
    return v


@dataclass
class DirEntry:
    name: str
    kind: str  # "file" | "dir"
    ino: int


class RamNode:
    """Espeja ramfs.rs::RamNode (archivo o directorio)."""

    def __init__(self, kind: str):
        self.ino = _alloc_ino()
        self.kind = kind  # "file" | "dir"
        if kind == "file":
            self._data = bytearray()
            self._children = None
        else:
            self._data = None
            self._children = {}  # nombre -> RamNode

    @staticmethod
    def new_dir() -> "RamNode":
        return RamNode("dir")

    @staticmethod
    def new_file() -> "RamNode":
        return RamNode("file")

    def size(self) -> int:
        if self.kind == "file":
            return len(self._data)
        return 0

    def read_at(self, off: int, length: int) -> bytes:
        """Espeja Inode::read_at. Devuelve los bytes leidos (hasta `length`)."""
        if self.kind != "file":
            raise KError(IS_A_DIRECTORY)
        if off < 0:
            return b""
        start = off
        if start >= len(self._data):
            return b""
        disponible = len(self._data) - start
        nread = min(disponible, length)
        return bytes(self._data[start:start + nread])

    def write_at(self, off: int, buf: bytes) -> int:
        """Espeja Inode::write_at. Extiende con ceros (huecos = 0)."""
        if self.kind != "file":
            raise KError(IS_A_DIRECTORY)
        if off < 0:
            raise KError(INVALID_ARGUMENT)
        start = off
        end = start + len(buf)
        if end > len(self._data):
            self._data.extend(b"\x00" * (end - len(self._data)))
        self._data[start:end] = buf
        return len(buf)

    def truncate(self, length: int) -> None:
        """Espeja Inode::truncate."""
        if self.kind != "file":
            raise KError(IS_A_DIRECTORY)
        if length < 0:
            raise KError(INVALID_ARGUMENT)
        if length > len(self._data):
            self._data.extend(b"\x00" * (length - len(self._data)))
        else:
            del self._data[length:]

    def readdir(self, index: int) -> Optional[DirEntry]:
        """Espeja Inode::readdir. Orden estable tipo BTreeMap (claves ordenadas)."""
        if self.kind != "dir":
            raise KError(NOT_A_DIRECTORY)
        names = sorted(self._children.keys())
        if index < 0 or index >= len(names):
            return None
        name = names[index]
        node = self._children[name]
        return DirEntry(name=name, kind=node.kind, ino=node.ino)

    def lookup(self, name: str) -> "RamNode":
        """Espeja Inode::lookup."""
        if self.kind != "dir":
            raise KError(NOT_A_DIRECTORY)
        node = self._children.get(name)
        if node is None:
            raise KError(NOT_FOUND)
        return node

    def create(self, name: str, kind: str) -> "RamNode":
        """Espeja Inode::create."""
        if self.kind != "dir":
            raise KError(NOT_A_DIRECTORY)
        if name in self._children:
            raise KError(ALREADY_EXISTS)
        if kind == "file":
            node = RamNode.new_file()
        elif kind == "dir":
            node = RamNode.new_dir()
        else:  # "device" | "symlink"
            raise KError(NOT_SUPPORTED)
        self._children[name] = node
        return node

    def unlink(self, name: str) -> None:
        """Espeja Inode::unlink. Directorio no vacio -> Busy."""
        if self.kind != "dir":
            raise KError(NOT_A_DIRECTORY)
        node = self._children.get(name)
        if node is None:
            raise KError(NOT_FOUND)
        if node.kind == "dir":
            vacio = len(node._children) == 0
        else:
            vacio = True
        if not vacio:
            raise KError(BUSY)
        del self._children[name]


class RamFs:
    """Espeja ramfs.rs::RamFs."""

    def __init__(self):
        self._root = RamNode.new_dir()

    @staticmethod
    def new() -> "RamFs":
        return RamFs()

    def name(self) -> str:
        return "ramfs"

    def root(self) -> RamNode:
        return self._root

    @staticmethod
    def _used_bytes(node: RamNode) -> int:
        if node.kind == "file":
            return len(node._data)
        total = 0
        for child in node._children.values():
            total += RamFs._used_bytes(child)
        return total

    def stat(self):
        usado = RamFs._used_bytes(self._root)
        return {"total_bytes": usado, "used_bytes": usado, "block_size": 1}


# ===========================================================================
# TESTS
# ===========================================================================


class TestShellTokenizer(unittest.TestCase):
    # --- Casos portados de los #[test] de parser.rs -----------------------
    def test_palabras_simples(self):
        self.assertEqual(tokenize("echo hola mundo"),
                         [W("echo"), W("hola"), W("mundo")])

    def test_colapsa_espacios_repetidos(self):
        self.assertEqual(tokenize("  ls    -la  "), [W("ls"), W("-la")])

    def test_respeta_comillas_dobles(self):
        self.assertEqual(tokenize('echo "hola   mundo"'),
                         [W("echo"), W("hola   mundo")])

    def test_respeta_comillas_simples(self):
        self.assertEqual(tokenize("echo 'a | b > c'"),
                         [W("echo"), W("a | b > c")])

    def test_comillas_vacias_producen_palabra_vacia(self):
        self.assertEqual(tokenize('echo ""'), [W("echo"), W("")])

    def test_concatena_partes_entrecomilladas(self):
        self.assertEqual(tokenize('a"b c"d'), [W("ab cd")])

    def test_escape_fuera_de_comillas(self):
        self.assertEqual(tokenize("a\\ b"), [W("a b")])

    def test_escape_en_comillas_dobles(self):
        # entrada Rust: "\"a\\\"b\"" == la cadena  "a\"b"
        self.assertEqual(tokenize('"a\\"b"'), [W('a"b')])

    def test_detecta_operadores_pegados(self):
        self.assertEqual(tokenize("echo hi>out"),
                         [W("echo"), W("hi"), (">",), W("out")])

    def test_distingue_append_de_truncate(self):
        self.assertEqual(tokenize("cat a >> b"),
                         [W("cat"), W("a"), (">>",), W("b")])

    def test_detecta_tuberia(self):
        self.assertEqual(tokenize("ls | cat"), [W("ls"), ("|",), W("cat")])

    def test_comilla_sin_cerrar_es_error(self):
        with self.assertRaises(KError) as cm:
            tokenize('echo "abc')
        self.assertEqual(cm.exception.code, INVALID_ARGUMENT)

    # --- Casos adicionales derivados de la especificacion -----------------
    def test_comilla_simple_sin_cerrar_es_error(self):
        with self.assertRaises(KError) as cm:
            tokenize("echo 'abc")
        self.assertEqual(cm.exception.code, INVALID_ARGUMENT)

    def test_escape_de_barra_en_comillas_dobles(self):
        # "a\\b" en Rust -> la cadena a\b  =>  palabra  a\b
        self.assertEqual(tokenize(r'"a\\b"'), [W(r"a\b")])

    def test_barra_no_escape_en_comillas_dobles(self):
        # dentro de "" solo \" y \\ son escapes; \n queda literal como \ + n
        self.assertEqual(tokenize(r'"a\nb"'), [W(r"a\nb")])

    def test_barra_final_de_linea_literal(self):
        self.assertEqual(tokenize("a\\"), [W("a\\")])

    def test_tuberia_pegada_a_palabras(self):
        self.assertEqual(tokenize("ls|cat"), [W("ls"), ("|",), W("cat")])

    def test_append_pegado(self):
        self.assertEqual(tokenize("echo hi>>out"),
                         [W("echo"), W("hi"), (">>",), W("out")])

    def test_linea_vacia_sin_tokens(self):
        self.assertEqual(tokenize("   \t \r\n "), [])

    def test_multiples_espacios_tab_cr(self):
        self.assertEqual(tokenize("a\tb\rc"), [W("a"), W("b"), W("c")])


class TestShellParser(unittest.TestCase):
    def test_comando_con_redireccion(self):
        cmd = parse("echo hola > /tmp/a.txt")
        self.assertEqual(cmd, Command("echo", ["hola"], ("trunc", "/tmp/a.txt")))

    def test_append(self):
        cmd = parse("cat log >> /tmp/all")
        self.assertEqual(cmd.redirect, ("append", "/tmp/all"))

    def test_linea_vacia_es_none(self):
        self.assertIsNone(parse("   \t "))

    def test_pipeline_dos_etapas(self):
        stages = parse_pipeline("ls -l | cat")
        self.assertEqual(len(stages), 2)
        self.assertEqual(stages[0].name, "ls")
        self.assertEqual(stages[0].args, ["-l"])
        self.assertEqual(stages[1].name, "cat")

    def test_redireccion_sin_archivo_es_error(self):
        with self.assertRaises(KError) as cm:
            parse_pipeline("echo hi >")
        self.assertEqual(cm.exception.code, INVALID_ARGUMENT)

    def test_tuberia_con_etapa_vacia_es_error(self):
        for line in ("| ls", "ls |"):
            with self.assertRaises(KError) as cm:
                parse_pipeline(line)
            self.assertEqual(cm.exception.code, INVALID_ARGUMENT)

    def test_redireccion_seguida_de_operador_es_error(self):
        with self.assertRaises(KError):
            parse_pipeline("echo hi > | cat")

    def test_pipeline_tres_etapas_con_redir(self):
        stages = parse_pipeline("a b | c | d > out")
        self.assertEqual([s.name for s in stages], ["a", "c", "d"])
        self.assertEqual(stages[0].args, ["b"])
        self.assertEqual(stages[2].redirect, ("trunc", "out"))

    def test_redir_antes_de_args_se_asocia_a_la_etapa(self):
        # "> out echo hi": name=echo, args=[hi], redirect trunc out
        cmd = parse("> out echo hi")
        self.assertEqual(cmd, Command("echo", ["hi"], ("trunc", "out")))


class TestShellSemi(unittest.TestCase):
    def test_separa_ordenes(self):
        pipes = parse_line("mkdir a ; cd a")
        self.assertEqual(len(pipes), 2)
        self.assertEqual(pipes[0][0], Command("mkdir", ["a"], ("none",)))
        self.assertEqual(pipes[1][0], Command("cd", ["a"], ("none",)))

    def test_sin_espacios_alrededor(self):
        pipes = parse_line("pwd;ls")
        self.assertEqual(len(pipes), 2)
        self.assertEqual(pipes[0][0].name, "pwd")
        self.assertEqual(pipes[1][0].name, "ls")

    def test_las_comillas_protegen_el_punto_y_coma(self):
        # LA razon de que `;` sea un token y no un line.split(';') previo. Con split
        # esto se partiria en dos y `echo` recibiria "a como argumento.
        pipes = parse_line('echo "a;b"')
        self.assertEqual(len(pipes), 1)
        self.assertEqual(pipes[0][0], Command("echo", ["a;b"], ("none",)))
        pipes = parse_line("echo 'x ; y'")
        self.assertEqual(len(pipes), 1)
        self.assertEqual(pipes[0][0].args, ["x ; y"])

    def test_punto_y_coma_final_o_repetido_es_inofensivo(self):
        for linea in ("ls ;", "ls ; ", " ; ls", "ls ;; pwd", ";;;"):
            parse_line(linea)  # no lanza
        self.assertEqual(len(parse_line("ls ;")), 1)
        self.assertEqual(len(parse_line("ls ;; pwd")), 2)
        self.assertEqual(parse_line(";;;"), [])

    def test_convive_con_tuberias_y_redirecciones(self):
        pipes = parse_line("ls | cat ; echo hi > f")
        self.assertEqual(len(pipes), 2)
        self.assertEqual(len(pipes[0]), 2)  # ls | cat
        self.assertEqual(pipes[1][0], Command("echo", ["hi"], ("trunc", "f")))

    def test_error_de_sintaxis_en_una_orden_invalida_toda_la_linea(self):
        # La linea entera se parsea antes de ejecutar nada, asi que `rm x ; echo >`
        # no debe borrar x y luego quejarse.
        with self.assertRaises(KError):
            parse_line("rm x ; echo >")

    def test_parse_pipeline_rechaza_el_punto_y_coma(self):
        with self.assertRaises(KError) as cm:
            parse_pipeline("ls ; cat")
        self.assertEqual(cm.exception.code, INVALID_ARGUMENT)


class TestVfsNormalize(unittest.TestCase):
    def test_normaliza_raiz(self):
        for p in ("/", "///", "/.", "/..", "/../.."):
            self.assertEqual(normalize_against("", p), "/")

    def test_colapsa_barras_y_puntos(self):
        self.assertEqual(normalize_against("", "/a//b"), "/a/b")
        self.assertEqual(normalize_against("", "/a/./b"), "/a/b")
        self.assertEqual(normalize_against("", "/a/b/"), "/a/b")
        self.assertEqual(normalize_against("", "//a///b////c//"), "/a/b/c")

    def test_resuelve_punto_punto(self):
        self.assertEqual(normalize_against("", "/a/../b"), "/b")
        self.assertEqual(normalize_against("", "/a/b/../c"), "/a/c")
        self.assertEqual(normalize_against("", "/a/b/../../c"), "/c")
        self.assertEqual(normalize_against("", "/a/../../b"), "/b")
        self.assertEqual(normalize_against("", "/dev/../tmp/./x"), "/tmp/x")

    def test_punto_punto_por_encima_de_raiz(self):
        self.assertEqual(normalize_against("", "/a/b/../../../c"), "/c")

    def test_rechaza_vacias(self):
        # "" no es una ruta relativa: no nombra nada. Este test se llamaba
        # `test_rechaza_relativas` y cubria tres cadenas que fallaban por el mismo
        # motivo accidental (ninguna empieza por "/"). Ya no es el mismo motivo:
        # "a/b" y "./x" ahora se resuelven contra la base, y solo "" sigue siendo
        # un error. Es la unica cadena que NO debe recoger la base.
        for base in ("", "/", "/tmp", "/tmp/x"):
            with self.assertRaises(KError) as cm:
                normalize_against(base, "")
            self.assertEqual(cm.exception.code, INVALID_ARGUMENT)

    def test_resuelve_relativas_contra_la_base(self):
        self.assertEqual(normalize_against("/tmp", "x"), "/tmp/x")
        self.assertEqual(normalize_against("/tmp", "./x"), "/tmp/x")
        self.assertEqual(normalize_against("/tmp", "a/b"), "/tmp/a/b")
        self.assertEqual(normalize_against("/", "a/b"), "/a/b")

    def test_relativa_nombra_el_cwd_y_su_padre(self):
        # Lo que hace que `rm .` sea por fin expresable en la frontera del VFS, en
        # vez de llegar ya colapsado por el shell.
        self.assertEqual(normalize_against("/tmp", "."), "/tmp")
        self.assertEqual(normalize_against("/tmp/x", ".."), "/tmp")

    def test_punto_punto_cruza_la_union(self):
        # La razon de que la base entre como componentes y no como prefijo de texto.
        self.assertEqual(normalize_against("/tmp/x", "../y"), "/tmp/y")
        self.assertEqual(normalize_against("/a/b/c", "../../d"), "/a/d")

    def test_relativa_no_escapa_por_debajo_de_la_raiz(self):
        self.assertEqual(normalize_against("/", ".."), "/")
        self.assertEqual(normalize_against("/tmp", "../../.."), "/")

    def test_absoluta_ignora_la_base(self):
        self.assertEqual(normalize_against("/tmp", "/a/b"), "/a/b")
        self.assertEqual(normalize_against("/tmp/x", "/"), "/")

    def test_nombre_demasiado_largo(self):
        largo = "/" + "a" * (MAX_NAME_LEN + 1)
        with self.assertRaises(KError) as cm:
            normalize_against("", largo)
        self.assertEqual(cm.exception.code, NAME_TOO_LONG)

    def test_nombre_en_el_limite_ok(self):
        # exactamente MAX_NAME_LEN caracteres: valido.
        justo = "/" + "a" * MAX_NAME_LEN
        self.assertEqual(normalize_against("", justo), justo)

    def test_split_parent(self):
        self.assertEqual(split_parent("/foo/bar"), ("/foo", "bar"))
        self.assertEqual(split_parent("/foo"), ("/", "foo"))
        with self.assertRaises(KError) as cm:
            split_parent("/")
        self.assertEqual(cm.exception.code, INVALID_ARGUMENT)

    def test_split_components(self):
        self.assertEqual(split_components("/"), [])
        self.assertEqual(split_components("/a/b/c"), ["a", "b", "c"])


class TestOtaSelect(unittest.TestCase):
    def _entry(self, seq, state=0):
        return OtaSelectEntry.new(seq, state)

    def test_crc32_vector_estandar(self):
        # `crc32_le` replica `esp_rom_crc32_le`, que INVIERTE la semilla al
        # entrar (`crc = !seed`) y el resultado al salir. Por tanto el vector
        # estandar CRC-32 de "123456789" (0xCBF43926) se obtiene con SEMILLA 0
        # (=> init interno 0xFFFFFFFF), no con 0xFFFFFFFF. Esto valida el port
        # bit-a-bit contra el CRC-32 canonico (zlib/ISO-HDLC).
        self.assertEqual(crc32_le(0x00000000, b"123456789"), 0xCBF43926)

    def test_crc32_convencion_semilla_esp_rom(self):
        # La convencion de esp-idf para otadata es
        # `crc32_le(UINT32_MAX, &ota_seq, 4)`. Con la inversion de semilla del
        # ESP ROM eso NO coincide con el CRC-32 crudo; se fija aqui como
        # regresion de la convencion realmente usada por el kernel.
        self.assertEqual(crc32_le(0xFFFFFFFF, b"123456789"), 0xD202D277)
        # Y es exactamente lo que compute_crc usa (semilla 0xFFFFFFFF sobre
        # los 4 bytes LE de ota_seq): auto-consistente escritura<->lectura.
        e = OtaSelectEntry.new(1, 0)
        self.assertEqual(e.crc, crc32_le(0xFFFFFFFF, (1).to_bytes(4, "little")))

    def test_entrada_nueva_es_valida(self):
        e = self._entry(1)
        self.assertTrue(e.is_valid())

    def test_entrada_vacia_no_es_valida(self):
        self.assertFalse(OtaSelectEntry.empty().is_valid())

    def test_seq_cero_no_es_valida(self):
        e = self._entry(0)  # new() calcula crc, pero seq==0 se rechaza
        self.assertFalse(e.is_valid())

    def test_crc_corrupto_no_es_valida(self):
        e = self._entry(5)
        e.crc ^= 0x1
        self.assertFalse(e.is_valid())

    def test_serializacion_ida_y_vuelta(self):
        e = self._entry(7, state=2)
        again = OtaSelectEntry.from_bytes(e.to_bytes())
        self.assertEqual(again, e)

    def test_to_bytes_longitud(self):
        self.assertEqual(len(self._entry(3).to_bytes()), OTA_SELECT_ENTRY_SIZE)

    def test_from_bytes_buffer_corto(self):
        with self.assertRaises(KError) as cm:
            OtaSelectEntry.from_bytes(b"\x00" * 31)
        self.assertEqual(cm.exception.code, INVALID_ARGUMENT)

    # --- select_active_index ---------------------------------------------
    def test_ambas_invalidas_devuelve_none(self):
        entries = [OtaSelectEntry.empty(), OtaSelectEntry.empty()]
        self.assertIsNone(select_active_index(entries))
        self.assertEqual(active_slot(entries), SLOT_FACTORY)

    def test_solo_copia0_valida(self):
        entries = [self._entry(1), OtaSelectEntry.empty()]
        self.assertEqual(select_active_index(entries), 0)

    def test_solo_copia1_valida(self):
        entries = [OtaSelectEntry.empty(), self._entry(1)]
        self.assertEqual(select_active_index(entries), 1)

    def test_mayor_seq_gana(self):
        entries = [self._entry(3), self._entry(5)]
        self.assertEqual(select_active_index(entries), 1)
        entries = [self._entry(9), self._entry(4)]
        self.assertEqual(select_active_index(entries), 0)

    def test_empate_prefiere_copia1(self):
        entries = [self._entry(4), self._entry(4)]
        self.assertEqual(select_active_index(entries), 1)

    # --- slot_from_seq ----------------------------------------------------
    def test_slot_from_seq_alterna(self):
        self.assertEqual(slot_from_seq(1), SLOT_FACTORY)  # (1-1)%2 = 0
        self.assertEqual(slot_from_seq(2), SLOT_OTA0)     # (2-1)%2 = 1
        self.assertEqual(slot_from_seq(3), SLOT_FACTORY)
        self.assertEqual(slot_from_seq(4), SLOT_OTA0)

    def test_active_slot_end_to_end(self):
        # copia0 seq3 (Factory), copia1 seq4 (Ota0): gana seq4 -> Ota0.
        entries = [self._entry(3), self._entry(4)]
        self.assertEqual(active_slot(entries), SLOT_OTA0)

    # --- next_seq_for_index ----------------------------------------------
    def test_next_seq_desde_cero(self):
        self.assertEqual(next_seq_for_index(0, 0), 1)  # Factory
        self.assertEqual(next_seq_for_index(0, 1), 2)  # Ota0

    def test_next_seq_estrictamente_mayor(self):
        # activo seq1 (Factory); pedir arrancar Ota0 -> seq2.
        self.assertEqual(next_seq_for_index(1, 1), 2)
        # activo seq2 (Ota0); pedir Factory -> seq3.
        self.assertEqual(next_seq_for_index(2, 0), 3)
        # pedir de nuevo el mismo slot activo salta al siguiente valido.
        self.assertEqual(next_seq_for_index(2, 1), 4)
        self.assertEqual(next_seq_for_index(3, 0), 5)

    def test_next_seq_mapea_al_slot_pedido(self):
        for cur in range(0, 10):
            for target in (0, 1):
                seq = next_seq_for_index(cur, target)
                self.assertGreater(seq, cur)
                self.assertEqual(slot_index(slot_from_seq(seq)), target)

    def test_next_seq_target_invalido(self):
        with self.assertRaises(KError) as cm:
            next_seq_for_index(0, 2)
        self.assertEqual(cm.exception.code, INVALID_ARGUMENT)

    # --- rotacion completa (simula set_boot_slot repetido) ---------------
    def test_rotacion_ab_conserva_copia_previa(self):
        entries = [OtaSelectEntry.empty(), OtaSelectEntry.empty()]
        # Sin otadata valida -> arranca Factory.
        self.assertEqual(active_slot(entries), SLOT_FACTORY)

        # set_boot_slot(Ota0)
        wc = compute_write_copy(entries)
        self.assertEqual(wc, 0)
        cur = 0
        seq = next_seq_for_index(cur, slot_index(SLOT_OTA0))
        entries[wc] = OtaSelectEntry.new(seq, 0)
        self.assertEqual(active_slot(entries), SLOT_OTA0)

        # set_boot_slot(Factory): escribe en la copia inactiva (1).
        wc = compute_write_copy(entries)
        self.assertEqual(wc, 1)
        active = select_active_index(entries)
        cur = entries[active].ota_seq
        seq = next_seq_for_index(cur, slot_index(SLOT_FACTORY))
        entries[wc] = OtaSelectEntry.new(seq, 0)
        self.assertEqual(active_slot(entries), SLOT_FACTORY)
        # La copia 0 (Ota0 anterior) sigue intacta y valida.
        self.assertTrue(entries[0].is_valid())


class TestRamfs(unittest.TestCase):
    def setUp(self):
        self.fs = RamFs.new()
        self.root = self.fs.root()

    def test_crear_y_listar(self):
        self.root.create("b.txt", "file")
        self.root.create("a.txt", "file")
        self.root.create("dir", "dir")
        # readdir en orden ordenado (BTreeMap): a.txt, b.txt, dir
        names = []
        i = 0
        while True:
            e = self.root.readdir(i)
            if e is None:
                break
            names.append(e.name)
            i += 1
        self.assertEqual(names, ["a.txt", "b.txt", "dir"])

    def test_crear_duplicado_es_error(self):
        self.root.create("x", "file")
        with self.assertRaises(KError) as cm:
            self.root.create("x", "file")
        self.assertEqual(cm.exception.code, ALREADY_EXISTS)

    def test_crear_dispositivo_no_soportado(self):
        with self.assertRaises(KError) as cm:
            self.root.create("d", "device")
        self.assertEqual(cm.exception.code, NOT_SUPPORTED)

    def test_escribir_y_leer(self):
        f = self.root.create("f", "file")
        n = f.write_at(0, b"hola")
        self.assertEqual(n, 4)
        self.assertEqual(f.read_at(0, 10), b"hola")
        self.assertEqual(f.size(), 4)

    def test_lectura_parcial_y_offset(self):
        f = self.root.create("f", "file")
        f.write_at(0, b"0123456789")
        self.assertEqual(f.read_at(3, 4), b"3456")
        # offset en/ pasado EOF -> vacio
        self.assertEqual(f.read_at(10, 4), b"")
        self.assertEqual(f.read_at(100, 4), b"")

    def test_escritura_con_hueco_rellena_ceros(self):
        f = self.root.create("f", "file")
        f.write_at(5, b"AB")
        self.assertEqual(f.size(), 7)
        self.assertEqual(f.read_at(0, 7), b"\x00\x00\x00\x00\x00AB")

    def test_sobrescritura_en_medio(self):
        f = self.root.create("f", "file")
        f.write_at(0, b"aaaaaa")
        f.write_at(2, b"XX")
        self.assertEqual(f.read_at(0, 6), b"aaXXaa")

    def test_truncate_extiende_y_recorta(self):
        f = self.root.create("f", "file")
        f.write_at(0, b"abcdef")
        f.truncate(3)
        self.assertEqual(f.read_at(0, 10), b"abc")
        f.truncate(5)  # extiende con ceros
        self.assertEqual(f.read_at(0, 10), b"abc\x00\x00")

    def test_borrar_archivo(self):
        self.root.create("f", "file")
        self.root.unlink("f")
        with self.assertRaises(KError) as cm:
            self.root.lookup("f")
        self.assertEqual(cm.exception.code, NOT_FOUND)

    def test_borrar_inexistente(self):
        with self.assertRaises(KError) as cm:
            self.root.unlink("nope")
        self.assertEqual(cm.exception.code, NOT_FOUND)

    def test_borrar_dir_no_vacio_es_busy(self):
        d = self.root.create("d", "dir")
        d.create("child", "file")
        with self.assertRaises(KError) as cm:
            self.root.unlink("d")
        self.assertEqual(cm.exception.code, BUSY)
        # tras vaciarlo, se puede borrar
        d.unlink("child")
        self.root.unlink("d")

    def test_lookup_devuelve_mismo_nodo(self):
        f = self.root.create("f", "file")
        self.assertIs(self.root.lookup("f"), f)

    def test_lookup_en_archivo_es_notdir(self):
        f = self.root.create("f", "file")
        with self.assertRaises(KError) as cm:
            f.lookup("x")
        self.assertEqual(cm.exception.code, NOT_A_DIRECTORY)

    def test_read_en_directorio_es_isdir(self):
        d = self.root.create("d", "dir")
        with self.assertRaises(KError) as cm:
            d.read_at(0, 4)
        self.assertEqual(cm.exception.code, IS_A_DIRECTORY)

    def test_readdir_en_archivo_es_notdir(self):
        f = self.root.create("f", "file")
        with self.assertRaises(KError) as cm:
            f.readdir(0)
        self.assertEqual(cm.exception.code, NOT_A_DIRECTORY)

    def test_stat_suma_recursiva(self):
        d = self.root.create("d", "dir")
        d.create("a", "file").write_at(0, b"12345")
        self.root.create("b", "file").write_at(0, b"XYZ")
        st = self.fs.stat()
        self.assertEqual(st["used_bytes"], 8)
        self.assertEqual(st["total_bytes"], 8)
        self.assertEqual(st["block_size"], 1)


if __name__ == "__main__":
    unittest.main(verbosity=2)
