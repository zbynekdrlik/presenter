#!/usr/bin/env python3
"""Unit + integration tests for repin-presenter-connection.py (#733).

Run by file path (the stem has hyphens, so `-m unittest` cannot import it):
    python3 ops/companion/repin-presenter-connection.test.py
"""
import importlib.util
import json
import os
import sqlite3
import tempfile
import unittest

_HERE = os.path.dirname(os.path.abspath(__file__))
_SPEC = importlib.util.spec_from_file_location(
    "repin_presenter_connection", os.path.join(_HERE, "repin-presenter-connection.py")
)
repin = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(repin)


def _conn(module_id="presenter", version="0.9.0", extra=None):
    d = {
        "moduleInstanceType": "connection",
        "moduleId": module_id,
        "moduleVersionId": version,
        "updatePolicy": "manual",
        "label": module_id,
        "enabled": True,
        "config": {"host": "presenter.lan", "port": 18175},
    }
    if extra:
        d.update(extra)
    return json.dumps(d)


class RepinValueTests(unittest.TestCase):
    def test_concrete_version_is_repinned_to_dev(self):
        new_value, changed, old = repin.repin_value(_conn(version="0.9.0"))
        self.assertTrue(changed)
        self.assertEqual(old, "0.9.0")
        self.assertEqual(json.loads(new_value)["moduleVersionId"], "dev")
        # everything else preserved
        self.assertEqual(json.loads(new_value)["config"]["port"], 18175)
        self.assertEqual(json.loads(new_value)["label"], "presenter")

    def test_repinned_value_uses_compact_json(self):
        new_value, _changed, _old = repin.repin_value(_conn(version="0.9.0", extra={"label": "prezentér"}))
        # compact separators, non-escaped UTF-8 (matches Companion's JSON.stringify)
        self.assertIn('"moduleVersionId":"dev"', new_value)
        self.assertIn("prezentér", new_value)

    def test_already_dev_is_idempotent_noop(self):
        new_value, changed, old = repin.repin_value(_conn(version="dev"))
        self.assertIsNone(new_value)
        self.assertFalse(changed)
        self.assertEqual(old, "dev")

    def test_null_version_latest_is_repinned_to_dev(self):
        new_value, changed, old = repin.repin_value(_conn(version=None))
        self.assertTrue(changed)
        self.assertIsNone(old)
        self.assertEqual(json.loads(new_value)["moduleVersionId"], "dev")

    def test_other_module_untouched(self):
        new_value, changed, _ = repin.repin_value(_conn(module_id="obs-studio", version="3.15.3"))
        self.assertIsNone(new_value)
        self.assertFalse(changed)

    def test_non_connection_untouched(self):
        surface = json.dumps({"moduleInstanceType": "surface", "moduleId": "presenter", "moduleVersionId": "builtin"})
        new_value, changed, _ = repin.repin_value(surface)
        self.assertIsNone(new_value)
        self.assertFalse(changed)

    def test_malformed_json_is_skipped_not_raised(self):
        new_value, changed, old = repin.repin_value("{not json")
        self.assertIsNone(new_value)
        self.assertFalse(changed)
        self.assertIsNone(old)

    def test_custom_module_and_dev_ids(self):
        new_value, changed, old = repin.repin_value(
            _conn(module_id="widget", version="1.2.3"), module_id="widget", dev_version_id="local"
        )
        self.assertTrue(changed)
        self.assertEqual(old, "1.2.3")
        self.assertEqual(json.loads(new_value)["moduleVersionId"], "local")


class RepinRowsTests(unittest.TestCase):
    def test_only_presenter_connections_change(self):
        rows = [
            ("a", _conn(module_id="obs-studio", version="3.15.3")),
            ("b", _conn(version="0.9.0")),
            ("c", _conn(module_id="generic-osc", version="2.7.0")),
            ("d", _conn(version="0.8.1")),
        ]
        updates, matched = repin.repin_rows(rows)
        changed_ids = sorted(u[0] for u in updates)
        self.assertEqual(changed_ids, ["b", "d"])
        self.assertEqual(sorted(matched), ["b", "d"])

    def test_matched_includes_already_dev(self):
        rows = [("a", _conn(version="dev")), ("b", _conn(version="0.9.0"))]
        updates, matched = repin.repin_rows(rows)
        self.assertEqual([u[0] for u in updates], ["b"])  # only b changes
        self.assertEqual(sorted(matched), ["a", "b"])  # both are presenter connections

    def test_no_presenter_connection(self):
        rows = [("a", _conn(module_id="obs-studio", version="3.15.3"))]
        updates, matched = repin.repin_rows(rows)
        self.assertEqual(updates, [])
        self.assertEqual(matched, [])

    def test_malformed_row_does_not_break_others(self):
        rows = [("bad", "{broken"), ("good", _conn(version="0.9.0"))]
        updates, matched = repin.repin_rows(rows)
        self.assertEqual([u[0] for u in updates], ["good"])
        self.assertEqual(matched, ["good"])


class ParseMajorMinorTests(unittest.TestCase):
    def test_snv_5_0_3(self):
        self.assertEqual(repin.parse_major_minor("5.0.3+9703-stable-2daa0d7670"), "v5.0")

    def test_pp_4_3_1(self):
        self.assertEqual(repin.parse_major_minor("4.3.1+9209-stable-bf5535c82b"), "v4.3")

    def test_leading_whitespace(self):
        self.assertEqual(repin.parse_major_minor("  4.3.1"), "v4.3")

    def test_garbage(self):
        self.assertIsNone(repin.parse_major_minor("not-a-version"))

    def test_empty(self):
        self.assertIsNone(repin.parse_major_minor(""))
        self.assertIsNone(repin.parse_major_minor(None))


