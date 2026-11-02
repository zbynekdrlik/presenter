#!/usr/bin/env python3
"""Self-tests for fetch_study_sources.py (#851 D3a).

Run: python3 -m unittest discover -s scripts/dev -p 'test_fetch_study_sources.py' -v
"""

from __future__ import annotations

import json
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))

import fetch_study_sources as fss  # noqa: E402


class CheckCommitted(unittest.TestCase):
    def setUp(self) -> None:
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.dir = Path(temp.name)
        with zipfile.ZipFile(self.dir / "a.zip", "w") as archive:
            archive.writestr("a.xml", b"xml")
        (self.dir / "b.zip").write_bytes(b"upstream")
        self.write_sources(fss.sha256(b"xml"), fss.sha256(b"upstream"))

    def write_sources(self, member: str, whole: str) -> None:
        sources = {
            "files": [
                {
                    "file": "a.zip",
                    "members": [{"name": "a.xml", "url": "http://x/a", "sha256": member}],
                },
                {"file": "b.zip", "url": "http://x/b", "sha256": whole},
            ]
        }
        (self.dir / "sources.json").write_text(json.dumps(sources), encoding="utf-8")

    def test_matching_files_pass(self) -> None:
        self.assertEqual(fss.check_committed(self.dir), [])

    def test_a_changed_member_is_named(self) -> None:
        self.write_sources("0" * 64, fss.sha256(b"upstream"))
        self.assertEqual(
            fss.check_committed(self.dir), ["a.zip: a.xml differs from sources.json"]
        )

    def test_a_changed_file_is_named(self) -> None:
        (self.dir / "b.zip").write_bytes(b"other")
        self.assertEqual(
            fss.check_committed(self.dir), ["b.zip: SHA-256 differs from sources.json"]
        )

    def test_a_missing_file_is_named(self) -> None:
        (self.dir / "b.zip").unlink()
        self.assertEqual(fss.check_committed(self.dir), ["b.zip: missing"])

    def test_a_missing_member_is_named(self) -> None:
        with zipfile.ZipFile(self.dir / "a.zip", "w") as archive:
            archive.writestr("other.xml", b"xml")
        self.assertEqual(fss.check_committed(self.dir), ["a.zip: a.xml missing"])

    def test_the_repository_files_pass(self) -> None:
        self.assertEqual(fss.check_committed(fss.STUDY_DIR), [])


class Fetch(unittest.TestCase):
    def test_changed_upstream_bytes_stop_the_fetch(self) -> None:
        with mock.patch.object(fss, "download", return_value=b"changed"):
            with self.assertRaises(SystemExit):
                fss.verified("http://x", fss.sha256(b"pinned"), "x")

    def test_a_fetch_rebuilds_what_the_check_accepts(self) -> None:
        with tempfile.TemporaryDirectory() as name:
            study = Path(name)
            sources = {
                "files": [
                    {
                        "file": "a.zip",
                        "members": [
                            {"name": "a.xml", "url": "http://x/a", "sha256": fss.sha256(b"xml")}
                        ],
                    }
                ]
            }
            (study / "sources.json").write_text(json.dumps(sources), encoding="utf-8")
            with mock.patch.object(fss, "download", return_value=b"xml"):
                fss.fetch(study)
            self.assertEqual(fss.check_committed(study), [])


if __name__ == "__main__":
    unittest.main()
