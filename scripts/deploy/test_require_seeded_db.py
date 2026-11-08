"""Tests for require_seeded_db.sh (#918): a PRBA deploy never starts on a missing database."""

from __future__ import annotations

import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / "require_seeded_db.sh"


def run(*args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(["bash", str(SCRIPT), *args], capture_output=True, text=True, check=False)


class RequireSeededDbTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.dir = Path(self.tmp.name)

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def test_missing_database_fails_with_the_seed_hint(self) -> None:
        result = run(str(self.dir / "presenter.db"))
        self.assertEqual(result.returncode, 1)
        self.assertIn("::error::database missing or empty", result.stdout)
        self.assertIn("seed_prba_from_snv.sh", result.stdout)
        self.assertFalse((self.dir / "presenter.db").exists())

    def test_empty_database_fails(self) -> None:
        db = self.dir / "presenter.db"
        db.write_bytes(b"")
        self.assertEqual(run(str(db)).returncode, 1)

    def test_seeded_database_passes(self) -> None:
        db = self.dir / "presenter.db"
        db.write_bytes(b"SQLite format 3\0" + b"\0" * 100)
        result = run(str(db))
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("Seeded database present", result.stdout)

    def test_no_argument_fails(self) -> None:
        self.assertNotEqual(run().returncode, 0)


if __name__ == "__main__":
    unittest.main()
