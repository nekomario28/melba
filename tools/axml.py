#!/usr/bin/env python3
"""Minimal binary AndroidManifest.xml (AXML) reader.

Why this exists: the host has no aapt2 / build-tools, so `apkanalyzer manifest
print` cannot run, and the REA Android provider needs a caller-supplied jadx
JAR. A ~120-line pure-Python parser answers the questions this project asks of
a manifest (package id, permissions, exported components, intent filters)
without any of those, and prints nothing it did not read.

Usage:
    python3 tools/axml.py base.apk [--services] [--permissions] [--all]

Exit codes: 0 manifest read, 2 no AndroidManifest.xml member, 3 parse error.
Only reads; never writes, never executes the artifact.
"""
from __future__ import annotations

import sys
import zipfile
import xml.etree.ElementTree as ET  # noqa: F401  (kept for future XML fallback)

# AXML chunk types
RES_XML_TYPE = 0x0003
RES_STRING_POOL_TYPE = 0x0001
RES_XML_START_NAMESPACE_TYPE = 0x0100
RES_XML_END_NAMESPACE_TYPE = 0x0101
RES_XML_START_ELEMENT_TYPE = 0x0102
RES_XML_END_ELEMENT_TYPE = 0x0103
RES_XML_CDATA_TYPE = 0x0104
RES_XML_RESOURCE_MAP_TYPE = 0x0180

TYPE_NULL = 0x00
TYPE_STRING = 0x03
TYPE_INT_DEC = 0x10
TYPE_INT_HEX = 0x11
TYPE_INT_BOOLEAN = 0x12

# Attribute resource ids we care about (framework attrs)
ATTR_NAME = "name"
ATTR_PERMISSION = "permission"
ATTR_EXPORTED = "exported"
ATTR_ENABLED = "enabled"


class AxmlError(Exception):
    pass


def _u8(b, o):
    if o >= len(b):
        raise AxmlError("truncated")
    return b[o]


def _u16(b, o):
    if o + 2 > len(b):
        raise AxmlError("truncated")
    return b[o] | (b[o + 1] << 8)


def _i32(b, o):
    if o + 4 > len(b):
        raise AxmlError("truncated")
    return int.from_bytes(b[o : o + 4], "little", signed=True)


def _read_string_pool(b, off):
    """Return (strings, header_size, chunk_size)."""
    if _u16(b, off + 0) != RES_STRING_POOL_TYPE:
        raise AxmlError("bad string pool chunk")
    header_size = _u16(b, off + 2)
    chunk_size = _i32(b, off + 4)
    string_count = _i32(b, off + 8)
    # flags at off+16: bit 8 = UTF-8
    utf8 = bool(_i32(b, off + 16) & (1 << 8))
    strings_start = _i32(b, off + 20)
    offsets_start = off + header_size
    strings = []
    for i in range(string_count):
        so = offsets_start + i * 4
        str_off = off + strings_start + _i32(b, so)
        if utf8:
            # u16len (chars) then u8len (bytes); each is 1-2 bytes — a set
            # high bit on the first byte means a second byte follows
            u16len = _u8(b, str_off)
            p = str_off + 1
            if u16len & 0x80:
                u16len = ((u16len & 0x7F) << 8) | _u8(b, p)
                p += 1
            n = _u8(b, p)
            p += 1
            if n & 0x80:
                n = ((n & 0x7F) << 8) | _u8(b, p)
                p += 1
            raw = b[p : p + n]
            strings.append(raw.decode("utf-8", "replace"))
        else:
            n = _u16(b, str_off)
            p = str_off + 2
            if n & 0x8000:
                n = ((n & 0x7FFF) << 16) | _u16(b, p)
                p += 2
            strings.append(b[p : p + n * 2].decode("utf-16-le", "replace"))
    return strings, header_size, chunk_size


def _dimension(b, ao, attr_size):
    """Decode one ResXMLTree_attribute's typed value.

    Layout (attribute-relative): ns(4) name(4) rawValue(4) typedValue.
    Res_value = size(2) res0(1) dataType(1) data(4), so dataType sits at
    ao+15 and the 4-byte payload at ao+16.
    """
    if attr_size < 20 or ao + 20 > len(b):
        return None
    data_type = b[ao + 15]
    raw = int.from_bytes(b[ao + 16 : ao + 20], "little")
    if data_type == TYPE_STRING:
        return ("string", raw)
    if data_type == TYPE_INT_BOOLEAN:
        return bool(raw)
    if data_type in (TYPE_INT_DEC, TYPE_INT_HEX):
        return raw
    if data_type == TYPE_NULL:
        return None
    return ("raw", data_type, raw)


