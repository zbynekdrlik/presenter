#!/usr/bin/env python3
"""Convert a web-overlays Bible folder into a USFM zip the importer reads (#911).

Source: https://github.com/filiphanes/web-overlays/tree/master/static/bible
  <id>.json.js      index: `bookslist` with `abbreviation` + `name` per book
                    (a JS module; some entries carry trailing commas, so the
                    pairs are read with a regex, not a JSON parser)
  <id>/<abbr>.json  {"<chapter>": ["verse 1", "verse 2", ...]}

Output: one `<CODE>.usfm` per book inside the zip — `\\id`, `\\h <name>`, `\\c`,
`\\v`. The `\\h` line carries the translation's own book name (the importer
reads it with `names_from_header`). Books outside the 66-book canon (the
deuterocanon of SSV / Botek) are skipped and listed on stderr. The zip is
deterministic (fixed timestamps, sorted entries), so its sha256 is stable.

Usage: web_overlays_to_usfm.py <index.json.js> <book dir> <out.zip>
"""

from __future__ import annotations

import json
import re
import sys
import zipfile
from pathlib import Path

# web-overlays abbreviation -> USFM book code, the 66 canonical books.
BOOK_CODES: dict[str, str] = {
    "gn": "GEN", "ex": "EXO", "lv": "LEV", "nm": "NUM", "dt": "DEU",
    "joz": "JOS", "sdc": "JDG", "rut": "RUT", "1sam": "1SA", "2sam": "2SA",
    "1krl": "1KI", "2krl": "2KI", "1krn": "1CH", "2krn": "2CH", "ezd": "EZR",
    "neh": "NEH", "est": "EST", "job": "JOB", "z": "PSA", "pris": "PRO",
    "koh": "ECC", "vlp": "SNG", "iz": "ISA", "jer": "JER", "nar": "LAM",
    "ez": "EZK", "dan": "DAN", "oz": "HOS", "joel": "JOL", "am": "AMO",
    "abd": "OBA", "jon": "JON", "mich": "MIC", "nah": "NAM", "hab": "HAB",
    "sof": "ZEP", "ag": "HAG", "zach": "ZEC", "mal": "MAL",
    "mt": "MAT", "mk": "MRK", "lk": "LUK", "jn": "JHN", "sk": "ACT",
    "rim": "ROM", "1kor": "1CO", "2kor": "2CO", "ga": "GAL", "ef": "EPH",
    "flp": "PHP", "kol": "COL", "1tes": "1TH", "2tes": "2TH", "1tim": "1TI",
    "2tim": "2TI", "tit": "TIT", "flm": "PHM", "heb": "HEB", "jk": "JAS",
    "1pt": "1PE", "2pt": "2PE", "1jn": "1JN", "2jn": "2JN", "3jn": "3JN",
    "jud": "JUD", "zj": "REV",
}

# Some folders name the files differently from the index (AMPC: `1sm.json` for
# the index's `1sam`).
FILE_ALIASES: dict[str, tuple[str, ...]] = {
    "1sam": ("1sm",), "2sam": ("2sm",), "1krl": ("1kr",), "2krl": ("2kr",),
    "pris": ("pr",), "koh": ("kaz",), "vlp": ("pis",), "joel": ("jl",),
    "ag": ("hag",), "ga": ("gal",), "flp": ("fp",), "flm": ("fm",), "jk": ("jak",),
}

_ENTRY = re.compile(r'"abbreviation"\s*:\s*"([^"]+)"[^{}]*?"name"\s*:\s*"([^"]+)"', re.S)
_FIXED_TIME = (2026, 1, 1, 0, 0, 0)


def read_index(text: str) -> list[tuple[str, str]]:
    """The (abbreviation, name) pairs of `bookslist`, in source order."""
    pairs = _ENTRY.findall(text)
    if not pairs:
        raise ValueError("index has no bookslist entries")
    return pairs


def clean(text: str) -> str:
    return " ".join(text.split())


def book_usfm(code: str, name: str, chapters: dict[str, list[str]]) -> str:
    lines = [f"\\id {code}", f"\\h {name}"]
    for chapter in sorted(chapters, key=int):
        lines.append(f"\\c {int(chapter)}")
        for number, verse in enumerate(chapters[chapter], start=1):
            text = clean(verse)
            if text:
                lines.append(f"\\v {number} {text}")
    return "\n".join(lines) + "\n"


def book_file(book_dir: Path, abbreviation: str) -> Path:
    for name in (abbreviation, *FILE_ALIASES.get(abbreviation, ())):
        path = book_dir / f"{name}.json"
        if path.is_file():
            return path
    raise FileNotFoundError(f"no file for {abbreviation} in {book_dir}")


def convert(index_text: str, book_dir: Path) -> tuple[dict[str, str], list[str]]:
    """USFM per book code, and the abbreviations skipped (outside the canon)."""
    books: dict[str, str] = {}
    skipped: list[str] = []
    for abbreviation, name in read_index(index_text):
        code = BOOK_CODES.get(abbreviation)
        if code is None:
            skipped.append(abbreviation)
            continue
        path = book_file(book_dir, abbreviation)
        chapters = json.loads(path.read_text(encoding="utf-8"))
        books[code] = book_usfm(code, name, chapters)
    missing = sorted(set(BOOK_CODES.values()) - set(books))
    if missing:
        raise ValueError(f"books missing from the source: {', '.join(missing)}")
    return books, skipped


def write_zip(books: dict[str, str], out: Path) -> None:
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as archive:
        for code in sorted(books):
            info = zipfile.ZipInfo(f"{code}.usfm", date_time=_FIXED_TIME)
            info.compress_type = zipfile.ZIP_DEFLATED
            archive.writestr(info, books[code].encode("utf-8"))


def main(argv: list[str]) -> int:
    if len(argv) != 4:
        print(__doc__, file=sys.stderr)
        return 2
    index, book_dir, out = Path(argv[1]), Path(argv[2]), Path(argv[3])
    books, skipped = convert(index.read_text(encoding="utf-8"), book_dir)
    write_zip(books, out)
    verses = sum(text.count("\\v ") for text in books.values())
    print(f"{out}: {len(books)} books, {verses} verses")
    if skipped:
        print(f"skipped (outside the 66-book canon): {' '.join(skipped)}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
