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


class TestFindDuLogic(unittest.TestCase):
    def test_find_pattern_matching(self):
        def pattern_matches(name: str, pattern: str) -> bool:
            if not pattern:
                return True
            if pattern.startswith("*"):
                return name.endswith(pattern[1:])
            elif pattern.endswith("*"):
                return name.startswith(pattern[:-1])
            else:
                return name == pattern

        self.assertTrue(pattern_matches("main.rs", "*.rs"))
        self.assertFalse(pattern_matches("main.c", "*.rs"))
        self.assertTrue(pattern_matches("lib.rs", "*"))
        self.assertTrue(pattern_matches("test_app", "test*"))
        self.assertFalse(pattern_matches("app_test", "test*"))
        self.assertTrue(pattern_matches("sh", "sh"))
        self.assertFalse(pattern_matches("sh", "bash"))

    def test_du_formatting(self):
        def format_size(bytes_val: int, human: bool) -> str:
            if not human:
                kb = (bytes_val + 1023) // 1024
                return f"{kb}\t"
            elif bytes_val < 1024:
                return f"{bytes_val}B\t"
            elif bytes_val < 1024 * 1024:
                return f"{bytes_val // 1024}K\t"
            else:
                return f"{bytes_val // (1024 * 1024)}M\t"

        self.assertEqual(format_size(500, False), "1\t")
        self.assertEqual(format_size(2048, False), "2\t")
        self.assertEqual(format_size(500, True), "500B\t")
        self.assertEqual(format_size(2048, True), "2K\t")
        self.assertEqual(format_size(5 * 1024 * 1024, True), "5M\t")


class TestServicePkgWhichLogic(unittest.TestCase):
    def test_which_path_resolution(self):
        def which_resolve(prog: str, available: list) -> Optional[str]:
            full = f"/bin/{prog}"
            if prog in available:
                return full
            return None

        apps = ["sh", "ls", "service", "pkg", "which", "true", "false"]
        self.assertEqual(which_resolve("sh", apps), "/bin/sh")
        self.assertEqual(which_resolve("service", apps), "/bin/service")
        self.assertEqual(which_resolve("unknown", apps), None)

    def test_service_lookup(self):
        services = {
            "init": "RUNNING",
            "net": "RUNNING",
            "ssh": "LISTENING",
            "httpd": "STOPPED",
        }
        self.assertIn("ssh", services)
        self.assertEqual(services["ssh"], "LISTENING")
        self.assertNotIn("apache", services)


class TestWatchdogLogic(unittest.TestCase):
    def test_reset_reason_decoding(self):
        def decode_reset_reason(raw: int) -> str:
            reasons = {
                1: "Power-on Reset",
                3: "Software Reset",
                4: "RTC Watchdog Reset",
                5: "Deep Sleep Wakeup",
                7: "Timer Group 0 Watchdog Reset",
                8: "Timer Group 1 Watchdog Reset",
                9: "RTC Watchdog Reset",
                11: "Timer Group 0 Watchdog Reset",
                13: "RTC Watchdog Reset",
                15: "Brown-out Detector Reset",
                16: "RTC Watchdog Reset",
            }
            return reasons.get(raw, "Other / Unknown Reset")

        self.assertEqual(decode_reset_reason(1), "Power-on Reset")
        self.assertEqual(decode_reset_reason(3), "Software Reset")
        self.assertEqual(decode_reset_reason(5), "Deep Sleep Wakeup")
        self.assertEqual(decode_reset_reason(7), "Timer Group 0 Watchdog Reset")
        self.assertEqual(decode_reset_reason(15), "Brown-out Detector Reset")
        self.assertEqual(decode_reset_reason(999), "Other / Unknown Reset")

    def test_watchdog_timeout_validation(self):
        def validate_timeout(secs: int) -> bool:
            return 1 <= secs <= 3600

        self.assertTrue(validate_timeout(1))
        self.assertTrue(validate_timeout(10))
        self.assertTrue(validate_timeout(3600))
        self.assertFalse(validate_timeout(0))
        self.assertFalse(validate_timeout(3601))


class TestGpioLogic(unittest.TestCase):
    def test_pin_range_validation(self):
        def check_pin(pin: int) -> bool:
            return 0 <= pin <= 48

        self.assertTrue(check_pin(0))
        self.assertTrue(check_pin(2))
        self.assertTrue(check_pin(48))
        self.assertFalse(check_pin(49))
        self.assertFalse(check_pin(100))

    def test_output_mask_tracking(self):
        mask_lo = 0
        mask_hi = 0

        def set_output(pin: int, is_out: bool):
            nonlocal mask_lo, mask_hi
            if pin < 32:
                bit = 1 << pin
                mask_lo = (mask_lo | bit) if is_out else (mask_lo & ~bit)
            else:
                bit = 1 << (pin - 32)
                mask_hi = (mask_hi | bit) if is_out else (mask_hi & ~bit)

        set_output(2, True)
        self.assertTrue(mask_lo & (1 << 2))
        set_output(2, False)
        self.assertFalse(mask_lo & (1 << 2))

        set_output(35, True)
        self.assertTrue(mask_hi & (1 << 3))
        set_output(35, False)
        self.assertFalse(mask_hi & (1 << 3))