def parse_manifest(blob: bytes):
    """Parse the AXML blob into nested dicts/lists."""
    if _u16(blob, 0) != RES_XML_TYPE:
        raise AxmlError("not an AXML file")
    strings = []
    resource_ids = []
    pos = 8
    root = {"tag": "manifest", "attrs": {}, "children": []}
    stack = [root]
    while pos < len(blob):
        ctype = _u16(blob, pos)
        header_size = _u16(blob, pos + 2)
        chunk_size = _i32(blob, pos + 4)
        if chunk_size <= 0:
            break
        body = blob[pos : pos + chunk_size]
        if ctype == RES_STRING_POOL_TYPE:
            strings, _, _ = _read_string_pool(blob, pos)
        elif ctype == RES_XML_RESOURCE_MAP_TYPE:
            n = (chunk_size - header_size) // 4
            resource_ids = [_i32(blob, pos + header_size + i * 4) for i in range(n)]
        elif ctype == RES_XML_START_ELEMENT_TYPE:
            # node header (16B) then attrExt: ns@16 name@20
            # attributeStart@24 attributeSize@26 attributeCount@28
            name_idx = _i32(body, 20)
            attr_start = _u16(body, 24)
            attr_size = _u16(body, 26)
            attr_count = _u16(body, 28)
            tag = strings[name_idx] if name_idx < len(strings) else "?"
            attrs = {}
            for i in range(attr_count):
                ao = 16 + attr_start + i * attr_size
                ns = _i32(body, ao + 0)
                aname = _i32(body, ao + 4)
                raw_val = _i32(body, ao + 8)
                typed = _i32(body, ao + 12)
                value = _dimension(body, ao, attr_size)
                # attr names are string-pool indexes; the resource map maps
                # attribute order -> framework res id, not to a name we can
                # print, so unmapped names fall back to their pool index
                if 0 <= aname < len(strings):
                    key = strings[aname]
                else:
                    key = f"attr#{aname}"
                if isinstance(value, tuple) and value and value[0] == "string":
                    idx = value[1]
                    val = strings[idx] if 0 <= idx < len(strings) else None
                else:
                    val = value
                if val is None and raw_val > 0 and raw_val < len(strings):
                    val = strings[raw_val]  # fallback to raw string
                attrs[key] = val
            node = {"tag": tag, "attrs": attrs, "children": []}
            stack[-1]["children"].append(node)
            stack.append(node)
        elif ctype == RES_XML_END_ELEMENT_TYPE:
            if len(stack) > 1:
                stack.pop()
        pos += chunk_size
    return root


def read_manifest_from_apk(apk_path: str):
    try:
        z = zipfile.ZipFile(apk_path)
    except (zipfile.BadZipFile, OSError) as e:
        raise AxmlError(f"cannot open apk: {e}")
    with z:
        name = "AndroidManifest.xml"
        if name not in z.namelist():
            # split APKs may keep the manifest in the base; look for any manifest member
            cands = [n for n in z.namelist() if n.endswith("AndroidManifest.xml")]
            if not cands:
                raise AxmlError("no AndroidManifest.xml member")
            name = cands[0]
        return z.read(name)


def walk(node):
    yield node
    for c in node.get("children", []):
        yield from walk(c)


def main(argv):
    if len(argv) < 2:
        print(__doc__)
        return 0
    path = argv[1]
    mode = argv[2] if len(argv) > 2 else "--all"
    try:
        blob = read_manifest_from_apk(path)
    except AxmlError as e:
        print(f"AXML_ERROR: {e}", file=sys.stderr)
        return 2
    try:
        root = parse_manifest(blob)
    except AxmlError as e:
        print(f"AXML_ERROR: {e}", file=sys.stderr)
        return 3

    manifest = next(
        (n for n in walk(root) if n is not root and n["tag"] == "manifest"), root
    )
    pkg = manifest["attrs"].get("package", "?")
    print(f"package: {pkg}")
    nodes = list(walk(root))
    perms = [n["attrs"].get(ATTR_NAME) for n in nodes if n["tag"] == "uses-permission"]
    perms = [p for p in perms if p]
    sdk = [n for n in nodes if n["tag"] in ("uses-sdk",)]
    if mode in ("--permissions", "--all"):
        print(f"permissions ({len(perms)}):")
        for p in perms:
            print(f"  {p}")
    if sdk and mode in ("--all",):
        for s in sdk:
            print(f"uses-sdk: { {k:v for k,v in s['attrs'].items() if k in ('minSdkVersion','targetSdkVersion','maxSdkVersion')} }")
    if mode in ("--services", "--all"):
        for tag in ("application", "service", "activity", "receiver", "provider", "activity-alias", "meta-data"):
            items = [n for n in nodes if n["tag"] == tag]
            print(f"\n== {tag} ({len(items)}) ==")
            for it in items:
                a = it["attrs"]
                extra = {k: v for k, v in a.items() if k in (ATTR_EXPORTED, ATTR_ENABLED, ATTR_PERMISSION)}
                print(f"  {a.get(ATTR_NAME,'?')}  {extra}")
    if mode in ("--filters", "--all"):
        for n in nodes:
            if n["tag"] in ("action", "category", "data"):
                a = n["attrs"]
                if a.get(ATTR_NAME):
                    print(f"{n['tag']}: {a[ATTR_NAME]}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
