"""Tests for web_overlays_to_usfm.py (#911): made-up verses, the source's shapes."""

from __future__ import annotations

import json
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import web_overlays_to_usfm as conv  # noqa: E402


def index_text(entries: list[tuple[str, str]], trailing_commas: bool) -> str:
    comma = "," if trailing_commas else ""
    body = ",\n".join(
        f'    {{\n      "abbreviation": "{a}",\n      "abb": "X",\n      "name": "{n}"{comma}\n    }}'
        for a, n in entries
    )
    return f'export const index = {{\n  "identifier": "t",\n  "bookslist": [\n{body}\n  ]\n}};\n'


class ConvertTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.dir = Path(self.tmp.name)
        self.entries = [(a, f"Kniha {a}") for a in conv.BOOK_CODES]
        for abbreviation, _ in self.entries:
            (self.dir / f"{abbreviation}.json").write_text(
                json.dumps({"1": ["Prvý  verš.", "Druhý\nverš."]}), encoding="utf-8"
            )

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def test_every_canonical_book_becomes_usfm_with_its_own_name(self) -> None:
        books, skipped = conv.convert(index_text(self.entries, True), self.dir)
        self.assertEqual(len(books), 66)
        self.assertEqual(skipped, [])
        self.assertEqual(
            books["PSA"], "\\id PSA\n\\h Kniha z\n\\c 1\n\\v 1 Prvý verš.\n\\v 2 Druhý verš.\n"
        )

    def test_a_book_outside_the_canon_is_skipped(self) -> None:
        (self.dir / "tob.json").write_text(json.dumps({"1": ["x"]}), encoding="utf-8")
        entries = self.entries[:10] + [("tob", "Tobiáš")] + self.entries[10:]
        books, skipped = conv.convert(index_text(entries, False), self.dir)
        self.assertEqual(skipped, ["tob"])
        self.assertNotIn("TOB", books)

    def test_chapters_are_numeric_order_and_empty_verses_dropped(self) -> None:
        text = conv.book_usfm("GEN", "Genezis", {"10": ["a"], "2": ["", "b"]})
        self.assertEqual(text, "\\id GEN\n\\h Genezis\n\\c 2\n\\v 2 b\n\\c 10\n\\v 1 a\n")

    def test_a_file_under_its_folder_name_is_found(self) -> None:
        (self.dir / "1sam.json").rename(self.dir / "1sm.json")
        books, _ = conv.convert(index_text(self.entries, True), self.dir)
        self.assertIn("\\h Kniha 1sam", books["1SA"])

    def test_a_missing_canonical_book_fails(self) -> None:
        with self.assertRaises(ValueError):
            conv.convert(index_text(self.entries[:-1], True), self.dir)

    def test_the_zip_is_deterministic(self) -> None:
        books, _ = conv.convert(index_text(self.entries, True), self.dir)
        first, second = self.dir / "a.zip", self.dir / "b.zip"
        conv.write_zip(books, first)
        conv.write_zip(books, second)
        self.assertEqual(first.read_bytes(), second.read_bytes())
        with zipfile.ZipFile(first) as archive:
            self.assertEqual(len(archive.namelist()), 66)
            self.assertIn("GEN.usfm", archive.namelist())


if __name__ == "__main__":
    unittest.main()