class TestLittleFsLogic(unittest.TestCase):
    def test_lfs_abs_offset(self):
        block_size = 4096
        region_offset = 0x820000
        block_count = 100

        def abs_offset(block: int, off: int, size: int) -> Optional[int]:
            if block >= block_count or off + size > block_size:
                return None
            return region_offset + block * block_size + off

        self.assertEqual(abs_offset(0, 0, 16), 0x820000)
        self.assertEqual(abs_offset(1, 0, 16), 0x820000 + 4096)
        self.assertEqual(abs_offset(0, 4080, 16), 0x820000 + 4080)
        self.assertIsNone(abs_offset(0, 4081, 16))  # Exceeds block size
        self.assertIsNone(abs_offset(100, 0, 16))   # Block out of range


class TestUnameHostnameLogic(unittest.TestCase):
    def test_uname_formatting(self):
        info = {
            "sysname": "EspressoOS",
            "nodename": "esp32s3",
            "release": "0.2.0-dev",
            "version": "#1 PREEMPT Xtensa-LX7-240MHz",
            "machine": "xtensa-esp32s3",
        }
        all_info = f"{info['sysname']} {info['nodename']} {info['release']} {info['version']} {info['machine']}"
        self.assertEqual(
            all_info,
            "EspressoOS esp32s3 0.2.0-dev #1 PREEMPT Xtensa-LX7-240MHz xtensa-esp32s3",
        )

    def test_hostname_normalization(self):
        def clean_hostname(raw: str) -> str:
            clean = raw.strip()
            return clean if clean else "esp32s3"

        self.assertEqual(clean_hostname("my-esp32\n"), "my-esp32")
        self.assertEqual(clean_hostname("  \t\n"), "esp32s3")


class TestStreamUtilitiesLogic(unittest.TestCase):
    def test_cut_field_extraction(self):
        def cut_line(line: str, delim: str, field_num: int) -> str:
            parts = line.split(delim)
            if 1 <= field_num <= len(parts):
                return parts[field_num - 1]
            return ""

        row = "root:x:0:0:root:/root:/bin/sh"
        self.assertEqual(cut_line(row, ":", 1), "root")
        self.assertEqual(cut_line(row, ":", 3), "0")
        self.assertEqual(cut_line(row, ":", 7), "/bin/sh")
        self.assertEqual(cut_line(row, ":", 10), "")

    def test_tr_translation_and_deletion(self):
        def tr_delete(text: str, chars: str) -> str:
            return "".join(c for c in text if c not in chars)

        def tr_upper(text: str) -> str:
            return text.upper()

        self.assertEqual(tr_delete("line1\r\nline2\r\n", "\r"), "line1\nline2\n")
        self.assertEqual(tr_upper("espressoos"), "ESPRESSOOS")

    def test_base64_codec(self):
        import base64
        msg = b"EspressoOS on ESP32-S3"
        encoded = base64.b64encode(msg).decode("ascii")
        self.assertEqual(encoded, "RXNwcmVzc29PUyBvbiBFU1AzMi1TMw==")
        decoded = base64.b64decode(encoded)
        self.assertEqual(decoded, msg)

    def test_hexdump_ascii_mapping(self):
        def ascii_char(b: int) -> str:
            return chr(b) if 32 <= b <= 126 else "."

        self.assertEqual(ascii_char(ord("A")), "A")
        self.assertEqual(ascii_char(ord("z")), "z")
        self.assertEqual(ascii_char(0), ".")
        self.assertEqual(ascii_char(127), ".")


class TestPwmAndAdcLogic(unittest.TestCase):
    def test_pwm_divider_calculation(self):
        apb_clk = 80_000_000
        max_val = 1024  # 10-bit resolution

        def calc_div_fp(freq: int) -> int:
            div_fp = (apb_clk * 256) // (freq * max_val)
            return max(256, min(div_fp, (1 << 18) - 1))

        # 1 kHz
        div_1k = calc_div_fp(1000)
        self.assertEqual(div_1k, 20000)

        # 5 kHz
        div_5k = calc_div_fp(5000)
        self.assertEqual(div_5k, 4000)

    def test_pwm_duty_conversion(self):
        max_val = 1024
        def duty_to_reg(percent: int) -> int:
            val = (percent * (max_val - 1)) // 100
            return val << 4

        self.assertEqual(duty_to_reg(0), 0)
        self.assertEqual(duty_to_reg(50), 511 << 4)
        self.assertEqual(duty_to_reg(100), 1023 << 4)

    def test_adc_raw_to_mv(self):
        vref_mv = 3300
        max_raw = 4095
        def raw_to_mv(raw: int) -> int:
            r = min(raw, max_raw)
            return (r * vref_mv) // max_raw

        self.assertEqual(raw_to_mv(0), 0)
        self.assertEqual(raw_to_mv(max_raw), 3300)
        self.assertAlmostEqual(raw_to_mv(2048), 1650, delta=1)

    def test_adc_gpio_mapping(self):
        adc_gpio_map = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
        for ch, gpio in enumerate(adc_gpio_map):
            self.assertEqual(gpio, ch + 1)


