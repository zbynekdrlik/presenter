#!/usr/bin/env python3
"""Fail when the WASM UI outgrows its committed budget (#850 B1c, spec §6.6 item 5).

A phone on the church Wi-Fi downloads the .wasm on every cold load, so its size
is a release criterion (S8/S9), not a curiosity. The budget is the first
measurement after wasm-opt plus 5 %; raising it is a reviewed edit of
ui_size_budget.json, never a silent drift. Both siblings count: Chrome offers
brotli only on HTTPS and on loopback, so a phone on the LAN (plain http) gets
the gzip one and only the public host gets brotli.

Usage: check_ui_size_budget.py <dist dir> [--budget <json>]
"""

import argparse
import json
import pathlib
import sys

DEFAULT_BUDGET = pathlib.Path(__file__).with_name("ui_size_budget.json")


def measure(dist: pathlib.Path) -> dict:
    """The raw, brotli and gzip sizes of the one .wasm in `dist`."""
    wasm = sorted(dist.glob("*.wasm"))
    if len(wasm) != 1:
        raise SystemExit(f"expected exactly one .wasm in {dist}, found {len(wasm)}")
    sizes = {"wasm_raw_bytes": wasm[0].stat().st_size}
    for key, suffix in (("wasm_br_bytes", ".br"), ("wasm_gz_bytes", ".gz")):
        sibling = wasm[0].with_name(wasm[0].name + suffix)
        if not sibling.is_file():
            raise SystemExit(f"{sibling} is missing: scripts/build-ui.sh precompresses every asset")
        sizes[key] = sibling.stat().st_size
    return sizes


def over_budget(measured: dict, budget: dict) -> list:
    """One line per measured size above its budget."""
    return [
        f"{key}: {measured[key]} > budget {budget[key]}"
        for key in sorted(budget)
        if measured.get(key, 0) > budget[key]
    ]


def main(argv: list) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("dist", type=pathlib.Path)
    parser.add_argument("--budget", type=pathlib.Path, default=DEFAULT_BUDGET)
    args = parser.parse_args(argv)
    measured = measure(args.dist)
    budget = json.loads(args.budget.read_text(encoding="utf-8"))
    print(json.dumps(measured))
    problems = over_budget(measured, budget)
    for line in problems:
        print(f"ERROR: UI size {line}", file=sys.stderr)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
