#!/usr/bin/env python3
"""Self-tests for gen_bible_book_names.py (#850 spec §5.2, §10).

Run: python3 -m unittest discover -s scripts/dev -p 'test_gen_bible_book_names.py' -v
"""

from __future__ import annotations

import json
import os
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import gen_bible_book_names as gen  # noqa: E402

FOLD_CASES = gen.REPO / "crates" / "presenter-core" / "src" / "bible" / "resolver" / "testdata" / "fold-cases.json"


def table_rows() -> list[gen.Row]:
    curated = gen.yaml.safe_load((gen.DATA / "curated.yaml").read_text(encoding="utf-8"))
    forms, codes = gen.collect_forms(gen.DATA, gen.CANONICAL.read_text(encoding="utf-8"))
    rows, _ = gen.build_table(forms, codes, curated)
    return rows


class CanonicalSourceTests(unittest.TestCase):
    def test_canonical_arrays_have_66_entries_each(self):
        books, arrays, pairs = gen.parse_canonical(gen.CANONICAL.read_text(encoding="utf-8"))
        self.assertEqual(len(books), 66)
        for name in gen.SLOVAK_ARRAYS:
            self.assertEqual(len(arrays[name]), 66, name)
        self.assertNotIn(("Ab", "OBA"), pairs["SLOVAK_ABBREVIATIONS"])

    def test_a_short_array_fails_loudly(self):
        text = gen.CANONICAL.read_text(encoding="utf-8").replace('    "Zjavenie Jána",\n', "", 1)
        with self.assertRaises(gen.GeneratorError):
            gen.parse_canonical(text)


class FoldTests(unittest.TestCase):
    def test_shared_fold_cases(self):
        cases = json.loads(FOLD_CASES.read_text(encoding="utf-8"))["cases"]
        self.assertGreaterEqual(len(cases), 40)
        for case in cases:
            self.assertEqual(gen.fold(case["input"]), (case["ordinal"], case["key"]), case["input"])

    def test_tiers(self):
        self.assertEqual(gen.tier("db:slk-seb"), 1)
        self.assertEqual(gen.tier("biblia.sk:sep:abbr"), 1)
        self.assertEqual(gen.tier("canonical.rs:SLOVAK_ABBREVIATIONS"), 1)
        self.assertEqual(gen.tier("curated:tier1"), 1)
        self.assertEqual(gen.tier("bprp:slk"), 2)
        self.assertEqual(gen.tier("db:eng-kjv"), 2)
        self.assertEqual(gen.tier("canonical.rs:english_name"), 2)
        self.assertEqual(gen.tier("osis"), 2)
        self.assertEqual(gen.tier("bprp:ces"), 3)
        self.assertEqual(gen.tier("bprp:eng:short"), 3)


class TableTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.rows = table_rows()
        codes = [code for code, _, _ in gen.parse_canonical(gen.CANONICAL.read_text(encoding="utf-8"))[0]]
        cls.by_key = {}
        for row in cls.rows:
            cls.by_key.setdefault((row.key, row.ordinal), set()).add(codes[row.index])

    def test_data_fixes(self):
        self.assertEqual(self.by_key[("ab", 0)], {"HAB"}, "Ab is Habakkuk, never Obadiah")
        self.assertIn("MRK", self.by_key[("mr", 0)])
        self.assertIn("PHM", self.by_key[("filimonovi", 0)], "keep SEB's stored misspelling")
        self.assertEqual(self.by_key[("ps", 0)], {"PSA"}, "P.Š. must not fold into Psalms")
        self.assertIn("HEB", self.by_key[("zid", 0)])

    def test_slovak_beats_english_on_the_1k_collision(self):
        tiers = {(r.index, r.tier) for r in self.rows if r.key == "k" and r.ordinal == 1}
        self.assertIn((45, 1), tiers, "1K = 1 Korinťanom at tier 1 (SEP)")
        self.assertIn((10, 3), tiers, "1 K = 1 Kings only at tier 3 (BPRP English)")

    def test_rows_are_sorted_and_unique(self):
        keys = [(r.key, r.ordinal, r.index) for r in self.rows]
        self.assertEqual(keys, sorted(keys))
        self.assertEqual(len(keys), len(set(keys)))

    def test_short_defaults_reject_an_unreachable_code(self):
        curated = {"short_defaults": {"z": ["PSA", "GEN"]}}
        codes = [c for c, _, _ in gen.parse_canonical(gen.CANONICAL.read_text(encoding="utf-8"))[0]]
        with self.assertRaises(gen.GeneratorError):
            gen.short_defaults(curated, self.rows, codes)


class CheckModeTests(unittest.TestCase):
    def test_check_passes_on_a_fresh_table_and_fails_on_drift(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp) / "names_table.rs"
            self.assertEqual(gen.main(["--output", str(output)]), 0)
            self.assertEqual(gen.main(["--check", "--output", str(output)]), 0)
            output.write_text(output.read_text(encoding="utf-8").replace('"zid"', '"zyd"'), encoding="utf-8")
            self.assertEqual(gen.main(["--check", "--output", str(output)]), 1)

    def test_committed_table_is_up_to_date(self):
        self.assertEqual(gen.main(["--check"]), 0)

    def test_a_missing_source_fails_without_writing(self):
        with tempfile.TemporaryDirectory() as tmp:
            data = Path(tmp) / "data"
            shutil.copytree(gen.DATA, data)
            (data / "biblia-sk.json").unlink()
            output = Path(tmp) / "names_table.rs"
            self.assertEqual(gen.main(["--data", str(data), "--output", str(output)]), 1)
            self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main()
