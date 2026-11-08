"""Tests for prba_seed_clean.py (#918): a copy of the SNV DB loses every SNV device."""

from __future__ import annotations

import sqlite3
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / "prba_seed_clean.py"

DEVICE_TABLES = [
    "resolume_hosts",
    "resolume_push_audit",
    "android_stage_displays",
    "video_sources",
    "ableset_settings",
    "able_set_settings",
    "osc_settings",
    "stage_state",
    "ui_usage",
    "interpreter_tips",
    "settings_audit",
]


def make_db(path: Path) -> None:
    db = sqlite3.connect(path)
    for table in DEVICE_TABLES:
        db.execute(f'CREATE TABLE "{table}" (id TEXT PRIMARY KEY, note TEXT)')
        db.execute(f'INSERT INTO "{table}" VALUES (?, ?)', ("1", "10.77.9.205"))
    db.execute("CREATE TABLE presentations (id TEXT PRIMARY KEY, name TEXT)")
    db.executemany("INSERT INTO presentations VALUES (?, ?)", [("a", "Pieseň A"), ("b", "Pieseň B")])
    db.execute("CREATE TABLE stream_outputs (id INTEGER PRIMARY KEY, slug TEXT)")
    db.execute("INSERT INTO stream_outputs VALUES (1, 'stream')")
    db.execute('CREATE TABLE app_settings ("key" TEXT PRIMARY KEY, value TEXT)')
    db.executemany(
        "INSERT INTO app_settings VALUES (?, ?)",
        [
            ("listen.mics", "[]"),
            ("sync.peer", "http://100.101.72.101"),
            ("feature.companion.enabled", "1"),
            ("feature.companion.port", "18175"),
            ("ai-settings", '{"apiUrl":"http://127.0.0.1:18787/v1"}'),
            ("feature.stage.layout", "worship-snv"),
        ],
    )
    db.commit()
    db.close()


def run(path: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, "-I", str(SCRIPT), str(path)], capture_output=True, text=True, check=False
    )


def count(path: Path, table: str) -> int:
    db = sqlite3.connect(path)
    try:
        return db.execute(f'SELECT COUNT(*) FROM "{table}"').fetchone()[0]
    finally:
        db.close()


class SeedCleanTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.db = Path(self.tmp.name) / "seed.db"
        make_db(self.db)

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def test_device_tables_are_emptied(self) -> None:
        result = run(self.db)
        self.assertEqual(result.returncode, 0, result.stderr)
        for table in DEVICE_TABLES:
            self.assertEqual(count(self.db, table), 0, table)
            self.assertIn(f"cleared {table}=0", result.stdout)

    def test_songs_and_stream_design_are_kept(self) -> None:
        result = run(self.db)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(count(self.db, "presentations"), 2)
        self.assertEqual(count(self.db, "stream_outputs"), 1)
        self.assertIn("kept presentations=2", result.stdout)

    def test_site_keys_go_and_the_rest_stays(self) -> None:
        self.assertEqual(run(self.db).returncode, 0)
        db = sqlite3.connect(self.db)
        keys = {row[0] for row in db.execute("SELECT key FROM app_settings")}
        db.close()
        self.assertEqual(keys, {"feature.stage.layout"})

    def test_leftover_address_fails(self) -> None:
        db = sqlite3.connect(self.db)
        db.execute("INSERT INTO app_settings VALUES ('ndi.host', 'http://10.77.9.205')")
        db.commit()
        db.close()
        result = run(self.db)
        self.assertEqual(result.returncode, 1)
        self.assertIn("leftover: app_settings.ndi.host", result.stderr)

    def test_lan_name_counts_as_an_address(self) -> None:
        db = sqlite3.connect(self.db)
        db.execute("INSERT INTO app_settings VALUES ('x.target', 'presenter.lan')")
        db.commit()
        db.close()
        self.assertEqual(run(self.db).returncode, 1)

    def test_missing_table_is_not_an_error(self) -> None:
        db = sqlite3.connect(self.db)
        db.execute("DROP TABLE able_set_settings")
        db.commit()
        db.close()
        result = run(self.db)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_missing_file_fails(self) -> None:
        result = run(Path(self.tmp.name) / "nope.db")
        self.assertEqual(result.returncode, 1)


if __name__ == "__main__":
    unittest.main()