class ResolveDbPathTests(unittest.TestCase):
    def _mk(self, root, rel):
        p = os.path.join(root, rel)
        os.makedirs(os.path.dirname(p), exist_ok=True)
        open(p, "w").close()
        return p

    def _build(self, root, content):
        p = os.path.join(root, "BUILD")
        with open(p, "w") as fh:
            fh.write(content)
        return p

    def test_explicit_db_wins(self):
        with tempfile.TemporaryDirectory() as t:
            db = self._mk(t, "somewhere/db.sqlite")
            self.assertEqual(repin.resolve_db_path(t, "/nope/BUILD", db), db)

    def test_explicit_db_missing_raises(self):
        with tempfile.TemporaryDirectory() as t:
            with self.assertRaises(FileNotFoundError):
                repin.resolve_db_path(t, "/nope/BUILD", os.path.join(t, "nope.sqlite"))

    def test_build_derived_versioned_db(self):
        with tempfile.TemporaryDirectory() as t:
            db = self._mk(t, "v5.0/db.sqlite")
            self._mk(t, "db.sqlite")  # legacy also present — must NOT be chosen
            build = self._build(t, "5.0.3+9703-stable-x")
            self.assertEqual(repin.resolve_db_path(t, build, None), db)

    def test_build_derived_db_missing_fails_loud_no_legacy_fallback(self):
        with tempfile.TemporaryDirectory() as t:
            self._mk(t, "db.sqlite")  # legacy present but BUILD says v5.0 which is absent
            build = self._build(t, "5.0.3+x")
            with self.assertRaises(FileNotFoundError):
                repin.resolve_db_path(t, build, None)

    def test_build_unparseable_picks_newest_versioned(self):
        with tempfile.TemporaryDirectory() as t:
            self._mk(t, "v4.2/db.sqlite")
            newest = self._mk(t, "v4.3/db.sqlite")
            self._mk(t, "db.sqlite")
            build = self._build(t, "garbage")
            self.assertEqual(repin.resolve_db_path(t, build, None), newest)

    def test_no_build_no_versioned_falls_back_to_legacy(self):
        with tempfile.TemporaryDirectory() as t:
            legacy = self._mk(t, "db.sqlite")
            self.assertEqual(repin.resolve_db_path(t, "/nope/BUILD", None), legacy)

    def test_nothing_found_raises(self):
        with tempfile.TemporaryDirectory() as t:
            with self.assertRaises(FileNotFoundError):
                repin.resolve_db_path(t, "/nope/BUILD", None)


class MainIntegrationTests(unittest.TestCase):
    def _make_db(self, rows):
        fd, path = tempfile.mkstemp(suffix=".sqlite")
        os.close(fd)
        con = sqlite3.connect(path)
        con.execute("CREATE TABLE instances (id STRING, value STRING)")
        con.executemany("INSERT INTO instances (id, value) VALUES (?, ?)", rows)
        con.commit()
        con.close()
        return path

    def _pin(self, path, conn_id):
        con = sqlite3.connect(path)
        try:
            for i, v in con.execute("SELECT id, value FROM instances"):
                if i == conn_id:
                    return json.loads(v).get("moduleVersionId")
        finally:
            con.close()
        return None

    def test_main_repins_presenter_only_and_is_idempotent(self):
        path = self._make_db([
            ("p", _conn(version="0.9.0")),
            ("o", _conn(module_id="obs-studio", version="3.15.3")),
        ])
        try:
            rc = repin.main(["--db", path, "--build-file", "/nope"])
            self.assertEqual(rc, 0)
            self.assertEqual(self._pin(path, "p"), "dev")
            self.assertEqual(self._pin(path, "o"), "3.15.3")  # untouched
            # second run: idempotent no-op, still rc 0
            rc2 = repin.main(["--db", path, "--build-file", "/nope"])
            self.assertEqual(rc2, 0)
            self.assertEqual(self._pin(path, "p"), "dev")
        finally:
            os.unlink(path)

    def test_main_dry_run_does_not_write(self):
        path = self._make_db([("p", _conn(version="0.9.0"))])
        try:
            rc = repin.main(["--db", path, "--build-file", "/nope", "--dry-run"])
            self.assertEqual(rc, 0)
            self.assertEqual(self._pin(path, "p"), "0.9.0")  # unchanged
        finally:
            os.unlink(path)

    def test_expect_connection_fails_when_absent(self):
        path = self._make_db([("o", _conn(module_id="obs-studio", version="3.15.3"))])
        try:
            rc = repin.main(["--db", path, "--build-file", "/nope", "--expect-connection"])
            self.assertEqual(rc, 2)  # loud failure — no presenter connection present
        finally:
            os.unlink(path)

    def test_expect_connection_ok_when_present(self):
        path = self._make_db([("p", _conn(version="dev"))])
        try:
            rc = repin.main(["--db", path, "--build-file", "/nope", "--expect-connection"])
            self.assertEqual(rc, 0)  # present + already dev
        finally:
            os.unlink(path)


if __name__ == "__main__":
    unittest.main()
