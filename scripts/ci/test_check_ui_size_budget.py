"""Unit tests for check_ui_size_budget.py (#850 B1c)."""

import pathlib
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).parent))

from check_ui_size_budget import main, measure, over_budget  # noqa: E402


class MeasureTests(unittest.TestCase):
    def test_reads_the_wasm_and_its_brotli_sibling(self):
        with tempfile.TemporaryDirectory() as tmp:
            dist = pathlib.Path(tmp)
            (dist / "presenter-ui-abc_bg.wasm").write_bytes(b"x" * 100)
            (dist / "presenter-ui-abc_bg.wasm.br").write_bytes(b"x" * 30)
            (dist / "presenter-ui-abc_bg.wasm.gz").write_bytes(b"x" * 40)
            self.assertEqual(
                measure(dist),
                {"wasm_raw_bytes": 100, "wasm_br_bytes": 30, "wasm_gz_bytes": 40},
            )

    def test_a_missing_sibling_or_two_wasm_files_fail(self):
        with tempfile.TemporaryDirectory() as tmp:
            dist = pathlib.Path(tmp)
            (dist / "a_bg.wasm").write_bytes(b"x")
            with self.assertRaises(SystemExit):
                measure(dist)
            (dist / "a_bg.wasm.br").write_bytes(b"x")
            with self.assertRaises(SystemExit):
                measure(dist)
            (dist / "a_bg.wasm.gz").write_bytes(b"x")
            self.assertEqual(measure(dist)["wasm_gz_bytes"], 1)
            (dist / "b_bg.wasm").write_bytes(b"x")
            with self.assertRaises(SystemExit):
                measure(dist)


class BudgetTests(unittest.TestCase):
    def test_only_sizes_above_the_budget_are_reported(self):
        budget = {"wasm_raw_bytes": 100, "wasm_br_bytes": 30}
        self.assertEqual(over_budget({"wasm_raw_bytes": 100, "wasm_br_bytes": 30}, budget), [])
        self.assertEqual(
            over_budget({"wasm_raw_bytes": 101, "wasm_br_bytes": 30}, budget),
            ["wasm_raw_bytes: 101 > budget 100"],
        )

    def test_main_exits_non_zero_over_budget(self):
        with tempfile.TemporaryDirectory() as tmp:
            dist = pathlib.Path(tmp)
            (dist / "a_bg.wasm").write_bytes(b"x" * 10)
            (dist / "a_bg.wasm.br").write_bytes(b"x" * 5)
            (dist / "a_bg.wasm.gz").write_bytes(b"x" * 6)
            budget = dist / "budget.json"
            budget.write_text('{"wasm_raw_bytes": 9, "wasm_br_bytes": 5}', encoding="utf-8")
            self.assertEqual(main([str(dist), "--budget", str(budget)]), 1)
            budget.write_text('{"wasm_raw_bytes": 10, "wasm_br_bytes": 5}', encoding="utf-8")
            self.assertEqual(main([str(dist), "--budget", str(budget)]), 0)


if __name__ == "__main__":
    unittest.main()
