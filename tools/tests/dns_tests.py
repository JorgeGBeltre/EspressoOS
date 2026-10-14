#!/usr/bin/env python3
"""Tests de codificación y decodificación de paquetes DNS (RFC 1035).

Espeja la lógica de la herramienta de espacio de usuario `nslookup`
(userland/apps/src/bin/nslookup.rs).
"""
from __future__ import annotations

import struct
import unittest


def encode_dns_query(host: str, tx_id: int = 0x1234) -> bytes:
    header = struct.pack(">HHHHHH", tx_id, 0x0100, 1, 0, 0, 0)
    qname = bytearray()
    for part in host.split("."):
        if not part:
            continue
        p_bytes = part.encode("ascii")
        qname.append(len(p_bytes))
        qname.extend(p_bytes)
    qname.append(0)  # Root label
    question = bytes(qname) + struct.pack(">HH", 1, 1)  # TYPE A (1), CLASS IN (1)
    return header + question


def parse_dns_response(pkt: bytes) -> tuple[int, list[str]]:
    if len(pkt) < 12:
        raise ValueError("Packet too short")
    tx_id, flags, qdcount, ancount, _, _ = struct.unpack(">HHHHHH", pkt[:12])
    rcode = flags & 0x0F
    if rcode != 0:
        return rcode, []

    # Skip question section
    idx = 12
    while idx < len(pkt) and pkt[idx] != 0:
        if pkt[idx] & 0xC0 == 0xC0:
            idx += 2
            break
        idx += 1 + pkt[idx]
    if idx < len(pkt) and pkt[idx] == 0:
        idx += 1
    idx += 4  # Skip QTYPE and QCLASS

    # Parse answers
    ips = []
    for _ in range(ancount):
        if idx >= len(pkt):
            break
        # Skip NAME
        if pkt[idx] & 0xC0 == 0xC0:
            idx += 2
        else:
            while idx < len(pkt) and pkt[idx] != 0:
                idx += 1 + pkt[idx]
            if idx < len(pkt) and pkt[idx] == 0:
                idx += 1
        if idx + 10 > len(pkt):
            break
        atype, aclass, ttl, rdlen = struct.unpack(">HHIH", pkt[idx : idx + 10])
        idx += 10
        if atype == 1 and rdlen == 4 and idx + 4 <= len(pkt):
            ip_bytes = pkt[idx : idx + 4]
            ips.append(f"{ip_bytes[0]}.{ip_bytes[1]}.{ip_bytes[2]}.{ip_bytes[3]}")
        idx += rdlen

    return rcode, ips


class DnsProtoTests(unittest.TestCase):
    def test_encode_query_labels(self):
        query = encode_dns_query("example.com", 0xABCD)
        self.assertEqual(query[:2], b"\xab\xcd")
        self.assertEqual(query[2:4], b"\x01\x00")  # Standard query, RD=1
        self.assertEqual(query[4:6], b"\x00\x01")  # QDCOUNT = 1
        # QNAME: \x07example\x03com\x00
        expected_qname = b"\x07example\x03com\x00"
        self.assertIn(expected_qname, query)
        # Trailing QTYPE=A(1), QCLASS=IN(1)
        self.assertTrue(query.endswith(b"\x00\x01\x00\x01"))

    def test_parse_response_with_a_record(self):
        query = encode_dns_query("test.local", 0x1234)
        # Mock response: copy query, set QR=1, ANCOUNT=1, append A record
        resp_header = struct.pack(">HHHHHH", 0x1234, 0x8180, 1, 1, 0, 0)
        # Answer: compression pointer to question (0xC00C), TYPE=1, CLASS=1, TTL=60, RDLEN=4, IP=192.168.1.50
        answer = b"\xc0\x0c" + struct.pack(">HHIH", 1, 1, 60, 4) + bytes([192, 168, 1, 50])
        mock_packet = resp_header + query[12:] + answer

        rcode, ips = parse_dns_response(mock_packet)
        self.assertEqual(rcode, 0)
        self.assertEqual(ips, ["192.168.1.50"])

    def test_parse_nxdomain(self):
        # NXDOMAIN response (RCODE = 3)
        mock_packet = struct.pack(">HHHHHH", 0x1234, 0x8183, 1, 0, 0, 0) + b"\x04fail\x00\x00\x01\x00\x01"
        rcode, ips = parse_dns_response(mock_packet)
        self.assertEqual(rcode, 3)
        self.assertEqual(len(ips), 0)


if __name__ == "__main__":
    unittest.main()
