#!/usr/bin/env python3
"""Read TTF/OTF `name`-table metadata with the Python standard library only
(#778) — no pip installs, no fontTools. Used by `import-resolume-fonts.sh` to
decide, LOCALLY, which uploaded fonts are Microsoft/stock (never uploaded) vs
the church's own fonts.

Scans a directory for `*.ttf` / `*.otf` (case-insensitive) and prints one
tab-separated line per file to stdout:

    <filename>\t<family>\t<is_microsoft:0|1>\t<ok:0|1>

`family` is the typographic family (name id 16) falling back to the legacy
family (id 1). `is_microsoft` is 1 when the manufacturer (id 8) or copyright
(id 0) contains "microsoft" (case-insensitive) — the name-table half of the
stock-font filter. `ok` is 0 for a file that could not be parsed (a `.ttc`
collection, or a corrupt file) so the caller skips it.
"""
import os
import struct
import sys

# OpenType `name` table id's we care about.
NAME_COPYRIGHT = 0
NAME_FAMILY = 1
NAME_MANUFACTURER = 8
NAME_TYPOGRAPHIC_FAMILY = 16


def _decode(platform_id, raw):
    """Best-effort decode of a name record's bytes to a Python str."""
    try:
        if platform_id == 3 or platform_id == 0:  # Windows / Unicode → UTF-16BE
            return raw.decode("utf-16-be")
        if platform_id == 1:  # Macintosh → Roman (Latin-1 is a safe superset)
            return raw.decode("latin-1")
        return raw.decode("utf-16-be", "ignore")
    except Exception:
        return ""


def read_names(data):
    """Return {name_id: str} for the first non-empty record of each id, or None
    when the file is not a single parseable sfnt (e.g. a `.ttc` collection)."""
    if len(data) < 12:
        return None
    sfnt = data[:4]
    # Reject TrueType Collections (ttcf) and non-sfnt containers up front.
    if sfnt == b"ttcf":
        return None
    if sfnt not in (b"\x00\x01\x00\x00", b"true", b"OTTO"):
        return None
    try:
        num_tables = struct.unpack(">H", data[4:6])[0]
    except struct.error:
        return None
    name_off = None
    for i in range(num_tables):
        rec = 12 + i * 16
        if rec + 16 > len(data):
            return None
        tag = data[rec : rec + 4]
        if tag == b"name":
            name_off = struct.unpack(">I", data[rec + 8 : rec + 12])[0]
            break
    if name_off is None or name_off + 6 > len(data):
        return None
    try:
        _fmt, count, string_off = struct.unpack(">HHH", data[name_off : name_off + 6])
    except struct.error:
        return None
    records_base = name_off + 6
    strings_base = name_off + string_off
    out = {}
    for i in range(count):
        r = records_base + i * 12
        if r + 12 > len(data):
            break
        pid, _eid, _lid, nid, ln, so = struct.unpack(">HHHHHH", data[r : r + 12])
        start = strings_base + so
        raw = data[start : start + ln]
        if not raw:
            continue
        s = _decode(pid, raw).strip()
        if s and nid not in out:
            out[nid] = s
    return out


def classify(path):
    try:
        with open(path, "rb") as fh:
            data = fh.read()
    except OSError:
        return ("", 0, 0)
    names = read_names(data)
    if names is None:
        return ("", 0, 0)
    family = names.get(NAME_TYPOGRAPHIC_FAMILY) or names.get(NAME_FAMILY) or ""
    blob = (
        names.get(NAME_MANUFACTURER, "") + " " + names.get(NAME_COPYRIGHT, "")
    ).lower()
    is_ms = 1 if "microsoft" in blob else 0
    return (family, is_ms, 1 if family else 0)


def main():
    if len(sys.argv) != 2:
        sys.stderr.write("usage: read-font-metadata.py <dir>\n")
        return 2
    root = sys.argv[1]
    for name in sorted(os.listdir(root)):
        low = name.lower()
        if not (low.endswith(".ttf") or low.endswith(".otf")):
            continue
        family, is_ms, ok = classify(os.path.join(root, name))
        # Tabs/newlines can't appear in a filename we control, but strip any
        # stray whitespace in the family so the TSV stays one line per file.
        family = family.replace("\t", " ").replace("\n", " ").replace("\r", " ")
        sys.stdout.write(f"{name}\t{family}\t{is_ms}\t{ok}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
