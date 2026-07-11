#!/usr/bin/env python3
"""Genera la tabla de particiones binaria de esp32s3-os a partir de un CSV.

Produce el formato de tabla de particiones de Espressif (entradas de 32 bytes
con magic 0xAA50 + entrada MD5 final), compatible con la ROM del ESP32-S3 y con
espflash. Ejecutar:

    python tools/partition-gen/partition_gen.py [entrada.csv] [salida.bin]

Por defecto: partitions.csv (raíz del repo) -> partitions.bin
"""
from __future__ import annotations

import hashlib
import os
import struct
import sys

MAGIC_BYTES = b"\xAA\x50"
MD5_BEGIN = b"\xEB\xEB" + b"\xFF" * 14
TABLE_SIZE = 0xC00  # 3072 bytes reservados para la tabla
APP_ALIGN = 0x10000  # las particiones app se alinean a 64 KB

TYPES = {"app": 0x00, "data": 0x01}

SUBTYPES = {
    "app": {
        "factory": 0x00,
        **{f"ota_{i}": 0x10 + i for i in range(16)},
        "test": 0x20,
    },
    "data": {
        "ota": 0x00,
        "phy": 0x01,
        "nvs": 0x02,
        "coredump": 0x03,
        "nvs_keys": 0x04,
        "efuse": 0x05,
        "undefined": 0x06,
        "fat": 0x81,
        "spiffs": 0x82,
        "littlefs": 0x83,
    },
}


def parse_size(tok: str) -> int:
    tok = tok.strip()
    if tok.lower().endswith("k"):
        return int(tok[:-1], 0) * 1024
    if tok.lower().endswith("m"):
        return int(tok[:-1], 0) * 1024 * 1024
    return int(tok, 0)


def main() -> int:
    here = os.path.dirname(os.path.abspath(__file__))
    root = os.path.abspath(os.path.join(here, "..", ".."))
    csv_path = sys.argv[1] if len(sys.argv) > 1 else os.path.join(root, "partitions.csv")
    out_path = sys.argv[2] if len(sys.argv) > 2 else os.path.join(root, "partitions.bin")

    entries = bytearray()
    rows = []
    with open(csv_path, "r", encoding="utf-8") as fh:
        for lineno, raw in enumerate(fh, 1):
            line = raw.split("#", 1)[0].strip()
            if not line:
                continue
            parts = [p.strip() for p in line.split(",")]
            if len(parts) < 5:
                print(f"error {csv_path}:{lineno}: se esperan >=5 columnas", file=sys.stderr)
                return 1
            name, ptype, subtype, offset, size = parts[:5]
            flags = parts[5] if len(parts) > 5 and parts[5] else "0"

            if ptype not in TYPES:
                print(f"error {csv_path}:{lineno}: tipo desconocido '{ptype}'", file=sys.stderr)
                return 1
            if subtype not in SUBTYPES[ptype]:
                print(f"error {csv_path}:{lineno}: subtipo '{subtype}' inválido para '{ptype}'", file=sys.stderr)
                return 1

            off = parse_size(offset)
            sz = parse_size(size)
            if ptype == "app" and off % APP_ALIGN != 0:
                print(f"error {csv_path}:{lineno}: '{name}' (app) debe alinearse a 0x{APP_ALIGN:X}", file=sys.stderr)
                return 1
            if len(name.encode()) > 16:
                print(f"error {csv_path}:{lineno}: etiqueta '{name}' supera 16 bytes", file=sys.stderr)
                return 1

            rows.append((name, off, sz))
            entries += struct.pack(
                "<2sBBLL16sL",
                MAGIC_BYTES,
                TYPES[ptype],
                SUBTYPES[ptype][subtype],
                off,
                sz,
                name.encode(),
                int(flags, 0),
            )

    # Validación de solapes.
    ordered = sorted(rows, key=lambda r: r[1])
    for (n1, o1, s1), (n2, o2, _s2) in zip(ordered, ordered[1:]):
        if o1 + s1 > o2:
            print(f"error: '{n1}' se solapa con '{n2}'", file=sys.stderr)
            return 1

    # Entrada MD5 sobre las entradas previas.
    entries += MD5_BEGIN + hashlib.md5(entries).digest()

    if len(entries) > TABLE_SIZE:
        print(f"error: tabla ({len(entries)} B) excede {TABLE_SIZE} B", file=sys.stderr)
        return 1
    entries += b"\xFF" * (TABLE_SIZE - len(entries))

    with open(out_path, "wb") as fh:
        fh.write(entries)

    print(f"OK  {len(rows)} particiones -> {out_path} ({TABLE_SIZE} bytes)")
    for name, off, sz in rows:
        print(f"    {name:<10} @ 0x{off:08X}  {sz // 1024:>6} KB")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
