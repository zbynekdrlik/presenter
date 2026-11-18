#!/usr/bin/env python3
"""Keep each site's Access bypass IP equal to the site's real public address (#977).

A site whose ISP hands out a new public address loses its no-login bypass: every
device there gets the Cloudflare e-mail + PIN page (PP, 18.11.2026). For every site in
the Access config with an ``egress_ssh`` target (``user@host`` of a box ON that site's
network), this asks two independent "what is my IP" services over ssh; when both agree
and the address differs from the site's ``bypass_ips``, it rewrites the config
(atomically, mode 0600) and runs ``presenter_access.py --apply``, then a dry run that
must print only ``ok``. An unreachable box (the site PC is off) is logged and skipped.

Run from a timer with the token from the vault, exactly like presenter_access.py:

    python3 ~/devel/airuleset/airuleset.py secret exec \\
      --file <vault>/cloudflare-presenter-access --env CF_TOKEN -- \\
      python3 ops/cloudflare/egress_sync.py \\
      --config ~/.config/presenter/presenter-access.json
"""

from __future__ import annotations

import argparse
import copy
import ipaddress
import json
import os
import subprocess
import sys
import tempfile
from typing import Any, Callable

import presenter_access

IP_SOURCES = ("https://api.ipify.org", "https://ipv4.icanhazip.com")
SSH_OPTS = ["-o", "BatchMode=yes", "-o", "ConnectTimeout=10"]

Probe = Callable[[str], "list[str] | None"]
Reconcile = Callable[["list[str]"], int]


def parse_ipv4(text: str) -> str | None:
    """The public IPv4 address in a service's answer, or None for anything else."""
    try:
        addr = ipaddress.IPv4Address(text.strip())
    except ValueError:
        return None
    return None if addr.is_private or addr.is_loopback else str(addr)


def agreed_address(answers: list[str]) -> str | None:
    """One address only when every source answered the same public IPv4."""
    parsed = {parse_ipv4(answer) for answer in answers}
    if len(answers) != len(IP_SOURCES) or len(parsed) != 1 or None in parsed:
        return None
    return parsed.pop()


def with_new_bypass(config: dict[str, Any], host: str, address: str) -> dict[str, Any]:
    """A copy of the config whose site ``host`` lets ``address`` through, nothing else."""
    updated = copy.deepcopy(config)
    for site in updated["sites"]:
        if site["host"] == host:
            site["bypass_ips"] = [f"{address}/32"]
    return updated


def ssh_probe(target: str) -> list[str] | None:
    """Ask every IP source from the site box; None when the box does not answer."""
    script = "; ".join(f"curl -s --max-time 8 {url}; echo" for url in IP_SOURCES)
    try:
        done = subprocess.run(["ssh", *SSH_OPTS, target, script], capture_output=True,
                              text=True, timeout=60, check=False)
    except subprocess.TimeoutExpired:
        return None
    if done.returncode != 0:
        return None
    return done.stdout.splitlines()[: len(IP_SOURCES)]


def write_config(path: str, config: dict[str, Any]) -> None:
    """Replace the config file in one rename, owner-only, never half-written."""
    folder = os.path.dirname(path)
    fd, tmp = tempfile.mkstemp(dir=folder, prefix=".presenter-access.")
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            json.dump(config, handle, indent=1)
            handle.write("\n")
        os.chmod(tmp, 0o600)
        os.replace(tmp, path)
    except BaseException:
        os.unlink(tmp)
        raise


def sync(path: str, probe: Probe, reconcile: Reconcile) -> int:
    with open(path, encoding="utf-8") as handle:
        config = json.load(handle)
    changed = []
    for site in config["sites"]:
        target = site.get("egress_ssh")
        if not target:
            continue
        answers = probe(target)
        if answers is None:
            print(f"unreachable  {site['host']}  (site box did not answer, bypass kept)")
            continue
        address = agreed_address(answers)
        if address is None:
            print(f"::error::{site['host']}: the IP sources disagree or gave no address")
            return 1
        if site.get("bypass_ips") == [f"{address}/32"]:
            print(f"ok           {site['host']}  egress unchanged")
            continue
        print(f"::warning::{site['host']}: public address changed, updating its bypass")
        config = with_new_bypass(config, site["host"], address)
        changed.append(site["host"])
    if not changed:
        return 0
    presenter_access.validate_config(config)
    write_config(path, config)
    if reconcile(["--config", path, "--apply"]) != 0:
        print("::error::applying the new bypass failed")
        return 1
    return reconcile(["--config", path])


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--config", default=os.environ.get("PRESENTER_ACCESS_CONFIG"))
    args = parser.parse_args(argv)
    if not args.config:
        print("::error::set PRESENTER_ACCESS_CONFIG or pass --config", file=sys.stderr)
        return 2
    return sync(os.path.expanduser(args.config), ssh_probe, presenter_access.main)


if __name__ == "__main__":
    sys.exit(main())
