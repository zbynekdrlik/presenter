#!/usr/bin/env python3
"""Reconcile the Cloudflare Access apps of the public Presenter hosts (#850 spec §9).

Desired state = the JSON file named by --config (or PRESENTER_ACCESS_CONFIG): account
id, allowed e-mails, hosts and the on-site bypass IPs. Only
``presenter-access.example.json`` (placeholders) is committed — the repo is public; the
real file lives outside it (dev2: ``~/.config/presenter/presenter-access.json``, 0600).

Dry run by default; ``--apply`` writes. Creates missing apps, updates the session
duration and the policies of existing apps, never deletes anything (an extra policy
is reported). Prints app ids, domains and policy names only — never the token,
e-mails or IPs. The token reaches the script from the vault as CF_TOKEN:

    python3 ~/devel/airuleset/airuleset.py secret exec \\
      --file <vault>/cloudflare-presenter-access --env CF_TOKEN -- \\
      python3 ops/cloudflare/presenter_access.py \\
      --config ~/.config/presenter/presenter-access.json [--apply]
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import urllib.error
import urllib.request
from typing import Any, Callable

API = "https://api.cloudflare.com/client/v4"
APP_DEFAULTS = {
    "type": "self_hosted",
    "auto_redirect_to_identity": False,
    "allowed_idps": [],
    "app_launcher_visible": False,
    "http_only_cookie_attribute": True,
}
APP_KEYS = ("name", "domain", "type", "session_duration", *APP_DEFAULTS)
BYPASS_NAME = "presenter on-site network (no login)"
ALLOW_NAME = "presenter allowed users"
HEALTHZ_NAME = "presenter healthz probe bypass"

Call = Callable[[str, str, "dict[str, Any] | None"], "tuple[int, dict[str, Any]]"]


class ConfigError(ValueError):
    """The Access config is missing a key or holds an unusable value."""


def validate_config(config: dict[str, Any]) -> None:
    for key in ("account_id", "allowed_emails", "sites"):
        if not config.get(key):
            raise ConfigError(f"config needs a non-empty {key!r}")
    for site in config["sites"]:
        if not site.get("host"):
            raise ConfigError("every site needs a 'host'")
        for cidr in site.get("bypass_ips", []):
            if "/" not in cidr:
                raise ConfigError(f"bypass IP of {site['host']} needs a CIDR suffix such as /32")


def desired_apps(config: dict[str, Any]) -> list[dict[str, Any]]:
    """The Access apps the config asks for: per host one app + one /healthz bypass app."""
    validate_config(config)
    apps = []
    for site in config["sites"]:
        host = site["host"]
        policies = []
        if site.get("bypass_ips"):
            policies.append({"name": BYPASS_NAME, "decision": "bypass",
                             "include": [{"ip": {"ip": cidr}} for cidr in site["bypass_ips"]]})
        policies.append({"name": ALLOW_NAME, "decision": "allow",
                         "include": [{"email": {"email": e}} for e in config["allowed_emails"]]})
        for precedence, policy in enumerate(policies, start=1):
            policy["precedence"] = precedence
        apps.append({**APP_DEFAULTS, "name": f"presenter — {host}", "domain": host,
                     "session_duration": config.get("session_duration", "720h"), "policies": policies})
        apps.append({**APP_DEFAULTS, "name": f"presenter healthz — {host}", "domain": f"{host}/healthz",
                     "session_duration": config.get("healthz_session_duration", "24h"),
                     "policies": [{"name": HEALTHZ_NAME, "decision": "bypass", "precedence": 1,
                                   "include": [{"everyone": {}}]}]})
    return apps


def normalize_policy(policy: dict[str, Any]) -> dict[str, Any]:
    include = sorted(json.dumps(rule, sort_keys=True) for rule in policy.get("include") or [])
    return {"decision": policy.get("decision"), "precedence": policy.get("precedence"), "include": include}


def changed_fields(have: dict[str, Any], want: dict[str, Any]) -> list[str]:
    a, b = normalize_policy(have), normalize_policy(want)
    return [field for field in ("decision", "precedence", "include") if a[field] != b[field]]


def plan(existing: list[dict[str, Any]], desired: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """Actions that bring ``existing`` (GET /access/apps) to ``desired``; pure, no network."""
    by_domain = {app.get("domain"): app for app in existing}
    actions: list[dict[str, Any]] = []
    for want in desired:
        have = by_domain.get(want["domain"])
        if have is None:
            actions.append({"op": "create", "domain": want["domain"], "app": want})
            continue
        app_actions = []
        if have.get("session_duration") != want["session_duration"]:
            body = {key: have[key] for key in APP_KEYS if key in have}
            body["session_duration"] = want["session_duration"]
            body["policies"] = [{"id": p["id"], "precedence": p.get("precedence")} for p in have.get("policies") or []]
            app_actions.append({"op": "update_app", "domain": want["domain"], "app_id": have["id"], "body": body})
        have_policies = {p.get("name"): p for p in have.get("policies") or []}
        for policy in want["policies"]:
            current = have_policies.get(policy["name"])
            if current is None:
                app_actions.append({"op": "create_policy", "domain": want["domain"], "app_id": have["id"],
                                    "name": policy["name"], "policy": policy})
            elif changed_fields(current, policy):
                app_actions.append({"op": "update_policy", "domain": want["domain"], "app_id": have["id"],
                                    "policy_id": current["id"], "name": policy["name"], "policy": policy,
                                    "fields": changed_fields(current, policy)})
        wanted_names = {p["name"] for p in want["policies"]}
        for name in sorted(n for n in have_policies if n not in wanted_names):
            app_actions.append({"op": "extra_policy", "domain": want["domain"], "app_id": have["id"], "name": name})
        actions.extend(app_actions or [{"op": "ok", "domain": want["domain"], "app_id": have["id"]}])
    return actions


def describe(action: dict[str, Any]) -> str:
    """One output line: op, domain, ids, policy name and changed field names — no values."""
    parts = [f"{action['op']:<14}", action["domain"]]
    if "app_id" in action:
        parts.append(f"app={action['app_id']}")
    if "name" in action:
        parts.append(f"policy={action['name']!r}")
    if action.get("fields"):
        parts.append("changed=" + ",".join(action["fields"]))
    if action["op"] == "extra_policy":
        parts.append("(kept — delete it in the dashboard if it is unwanted)")
    return "  ".join(parts)


def apply_action(action: dict[str, Any], account: str, call: Call) -> bool:
    base = f"/accounts/{account}/access/apps"
    requests = {
        "create": ("POST", base, action.get("app")),
        "update_app": ("PUT", f"{base}/{action.get('app_id')}", action.get("body")),
        "create_policy": ("POST", f"{base}/{action.get('app_id')}/policies", action.get("policy")),
        "update_policy": ("PUT", f"{base}/{action.get('app_id')}/policies/{action.get('policy_id')}", action.get("policy")),
    }
    if action["op"] not in requests:
        return True
    method, path, body = requests[action["op"]]
    code, result = call(method, path, body)
    return code in (200, 201) and bool(result.get("success"))


def http_call(method: str, path: str, body: dict[str, Any] | None = None) -> tuple[int, dict[str, Any]]:
    token = os.environ["CF_TOKEN"].strip()
    data = json.dumps(body).encode() if body is not None else None
    request = urllib.request.Request(API + path, data=data, method=method,
                                     headers={"Authorization": "Bearer " + token, "Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            return response.status, json.load(response)
    except urllib.error.HTTPError as err:
        try:
            return err.code, json.load(err)
        except ValueError:
            return err.code, {}


def main(argv: list[str] | None = None, call: Call = http_call) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--apply", action="store_true", help="write the changes (default: dry run)")
    parser.add_argument("--config", default=os.environ.get("PRESENTER_ACCESS_CONFIG"))
    args = parser.parse_args(argv)
    if not args.config:
        print("::error::set PRESENTER_ACCESS_CONFIG or pass --config", file=sys.stderr)
        return 2
    try:
        with open(os.path.expanduser(args.config), encoding="utf-8") as handle:
            config = json.load(handle)
        desired = desired_apps(config)
    except (OSError, ValueError) as err:
        print(f"::error::access config: {err}", file=sys.stderr)
        return 2
    code, body = call("GET", f"/accounts/{config['account_id']}/access/apps?per_page=100", None)
    if code != 200:
        print(f"::error::listing Access apps failed: HTTP {code} {[e.get('message') for e in body.get('errors', [])]}",
              file=sys.stderr)
        return 1
    failures = 0
    for action in plan(body.get("result") or [], desired):
        line = describe(action)
        if action["op"] in ("ok", "extra_policy"):
            print(line)
            continue
        if not args.apply:
            print("would " + line)
            continue
        ok = apply_action(action, config["account_id"], call)
        failures += 0 if ok else 1
        print(("done   " if ok else "FAILED ") + line)
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