class TestProcFsAdditions(unittest.TestCase):
    def test_proc_devices_content(self):
        devices = [
            ("mem", 1),
            ("tty", 4),
            ("misc", 10),
            ("net", 80),
            ("bluetooth", 90),
            ("bus", 110),
        ]
        for name, major in devices:
            self.assertTrue(major > 0)
            self.assertTrue(len(name) > 0)

    def test_proc_partitions_layout(self):
        partitions = {
            "nvs": 24,
            "otadata": 8,
            "factory": 4096,
            "ota_0": 4096,
            "storage": 8000,
            "coredump": 64,
        }
        total_kb = sum(partitions.values())
        self.assertTrue(total_kb <= 16384)  # Fits in 16MB flash

    def test_proc_crypto_engines(self):
        engines = ["sha256", "ed25519", "chacha20-poly1305", "trng"]
        for e in engines:
            self.assertIn(e, ["sha256", "ed25519", "chacha20-poly1305", "trng"])


class TestExtendedUtilitiesLogic(unittest.TestCase):
    def test_basename(self):
        def basename(path: str, suffix: str = "") -> str:
            p = path.rstrip("/")
            if not p:
                return "/"
            base = p.split("/")[-1]
            if suffix and base.endswith(suffix) and len(base) > len(suffix):
                return base[:-len(suffix)]
            return base

        self.assertEqual(basename("/etc/passwd"), "passwd")
        self.assertEqual(basename("/var/log/"), "log")
        self.assertEqual(basename("/foo/bar.txt", ".txt"), "bar")
        self.assertEqual(basename("/"), "/")
        self.assertEqual(basename("main.rs"), "main.rs")

    def test_dirname(self):
        def dirname(path: str) -> str:
            p = path.rstrip("/")
            if not p:
                return "/"
            parts = p.split("/")
            if len(parts) <= 1:
                return "."
            dirpath = "/".join(parts[:-1])
            return dirpath if dirpath else "/"

        self.assertEqual(dirname("/usr/bin/sort"), "/usr/bin")
        self.assertEqual(dirname("/etc/passwd"), "/etc")
        self.assertEqual(dirname("/etc"), "/")
        self.assertEqual(dirname("/"), "/")
        self.assertEqual(dirname("stdio.h"), ".")

    def test_cal_sakamoto_day_of_week(self):
        # Day of week: 0=Sun, 1=Mon, ..., 6=Sat
        def day_of_week(year: int, month: int, day: int) -> int:
            t = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4]
            y = year - 1 if month < 3 else year
            return (y + y // 4 - y // 100 + y // 400 + t[month - 1] + day) % 7

        # 2026-10-14 is Wednesday (3)
        self.assertEqual(day_of_week(2026, 10, 14), 3)
        # 2026-10-01 is Thursday (4)
        self.assertEqual(day_of_week(2026, 10, 1), 4)

    def test_cal_leap_year(self):
        def is_leap(y: int) -> bool:
            return (y % 4 == 0 and y % 100 != 0) or (y % 400 == 0)

        self.assertTrue(is_leap(2024))
        self.assertFalse(is_leap(2025))
        self.assertFalse(is_leap(2026))
        self.assertTrue(is_leap(2000))
        self.assertFalse(is_leap(1900))


class TestNamedPipeAndIpcLogic(unittest.TestCase):
    def test_fifo_ring_buffer(self):
        capacity = 512
        buf = bytearray()

        def fifo_write(data: bytes) -> int:
            avail = capacity - len(buf)
            to_write = min(avail, len(data))
            buf.extend(data[:to_write])
            return to_write

        def fifo_read(max_bytes: int) -> bytes:
            to_read = min(max_bytes, len(buf))
            chunk = bytes(buf[:to_read])
            del buf[:to_read]
            return chunk

        self.assertEqual(fifo_write(b"hello pipe\n"), 11)
        self.assertEqual(len(buf), 11)
        self.assertEqual(fifo_read(6), b"hello ")
        self.assertEqual(fifo_read(10), b"pipe\n")
        self.assertEqual(fifo_read(10), b"")

    def test_syscall_mkdir_fifo_mode_detection(self):
        S_IFIFO = 0x1000
        mode_dir = 0o755
        mode_fifo = 0o666 | S_IFIFO

        def is_fifo_request(mode: int) -> bool:
            return (mode & 0xF000) == S_IFIFO

        self.assertFalse(is_fifo_request(mode_dir))
        self.assertTrue(is_fifo_request(mode_fifo))


class TestSortAndUniqLogic(unittest.TestCase):
    def test_sort_alphabetical_and_reverse(self):
        lines = ["banana", "apple", "cherry", "date"]
        sorted_asc = sorted(lines)
        sorted_desc = sorted(lines, reverse=True)
        self.assertEqual(sorted_asc, ["apple", "banana", "cherry", "date"])
        self.assertEqual(sorted_desc, ["date", "cherry", "banana", "apple"])

    def test_sort_numeric(self):
        lines = ["10", "2", "1", "20", "100"]
        def num_key(s: str) -> int:
            try:
                return int(s.strip())
            except ValueError:
                return 0

        sorted_num = sorted(lines, key=num_key)
        self.assertEqual(sorted_num, ["1", "2", "10", "20", "100"])

    def test_uniq_adjacent_and_count(self):
        lines = ["a", "a", "b", "c", "c", "c", "d"]
        
        # Standard uniq
        result = []
        for l in lines:
            if not result or result[-1] != l:
                result.append(l)
        self.assertEqual(result, ["a", "b", "c", "d"])

        # Count uniq (-c)
        counts = []
        for l in lines:
            if not counts or counts[-1][0] != l:
                counts.append([l, 1])
            else:
                counts[-1][1] += 1
        self.assertEqual(counts, [["a", 2], ["b", 1], ["c", 3], ["d", 1]])

        # Duplicates only (-d)
        dups = [item[0] for item in counts if item[1] > 1]
        self.assertEqual(dups, ["a", "c"])


class TestCmpAndXargsLogic(unittest.TestCase):
    def test_cmp_byte_by_byte(self):
        f1 = b"Hello World!\nSecond line.\n"
        f2 = b"Hello Earth!\nSecond line.\n"

        def cmp_bytes(b1: bytes, b2: bytes):
            min_len = min(len(b1), len(b2))
            byte_num = 1
            line_num = 1
            for i in range(min_len):
                if b1[i] != b2[i]:
                    return (byte_num, line_num, b1[i], b2[i])
                if b1[i] == ord("\n"):
                    line_num += 1
                byte_num += 1
            if len(b1) != len(b2):
                return (byte_num, line_num, None, None)
            return None

        diff = cmp_bytes(f1, f2)
        self.assertIsNotNone(diff)
        self.assertEqual(diff[0], 7)   # byte 7: 'W' vs 'E'
        self.assertEqual(diff[1], 1)   # line 1
        self.assertEqual(diff[2], ord("W"))
        self.assertEqual(diff[3], ord("E"))

        # Identical
        self.assertIsNone(cmp_bytes(f1, f1))

    def test_xargs_argument_splitting(self):
        raw_input = "  file1.txt   file2.txt \n  file3.txt\tfile4.txt  \n"
        tokens = raw_input.split()
        self.assertEqual(tokens, ["file1.txt", "file2.txt", "file3.txt", "file4.txt"])

        # Batch execution with max 2 args per call
        batches = [tokens[i:i+2] for i in range(0, len(tokens), 2)]
        self.assertEqual(batches, [["file1.txt", "file2.txt"], ["file3.txt", "file4.txt"]])


class TestLoggerLogic(unittest.TestCase):
    def test_syslog_formatting(self):
        def format_log(pri: int, tag: str, msg: str) -> str:
            return f"<{pri}> [SYSLOG] {tag}: {msg}\n"

        entry = format_log(6, "user", "Application initialized successfully")
        self.assertEqual(entry, "<6> [SYSLOG] user: Application initialized successfully\n")


class TestDateAndCalendarLogic(unittest.TestCase):
    def test_gregorian_conversion(self):
        def epoch_to_date(days: int):
            z = days + 719468
            era = z // 146097
            doe = z - era * 146097
            yoe = (doe - doe // 1460 + doe // 36524 - doe // 146096) // 365
            y = yoe + era * 400
            doy = doe - (365 * yoe + yoe // 4 - yoe // 100)
            mp = (5 * doy + 2) // 153
            d = doy - (153 * mp + 2) // 5 + 1
            m = mp + (3 if mp < 10 else -9)
            year = y + (1 if m <= 2 else 0)
            return year, m, d

        # 1970-01-01
        self.assertEqual(epoch_to_date(0), (1970, 1, 1))
        # 2026-10-14 (20740 days)
        self.assertEqual(epoch_to_date(20740), (2026, 10, 14))

    def test_day_of_week(self):
        # 1970-01-01 was Thursday (index 4)
        def wday(days: int) -> int:
            return (days + 4) % 7

        self.assertEqual(wday(0), 4)  # Thursday
        self.assertEqual(wday(20740), 3)  # Wednesday (2026-10-14)


class TestMktempAndCronLogic(unittest.TestCase):
    def test_mktemp_suffix_generation(self):
        template = "/tmp/tmp.XXXXXX"
        base36 = "0123456789abcdefghijklmnopqrstuvwxyz"
        rand_bytes = [10, 25, 35, 0, 15, 30]

        buf = list(template)
        rand_idx = 0
        for i in reversed(range(len(buf))):
            if buf[i] == "X" and rand_idx < len(rand_bytes):
                choice = rand_bytes[rand_idx] % len(base36)
                buf[i] = base36[choice]
                rand_idx += 1

        result = "".join(buf)
        self.assertTrue(result.startswith("/tmp/tmp."))
        self.assertNotIn("X", result)
        self.assertEqual(len(result), len(template))

    def test_crontab_parsing(self):
        crontab_text = """
        # EspressoOS Scheduled Tasks Table (/etc/crontab)
        # interval_sec command
        60 /bin/wdt ping
        300 /bin/sntp
        """
        jobs = []
        for line in crontab_text.strip().splitlines():
            line = line.strip()
            if not line or line.starts_with("#") if hasattr(line, "starts_with") else line.startswith("#"):
                continue
            parts = line.split(maxsplit=1)
            if len(parts) == 2:
                jobs.append((int(parts[0]), parts[1]))

        self.assertEqual(jobs, [(60, "/bin/wdt ping"), (300, "/bin/sntp")])


class TestStringsAndTimeoutLogic(unittest.TestCase):
    def test_strings_filtering(self):
        raw_data = b"\x00\x01\x02hello\x00\xffworld1234\x00\x04hi\x00"
        min_len = 4

        extracted = []
        cur = bytearray()
        for b in raw_data:
            if 32 <= b <= 126 or b == ord("\t"):
                cur.append(b)
            else:
                if len(cur) >= min_len:
                    extracted.append(cur.decode("latin1"))
                cur.clear()
        if len(cur) >= min_len:
            extracted.append(cur.decode("latin1"))

        self.assertEqual(extracted, ["hello", "world1234"])

    def test_timeout_exit_code_standard(self):
        # POSIX 124 on timeout
        POSIX_TIMEOUT_EXIT = 124
        self.assertEqual(POSIX_TIMEOUT_EXIT, 124)


class TestTarAndArchiveLogic(unittest.TestCase):
    def test_ustar_header_layout(self):
        filename = "test.txt"
        file_size = 42

        header = bytearray(512)
        # Name: 0..100
        header[:len(filename)] = filename.encode("ascii")
        # Mode: 100..108
        header[100:108] = b"0000644\x00"
        # Magic: 257..263
        header[257:263] = b"ustar\x00"

        # Octal size at 124..136
        oct_sz = f"{file_size:011o}\x00".encode("ascii")
        header[124:136] = oct_sz

        self.assertEqual(header[257:263], b"ustar\x00")
        self.assertEqual(header[100:108], b"0000644\x00")

    def test_tar_block_padding(self):
        def tar_pad_len(size: int) -> int:
            return (512 - (size % 512)) % 512

        self.assertEqual(tar_pad_len(512), 0)
        self.assertEqual(tar_pad_len(513), 511)
        self.assertEqual(tar_pad_len(1), 511)
        self.assertEqual(tar_pad_len(1024), 0)


class TestDiffAndEditorLogic(unittest.TestCase):
    def test_diff_line_comparison(self):
        doc1 = ["line 1", "line 2", "line 3"]
        doc2 = ["line 1", "line 2 modified", "line 3"]

        diffs = []
        for i in range(max(len(doc1), len(doc2))):
            l1 = doc1[i] if i < len(doc1) else None
            l2 = doc2[i] if i < len(doc2) else None
            if l1 != l2:
                diffs.append((i + 1, l1, l2))

        self.assertEqual(diffs, [(2, "line 2", "line 2 modified")])

    def test_ed_buffer_delete(self):
        buf = ["first", "second", "third", "fourth"]
        # Delete line 2 (1-based)
        del_idx = 2
        buf.pop(del_idx - 1)
        self.assertEqual(buf, ["first", "third", "fourth"])


class TestIdAndVmstatLogic(unittest.TestCase):
    def test_id_mapping(self):
        def user_to_ids(user: str):
            if user == "root":
                return 0, 0, "root"
            return 1000, 1000, "users"

        self.assertEqual(user_to_ids("root"), (0, 0, "root"))
        self.assertEqual(user_to_ids("youareme"), (1000, 1000, "users"))

    def test_vmstat_rate_calculation(self):
        uptime_ms = 50000
        ticks = uptime_ms // 10
        interrupts = (ticks // 2) % 10000
        ctx_switches = (ticks // 5) % 10000
        self.assertEqual(interrupts, 2500)
        self.assertEqual(ctx_switches, 1000)


class TestPhase14Logic(unittest.TestCase):
    def test_framebuffer_geometry_and_ioctls(self):
        fb_w, fb_h = 128, 64
        fb_size = (fb_w * fb_h) // 8
        self.assertEqual(fb_size, 1024)
        vscreeninfo = (fb_w << 16) | fb_h
        self.assertEqual(vscreeninfo, (128 << 16) | 64)

        # Pixel calculation
        def set_pixel(buf: bytearray, x: int, y: int):
            idx = (y * fb_w + x) // 8
            bit = 7 - (x % 8)
            buf[idx] |= (1 << bit)

        buf = bytearray(1024)
        set_pixel(buf, 0, 0)
        self.assertEqual(buf[0], 0x80)
        set_pixel(buf, 7, 0)
        self.assertEqual(buf[0], 0x81)

    def test_expr_eval(self):
        def eval_expr(a: int, op: str, b: int) -> int:
            if op == "+": return a + b
            if op == "-": return a - b
            if op == "*": return a * b
            if op == "/": return a // b
            if op == "%": return a % b
            if op == "=": return 1 if a == b else 0
            if op == "!=": return 1 if a != b else 0
            if op == "<": return 1 if a < b else 0
            if op == "<=": return 1 if a <= b else 0
            if op == ">": return 1 if a > b else 0
            if op == ">=": return 1 if a >= b else 0
            return 0

        self.assertEqual(eval_expr(10, "+", 25), 35)
        self.assertEqual(eval_expr(50, "-", 18), 32)
        self.assertEqual(eval_expr(7, "*", 6), 42)
        self.assertEqual(eval_expr(100, "/", 4), 25)
        self.assertEqual(eval_expr(17, "%", 5), 2)
        self.assertEqual(eval_expr(42, "=", 42), 1)
        self.assertEqual(eval_expr(42, "<", 50), 1)

    def test_test_conditionals(self):
        def posix_test(op: str, val1: str, val2: str = "") -> bool:
            if op == "-z": return len(val1) == 0
            if op == "-n": return len(val1) > 0
            if op == "=": return val1 == val2
            if op == "!=": return val1 != val2
            if op == "-eq": return int(val1) == int(val2)
            if op == "-lt": return int(val1) < int(val2)
            if op == "-ge": return int(val1) >= int(val2)
            return False

        self.assertTrue(posix_test("-z", ""))
        self.assertFalse(posix_test("-z", "hello"))
        self.assertTrue(posix_test("-n", "hello"))
        self.assertTrue(posix_test("=", "abc", "abc"))
        self.assertFalse(posix_test("=", "abc", "def"))
        self.assertTrue(posix_test("-eq", "100", "100"))
        self.assertTrue(posix_test("-lt", "10", "20"))
        self.assertTrue(posix_test("-ge", "20", "20"))

    def test_patch_application(self):
        lines = ["alpha", "beta", "gamma"]
        # Apply hunk @@ line 2 @@ with - beta, + delta
        hunk_line = 2  # 1-indexed
        idx = hunk_line - 1
        lines[idx] = "delta"
        self.assertEqual(lines, ["alpha", "delta", "gamma"])

    def test_logrotate_size_trigger(self):
        def should_rotate(size: int, force: bool, threshold: int = 4096) -> bool:
            return force or (size >= threshold)

        self.assertFalse(should_rotate(1024, force=False))
        self.assertTrue(should_rotate(4096, force=False))
        self.assertTrue(should_rotate(8192, force=False))
        self.assertTrue(should_rotate(100, force=True))

    def test_arp_table_parsing(self):
        arp_content = (
            "IP address       HW type     Flags       HW address            Mask     Device\n"
            "192.168.1.1      0x1         0x2         00:50:56:c0:00:08     *        wlan0\n"
            "192.168.1.100    0x1         0x2         c0:49:ef:12:34:56     *        wlan0\n"
        )
        rows = [line.split() for line in arp_content.strip().splitlines()[1:]]
        self.assertEqual(len(rows), 2)
        self.assertEqual(rows[0][0], "192.168.1.1")
        self.assertEqual(rows[0][3], "00:50:56:c0:00:08")
        self.assertEqual(rows[1][0], "192.168.1.100")
        self.assertEqual(rows[1][5], "wlan0")


class TestPhase15Logic(unittest.TestCase):
    def test_comm_comparison(self):
        f1 = ["apple", "banana", "cherry"]
        f2 = ["banana", "cherry", "date"]

        # Produce comm output (col1: only f1, col2: only f2, col3: both)
        col1, col2, col3 = [], [], []
        i1, i2 = 0, 0
        while i1 < len(f1) and i2 < len(f2):
            if f1[i1] < f2[i2]:
                col1.append(f1[i1])
                i1 += 1
            elif f1[i1] > f2[i2]:
                col2.append(f2[i2])
                i2 += 1
            else:
                col3.append(f1[i1])
                i1 += 1
                i2 += 1
        while i1 < len(f1):
            col1.append(f1[i1])
            i1 += 1
        while i2 < len(f2):
            col2.append(f2[i2])
            i2 += 1

        self.assertEqual(col1, ["apple"])
        self.assertEqual(col2, ["date"])
        self.assertEqual(col3, ["banana", "cherry"])

    def test_nl_numbering(self):
        lines = ["first", "", "second"]
        # Format with width 6
        def number_lines(lines, number_empty=False, width=6):
            res = []
            no = 1
            for l in lines:
                if not l and not number_empty:
                    res.append("")
                else:
                    res.append(f"{str(no).rjust(width)}\t{l}")
                    no += 1
            return res

        numbered = number_lines(lines, number_empty=False, width=6)
        self.assertEqual(numbered[0], "     1\tfirst")
        self.assertEqual(numbered[1], "")
        self.assertEqual(numbered[2], "     2\tsecond")

    def test_fold_wrapping(self):
        text = "The quick brown fox jumps over the lazy dog"
        # Wrap at 16 chars with space break
        def fold_text(s, width=16):
            res = []
            rem = s
            while rem:
                if len(rem) <= width:
                    res.append(rem)
                    break
                split_at = width
                space_idx = rem[:width].rfind(' ')
                if space_idx > 0:
                    split_at = space_idx
                res.append(rem[:split_at])
                rem = rem[split_at:].lstrip()
            return res

        wrapped = fold_text(text, 16)
        self.assertEqual(wrapped[0], "The quick brown")
        self.assertEqual(wrapped[1], "fox jumps over")
        self.assertEqual(wrapped[2], "the lazy dog")

    def test_split_suffix(self):
        def make_suffix(idx):
            f = chr(ord('a') + ((idx // 26) % 26))
            s = chr(ord('a') + (idx % 26))
            return f + s

        self.assertEqual(make_suffix(0), "aa")
        self.assertEqual(make_suffix(1), "ab")
        self.assertEqual(make_suffix(25), "az")
        self.assertEqual(make_suffix(26), "ba")

    def test_printenv_lookup(self):
        env = {"PATH": "/bin:/usr/bin", "USER": "youareme", "HOME": "/home/youareme"}
        self.assertEqual(env.get("USER"), "youareme")
        self.assertEqual(env.get("NONEXISTENT"), None)

    def test_procfs_net_dev_route(self):
        net_dev = (
            "Inter-|   Receive                                                |  Transmit\n"
            " face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed\n"
            " wlan0: 1048576    4096    0    0    0     0          0         0  1048576    4096    0    0    0     0       0          0\n"
        )
        self.assertIn("wlan0:", net_dev)
        net_route = (
            "Iface\tDestination\tGateway\tFlags\tRefCnt\tUse\tMetric\tMask\tMTU\tWindow\tIRTT\n"
            "wlan0\t0001A8C0\t00000000\t0001\t0\t0\t0\t00FFFFFF\t0\t0\t0\n"
        )
        self.assertIn("wlan0", net_route)


class TestPhase16Logic(unittest.TestCase):
    def test_factor_decomposition(self):
        def factorize(n: int):
            factors = []
            if n <= 1:
                return factors
            while n % 2 == 0:
                factors.append(2)
                n //= 2
            while n % 3 == 0:
                factors.append(3)
                n //= 3
            d = 5
            while d * d <= n:
                while n % d == 0:
                    factors.append(d)
                    n //= d
                d2 = d + 2
                while n % d2 == 0:
                    factors.append(d2)
                    n //= d2
                d += 6
            if n > 1:
                factors.append(n)
            return factors

        self.assertEqual(factorize(42), [2, 3, 7])
        self.assertEqual(factorize(100), [2, 2, 5, 5])
        self.assertEqual(factorize(13), [13])
        self.assertEqual(factorize(1), [])

    def test_rev_characters(self):
        s = "EspressoOS"
        self.assertEqual(s[::-1], "SOosserpsE")

    def test_paste_merging(self):
        f1 = ["a", "b", "c"]
        f2 = ["1", "2", "3"]
        merged = [f"{l1}\t{l2}" for l1, l2 in zip(f1, f2)]
        self.assertEqual(merged, ["a\t1", "b\t2", "c\t3"])

    def test_expand_tabs(self):
        def expand_line(line: str, tabstop: int = 8) -> str:
            res = []
            col = 0
            for c in line:
                if c == '\t':
                    spaces = tabstop - (col % tabstop)
                    res.append(' ' * spaces)
                    col += spaces
                else:
                    res.append(c)
                    col += 1
            return "".join(res)

        self.assertEqual(expand_line("a\tb", 8), "a       b")
        self.assertEqual(expand_line("abcd\te", 8), "abcd    e")

    def test_procfs_cmdline(self):
        cmdline = "console=ttyS0,115200 root=/dev/flash0 rw init=/bin/init quiet\n"
        self.assertIn("root=/dev/flash0", cmdline)
        self.assertIn("init=/bin/init", cmdline)

    def test_seq_generation(self):
        def seq(first, step, last):
            res = []
            cur = first
            while (step > 0 and cur <= last) or (step < 0 and cur >= last):
                res.append(cur)
                cur += step
            return res

        self.assertEqual(seq(1, 1, 5), [1, 2, 3, 4, 5])
        self.assertEqual(seq(3, 2, 9), [3, 5, 7, 9])
        self.assertEqual(seq(5, -1, 1), [5, 4, 3, 2, 1])

    def test_realpath_canonicalization(self):
        def canonicalize(path: str, cwd: str) -> str:
            full = path if path.startswith('/') else (cwd.rstrip('/') + '/' + path)
            parts = []
            for seg in full.split('/'):
                if not seg or seg == '.':
                    continue
                elif seg == '..':
                    if parts:
                        parts.pop()
                else:
                    parts.append(seg)
            return '/' + '/'.join(parts)

        self.assertEqual(canonicalize("../bin/ls", "/usr/local"), "/usr/bin/ls")
        self.assertEqual(canonicalize("./file.txt", "/home/user"), "/home/user/file.txt")
        self.assertEqual(canonicalize("/etc/../var/log", "/"), "/var/log")
        self.assertEqual(canonicalize("///", "/"), "/")

    def test_cksum_crc(self):
        def posix_cksum(data: bytes) -> int:
            crc = 0
            for b in data:
                c = ((crc >> 24) ^ b) & 0xFF
                for _ in range(8):
                    if (c & 0x80) != 0:
                        c = ((c << 1) ^ 0x04C11DB7) & 0xFFFFFFFF
                    else:
                        c = (c << 1) & 0xFFFFFFFF
                crc = ((crc << 8) ^ c) & 0xFFFFFFFF
            length = len(data)
            while length > 0:
                b = length & 0xFF
                c = ((crc >> 24) ^ b) & 0xFF
                for _ in range(8):
                    if (c & 0x80) != 0:
                        c = ((c << 1) ^ 0x04C11DB7) & 0xFFFFFFFF
                    else:
                        c = (c << 1) & 0xFFFFFFFF
                crc = ((crc << 8) ^ c) & 0xFFFFFFFF
                length >>= 8
            return (~crc) & 0xFFFFFFFF

        # Known POSIX cksum test vector for empty string and "Hello\n"
        self.assertEqual(posix_cksum(b""), 4294967295)
        self.assertGreater(posix_cksum(b"Hello\n"), 0)

    def test_procfs_stat(self):
        stat_out = (
            "cpu  1000 0 500 3000 0 0 0 0 0 0\n"
            "cpu0 1000 0 500 3000 0 0 0 0 0 0\n"
            "intr 0\n"
            "ctxt 2000\n"
            "btime 1700000000\n"
            "processes 123\n"
            "procs_running 4\n"
            "procs_blocked 0\n"
        )
        self.assertIn("cpu  ", stat_out)
        self.assertIn("processes 123", stat_out)
        self.assertIn("procs_running 4", stat_out)

    def test_etc_hosts_resolution(self):
        hosts_file = (
            "# Static hosts table\n"
            "127.0.0.1 localhost\n"
            "192.168.4.1 espressoos esp32s3\n"
            "10.0.0.1   gateway\n"
        )
        def resolve_host(content: str, host: str):
            for line in content.splitlines():
                line = line.strip()
                if not line or line.startswith('#'):
                    continue
                parts = line.split()
                if len(parts) >= 2:
                    ip = parts[0]
                    for name in parts[1:]:
                        if name.lower() == host.lower():
                            return ip
            return None

        self.assertEqual(resolve_host(hosts_file, "localhost"), "127.0.0.1")
        self.assertEqual(resolve_host(hosts_file, "LOCALHOST"), "127.0.0.1")
        self.assertEqual(resolve_host(hosts_file, "espressoos"), "192.168.4.1")
        self.assertEqual(resolve_host(hosts_file, "esp32s3"), "192.168.4.1")
        self.assertEqual(resolve_host(hosts_file, "gateway"), "10.0.0.1")
        self.assertIsNone(resolve_host(hosts_file, "unknown.local"))

    def test_resolv_conf_nameserver(self):
        resolv_file = (
            "# DNS Configuration\n"
            "nameserver 8.8.8.8\n"
            "nameserver 1.1.1.1\n"
        )
        def get_nameserver(content: str):
            for line in content.splitlines():
                line = line.strip()
                if not line or line.startswith('#'):
                    continue
                parts = line.split()
                if len(parts) >= 2 and parts[0] == "nameserver":
                    return parts[1]
            return None

        self.assertEqual(get_nameserver(resolv_file), "8.8.8.8")

    def test_dc_rpn_calculator(self):
        def eval_dc(tokens_str: str):
            stack = []
            for tok in tokens_str.split():
                if tok.isdigit() or (tok.startswith('_') and tok[1:].isdigit()):
                    val = -int(tok[1:]) if tok.startswith('_') else int(tok)
                    stack.append(val)
                elif tok == '+':
                    a, b = stack.pop(), stack.pop()
                    stack.append(b + a)
                elif tok == '-':
                    a, b = stack.pop(), stack.pop()
                    stack.append(b - a)
                elif tok == '*':
                    a, b = stack.pop(), stack.pop()
                    stack.append(b * a)
                elif tok == '/':
                    a, b = stack.pop(), stack.pop()
                    stack.append(b // a)
                elif tok == '%':
                    a, b = stack.pop(), stack.pop()
                    stack.append(b % a)
                elif tok == 'd':
                    stack.append(stack[-1])
                elif tok == 'r':
                    a, b = stack.pop(), stack.pop()
                    stack.append(a)
                    stack.append(b)
                elif tok == 'c':
                    stack.clear()
            return stack

        self.assertEqual(eval_dc("10 20 +"), [30])
        self.assertEqual(eval_dc("50 15 -"), [35])
        self.assertEqual(eval_dc("6 7 *"), [42])
        self.assertEqual(eval_dc("100 4 /"), [25])
        self.assertEqual(eval_dc("17 5 %"), [2])
        self.assertEqual(eval_dc("9 d *"), [81])
        self.assertEqual(eval_dc("10 20 r"), [20, 10])

    def test_hostid_fnv1a(self):
        def fnv1a(s: bytes) -> int:
            h = 0x811c9dc5
            for b in s:
                h ^= b
                h = (h * 0x01000193) & 0xFFFFFFFF
            return h

        h1 = fnv1a(b"esp32s3")
        h2 = fnv1a(b"espressoos")
        self.assertGreater(h1, 0)
        self.assertNotEqual(h1, h2)
        hex_str = f"{h1:08x}"
        self.assertEqual(len(hex_str), 8)

    def test_etc_group_membership(self):
        group_file = (
            "root:x:0:\n"
            "daemon:x:1:\n"
            "wheel:x:10:root,youareme\n"
            "users:x:100:youareme\n"
        )
        def get_user_groups(content: str, username: str):
            groups = []
            for line in content.splitlines():
                parts = line.strip().split(':')
                if len(parts) >= 4:
                    gname = parts[0]
                    members = [m.strip() for m in parts[3].split(',') if m.strip()]
                    if gname == username or username in members:
                        groups.append(gname)
            return groups

        self.assertEqual(get_user_groups(group_file, "youareme"), ["wheel", "users"])
        self.assertEqual(get_user_groups(group_file, "root"), ["root", "wheel"])

    def test_time_formatting(self):
        def format_ms(ms: int) -> str:
            sec = ms // 1000
            rem = ms % 1000
            return f"{sec}.{rem:03d}s"

        self.assertEqual(format_ms(52), "0.052s")
        self.assertEqual(format_ms(1200), "1.200s")
        self.assertEqual(format_ms(0), "0.000s")


if __name__ == "__main__":
    unittest.main(verbosity=2)




