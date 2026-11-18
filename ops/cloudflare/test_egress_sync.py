#!/usr/bin/env python3
"""Self-tests for egress_sync.py (#977) — no network, no ssh, no Cloudflare.

Run: python3 -m unittest discover -s ops/cloudflare -p 'test_egress_sync.py' -v
"""

from __future__ import annotations

import io
import json
import os
import stat
import sys
import tempfile
import unittest
from contextlib import redirect_stdout

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import egress_sync as es  # noqa: E402

CONFIG = {
    "account_id": "acc",
    "allowed_emails": ["person@example.com"],
    "sites": [
        {"host": "a.example.com", "bypass_ips": ["81.2.69.160/32"], "egress_ssh": "u@a"},
        {"host": "b.example.com", "bypass_ips": ["93.184.216.34/32"], "egress_ssh": "u@b"},
        {"host": "c.example.com", "bypass_ips": []},
    ],
    "session_duration": "720h",
    "healthz_session_duration": "24h",
}


class Pure(unittest.TestCase):
    def test_parse_ipv4_accepts_a_public_address_with_whitespace(self):
        self.assertEqual(es.parse_ipv4(" 151.101.1.69\n"), "151.101.1.69")

    def test_parse_ipv4_refuses_private_loopback_html_and_ipv6(self):
        for text in ("10.77.9.205", "127.0.0.1", "<html>", "", "2001:db8::1"):
            self.assertIsNone(es.parse_ipv4(text), text)

    def test_agreed_address_needs_every_source_to_agree(self):
        self.assertEqual(es.agreed_address(["151.101.1.69", "151.101.1.69"]), "151.101.1.69")
        self.assertIsNone(es.agreed_address(["151.101.1.69", "151.101.1.70"]))
        self.assertIsNone(es.agreed_address(["151.101.1.69", ""]))
        self.assertIsNone(es.agreed_address(["151.101.1.69"]))

    def test_with_new_bypass_replaces_only_that_site_and_keeps_the_input(self):
        updated = es.with_new_bypass(CONFIG, "b.example.com", "151.101.1.69")
        self.assertEqual(updated["sites"][1]["bypass_ips"], ["151.101.1.69/32"])
        self.assertEqual(updated["sites"][0]["bypass_ips"], ["81.2.69.160/32"])
        self.assertEqual(CONFIG["sites"][1]["bypass_ips"], ["93.184.216.34/32"])


class Sync(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.path = os.path.join(self.dir.name, "access.json")
        with open(self.path, "w", encoding="utf-8") as handle:
            json.dump(CONFIG, handle)
        os.chmod(self.path, 0o600)
        self.calls: list[list[str]] = []

    def tearDown(self):
        self.dir.cleanup()

    def reconcile(self, args):
        self.calls.append(args)
        return 0

    def run_sync(self, answers):
        out = io.StringIO()
        with redirect_stdout(out):
            code = es.sync(self.path, lambda target: answers[target], self.reconcile)
        return code, out.getvalue()

    def saved(self):
        with open(self.path, encoding="utf-8") as handle:
            return json.load(handle)

    def test_unchanged_addresses_touch_nothing(self):
        code, out = self.run_sync({"u@a": ["81.2.69.160"] * 2, "u@b": ["93.184.216.34"] * 2})
        self.assertEqual(code, 0)
        self.assertEqual(self.calls, [])
        self.assertEqual(self.saved(), CONFIG)
        self.assertEqual(out.count("egress unchanged"), 2)

    def test_a_changed_address_rewrites_the_config_then_applies_and_dry_runs(self):
        code, out = self.run_sync({"u@a": ["81.2.69.160"] * 2, "u@b": ["151.101.1.69"] * 2})
        self.assertEqual(code, 0)
        self.assertEqual(self.saved()["sites"][1]["bypass_ips"], ["151.101.1.69/32"])
        self.assertEqual(self.saved()["sites"][0]["bypass_ips"], ["81.2.69.160/32"])
        self.assertEqual(self.calls, [["--config", self.path, "--apply"], ["--config", self.path]])
        self.assertEqual(stat.S_IMODE(os.stat(self.path).st_mode), 0o600)
        self.assertIn("b.example.com: public address changed", out)

    def test_an_unreachable_box_keeps_its_bypass(self):
        code, out = self.run_sync({"u@a": None, "u@b": ["93.184.216.34"] * 2})
        self.assertEqual(code, 0)
        self.assertEqual(self.calls, [])
        self.assertIn("unreachable  a.example.com", out)

    def test_disagreeing_sources_fail_and_change_nothing(self):
        code, _ = self.run_sync({"u@a": ["151.101.1.69", "151.101.1.1"], "u@b": ["93.184.216.34"] * 2})
        self.assertEqual(code, 1)
        self.assertEqual(self.calls, [])
        self.assertEqual(self.saved(), CONFIG)

    def test_a_failed_apply_fails_the_run(self):
        def failing(args):
            self.calls.append(args)
            return 1

        out = io.StringIO()
        with redirect_stdout(out):
            code = es.sync(self.path, lambda t: {"u@a": ["81.2.69.160"] * 2, "u@b": ["151.101.1.69"] * 2}[t], failing)
        self.assertEqual(code, 1)
        self.assertEqual(len(self.calls), 1)


if __name__ == "__main__":
    unittest.main()
