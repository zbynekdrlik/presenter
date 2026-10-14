#!/usr/bin/env python3
"""Self-tests for presenter_access.py (#850 spec §9) — pure functions, no network.

Run: python3 -m unittest discover -s ops/cloudflare -p 'test_presenter_access.py' -v
"""

from __future__ import annotations

import copy
import io
import json
import os
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from urllib.parse import parse_qs, urlparse

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import presenter_access as pa  # noqa: E402

EXAMPLE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "presenter-access.example.json")


def example_config() -> dict:
    with open(EXAMPLE, encoding="utf-8") as handle:
        return json.load(handle)


def live_apps(config: dict) -> list[dict]:
    """What GET /access/apps returns once the desired state is applied: ids added."""
    apps = copy.deepcopy(pa.desired_apps(config))
    for n, app in enumerate(apps):
        app["id"] = f"app-{n}"
        app["aud"] = f"aud-{n}"
        for m, policy in enumerate(app["policies"]):
            policy["id"] = f"pol-{n}-{m}"
            policy["created_at"] = "2026-10-13T12:00:00Z"
    return apps


class DesiredAppsTests(unittest.TestCase):
    def test_two_apps_per_host_with_bypass_then_allow(self):
        apps = pa.desired_apps(example_config())
        self.assertEqual([a["domain"] for a in apps], [
            "presenter-site-a.example.com", "presenter-site-a.example.com/healthz",
            "presenter-site-b.example.com", "presenter-site-b.example.com/healthz"])
        main = apps[0]
        self.assertEqual(main["session_duration"], "720h")
        self.assertEqual([(p["name"], p["decision"], p["precedence"]) for p in main["policies"]],
                         [(pa.BYPASS_NAME, "bypass", 1), (pa.ALLOW_NAME, "allow", 2)])
        self.assertEqual(main["policies"][0]["include"], [{"ip": {"ip": "192.0.2.10/32"}}])
        self.assertEqual(main["policies"][1]["include"], [{"email": {"email": "person@example.com"}}])
        self.assertEqual(apps[1]["session_duration"], "24h")
        self.assertEqual(apps[1]["policies"][0]["include"], [{"everyone": {}}])

    def test_a_site_without_bypass_ips_gets_only_the_allow_policy(self):
        config = example_config()
        config["sites"][0]["bypass_ips"] = []
        policies = pa.desired_apps(config)[0]["policies"]
        self.assertEqual([(p["name"], p["precedence"]) for p in policies], [(pa.ALLOW_NAME, 1)])

    def test_invalid_configs_are_refused(self):
        for broken in ({"allowed_emails": ["a@example.com"], "sites": [{"host": "h"}]},
                       {**example_config(), "allowed_emails": []},
                       {**example_config(), "sites": [{"host": "h", "bypass_ips": ["192.0.2.1"]}]}):
            with self.assertRaises(pa.ConfigError):
                pa.desired_apps(broken)


class PlanTests(unittest.TestCase):
    def test_applied_state_plans_only_ok(self):
        config = example_config()
        actions = pa.plan(live_apps(config), pa.desired_apps(config))
        self.assertEqual([a["op"] for a in actions], ["ok"] * 4)

    def test_missing_apps_are_created(self):
        config = example_config()
        actions = pa.plan(live_apps(config)[:2], pa.desired_apps(config))
        self.assertEqual([a["op"] for a in actions], ["ok", "ok", "create", "create"])

    def test_a_new_email_updates_the_allow_policy_in_place(self):
        config = example_config()
        existing = live_apps(config)
        config["allowed_emails"].append("second@example.com")
        actions = pa.plan(existing, pa.desired_apps(config))
        update = [a for a in actions if a["op"] == "update_policy"]
        self.assertEqual(len(update), 2)
        self.assertEqual(update[0]["policy_id"], "pol-0-1")
        self.assertEqual(update[0]["fields"], ["include"])

    def test_include_order_does_not_count_as_a_change(self):
        config = example_config()
        config["allowed_emails"] = ["a@example.com", "b@example.com"]
        existing = live_apps(config)
        existing[0]["policies"][1]["include"].reverse()
        self.assertEqual(pa.plan(existing, pa.desired_apps(config))[0]["op"], "ok")

    def test_session_change_keeps_the_existing_policies(self):
        config = example_config()
        existing = live_apps(config)
        config["session_duration"] = "336h"
        actions = pa.plan(existing, pa.desired_apps(config))
        update = [a for a in actions if a["op"] == "update_app"]
        self.assertEqual(len(update), 2)
        self.assertEqual(update[0]["body"]["session_duration"], "336h")
        self.assertEqual(update[0]["body"]["policies"], [{"id": "pol-0-0", "precedence": 1}, {"id": "pol-0-1", "precedence": 2}])
        self.assertNotIn("aud", update[0]["body"])

    def test_an_extra_policy_is_reported_never_deleted(self):
        config = example_config()
        existing = live_apps(config)
        existing[0]["policies"].append({"id": "pol-x", "name": "manual", "decision": "allow", "precedence": 3, "include": []})
        actions = pa.plan(existing, pa.desired_apps(config))
        self.assertEqual(actions[0]["op"], "extra_policy")
        self.assertTrue(pa.apply_action(actions[0], "acct", lambda *a: self.fail("extra_policy must not call the API")))


