#!/usr/bin/env python3
"""Unit tests for repin-presenter-connection.py (#733).

Run: python3 -m unittest ops/companion/repin-presenter-connection.test
(the module name has a dot in the file stem, so it is loaded by path below).
"""
import importlib.util
import json
import os
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

    def test_already_dev_is_idempotent_noop(self):
        new_value, changed, old = repin.repin_value(_conn(version="dev"))
        self.assertIsNone(new_value)
        self.assertFalse(changed)
        self.assertEqual(old, "dev")

    def test_null_version_latest_is_repinned_to_dev(self):
        # null (=latest) is still not the stable dev id -> normalize to dev
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


if __name__ == "__main__":
    unittest.main()