class OutputTests(unittest.TestCase):
    def test_describe_never_prints_emails_or_ips(self):
        config = example_config()
        existing = live_apps(config)
        config["allowed_emails"].append("second@example.com")
        for action in pa.plan(existing, pa.desired_apps(config)):
            line = pa.describe(action)
            self.assertNotIn("@", line)
            self.assertNotIn("192.0.2", line)

    def test_dry_run_writes_nothing(self):
        config = example_config()
        calls = []

        def fake(method, path, body):
            calls.append((method, path))
            return 200, {"success": True, "result": live_apps(config)[:2]}

        with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as handle:
            json.dump(config, handle)
        try:
            out = io.StringIO()
            with redirect_stdout(out):
                code = pa.main(["--config", handle.name], call=fake)
        finally:
            os.unlink(handle.name)
        self.assertEqual(code, 0)
        self.assertEqual([m for m, _ in calls], ["GET"])
        self.assertIn("would create", out.getvalue())

    def test_apply_posts_the_missing_apps(self):
        config = example_config()
        calls = []

        def fake(method, path, body):
            calls.append((method, path))
            if method == "GET":
                return 200, {"success": True, "result": live_apps(config)[:2]}
            return 200, {"success": True, "result": {"id": "new"}}

        with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as handle:
            json.dump(config, handle)
        try:
            with redirect_stdout(io.StringIO()):
                code = pa.main(["--config", handle.name, "--apply"], call=fake)
        finally:
            os.unlink(handle.name)
        self.assertEqual(code, 0)
        self.assertEqual(calls[1:], [("POST", "/accounts/<cloudflare-account-id>/access/apps")] * 2)


def run_main(config: dict, call, *flags: str) -> tuple[int, str, str]:
    """main() against a fake API: (exit code, stdout, stderr)."""
    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as handle:
        json.dump(config, handle)
    try:
        out, err = io.StringIO(), io.StringIO()
        with redirect_stdout(out), redirect_stderr(err):
            code = pa.main(["--config", handle.name, *flags], call=call)
    finally:
        os.unlink(handle.name)
    return code, out.getvalue(), err.getvalue()


def page_of(path: str) -> int:
    return int(parse_qs(urlparse(path).query)["page"][0])


class ListingTests(unittest.TestCase):
    def test_an_app_on_the_second_page_is_not_created_twice(self):
        config = example_config()
        foreign = {"id": "other", "domain": "other.example.com", "policies": []}
        pages = [[foreign], live_apps(config)]
        calls = []

        def fake(method, path, body):
            calls.append((method, path))
            if method != "GET":
                return 200, {"success": True, "result": {"id": "new"}}
            page = page_of(path)
            return 200, {"success": True, "result": pages[page - 1],
                         "result_info": {"page": page, "per_page": pa.PER_PAGE, "total_pages": len(pages)}}

        code, out, _ = run_main(config, fake, "--apply")
        self.assertEqual(code, 0)
        self.assertEqual([(m, page_of(p)) for m, p in calls], [("GET", 1), ("GET", 2)])
        self.assertNotIn("create", out)
        self.assertEqual([line.split()[0] for line in out.splitlines()], ["ok"] * 4)

    def test_a_single_page_without_result_info_is_one_request(self):
        config = example_config()
        calls = []

        def fake(method, path, body):
            calls.append(method)
            return 200, {"success": True, "result": live_apps(config)}

        code, _, _ = run_main(config, fake)
        self.assertEqual((code, calls), (0, ["GET"]))

    def test_a_runaway_page_count_fails_instead_of_looping(self):
        calls = []

        def fake(method, path, body):
            calls.append(method)
            return 200, {"success": True, "result": [], "result_info": {"total_pages": 10**6}}

        code, _, err = run_main(example_config(), fake, "--apply")
        self.assertEqual(code, 1)
        self.assertEqual(calls, ["GET"] * pa.MAX_PAGES)
        self.assertIn("pages of Access apps", err)

    def test_a_listing_error_stops_before_any_write(self):
        calls = []

        def fake(method, path, body):
            calls.append(method)
            return 403, {"success": False, "errors": [{"message": "denied"}]}

        code, _, err = run_main(example_config(), fake, "--apply")
        self.assertEqual((code, calls), (1, ["GET"]))
        self.assertIn("HTTP 403", err)


if __name__ == "__main__":
    unittest.main()
