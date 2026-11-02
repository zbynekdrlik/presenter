#!/usr/bin/env python3
"""Fetch and verify the public study data in data/study/ (#851 D3a, D3b).

  fetch_study_sources.py          download, verify against sources.json, write
  fetch_study_sources.py --check  verify the committed files, offline (CI)

Hashes are of the upstream bytes: strongs.zip is built here, so its members are
what is pinned; the cross-references zip is upstream's own file. OpenBible
republishes that one under the same URL, so a changed file fails until
sources.json is updated on purpose.
"""

from __future__ import annotations

import argparse
import hashlib
import io
import json
import sys
import urllib.request
import zipfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
STUDY_DIR = REPO_ROOT / "data" / "study"
ZIP_EPOCH = (1980, 1, 1, 0, 0, 0)


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def load_sources(study_dir: Path) -> dict:
    return json.loads((study_dir / "sources.json").read_text(encoding="utf-8"))


def check_committed(study_dir: Path) -> list[str]:
    """Every mismatch between sources.json and the files, as messages."""
    problems: list[str] = []
    entries = load_sources(study_dir)["files"]
    listed = {entry["file"] for entry in entries} | {"sources.json"}
    for path in sorted(study_dir.iterdir()):
        if path.is_file() and path.name not in listed:
            problems.append(f"{path.name}: not listed in sources.json")
    for entry in entries:
        path = study_dir / entry["file"]
        if "sha256" not in entry and not entry.get("members"):
            problems.append(f"{entry['file']}: no hash in sources.json")
            continue
        if not path.is_file():
            problems.append(f"{entry['file']}: missing")
            continue
        if "sha256" in entry and sha256(path.read_bytes()) != entry["sha256"]:
            problems.append(f"{entry['file']}: SHA-256 differs from sources.json")
        members = entry.get("members", [])
        if not members:
            continue
        with zipfile.ZipFile(path) as archive:
            for member in members:
                name = member["name"]
                if name not in archive.namelist():
                    problems.append(f"{entry['file']}: {name} missing")
                elif sha256(archive.read(name)) != member["sha256"]:
                    problems.append(f"{entry['file']}: {name} differs from sources.json")
    return problems


def download(url: str) -> bytes:
    with urllib.request.urlopen(url, timeout=60) as response:
        return response.read()


def verified(url: str, expected: str, label: str) -> bytes:
    data = download(url)
    if sha256(data) != expected:
        raise SystemExit(f"{label}: upstream bytes differ from sources.json ({url})")
    return data


def fetch(study_dir: Path) -> None:
    """Everything is downloaded and verified before the first file is touched."""
    built: list[tuple[Path, bytes]] = []
    for entry in load_sources(study_dir)["files"]:
        if "members" in entry:
            buffer = io.BytesIO()
            with zipfile.ZipFile(buffer, "w", zipfile.ZIP_DEFLATED) as archive:
                for member in entry["members"]:
                    data = verified(member["url"], member["sha256"], member["name"])
                    info = zipfile.ZipInfo(member["name"], ZIP_EPOCH)
                    info.compress_type = zipfile.ZIP_DEFLATED
                    archive.writestr(info, data)
            built.append((study_dir / entry["file"], buffer.getvalue()))
        else:
            data = verified(entry["url"], entry["sha256"], entry["file"])
            built.append((study_dir / entry["file"], data))
    for path, data in built:
        path.write_bytes(data)
        print(f"wrote {path}")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="verify offline")
    args = parser.parse_args(argv)
    if not args.check:
        fetch(STUDY_DIR)
    problems = check_committed(STUDY_DIR)
    for problem in problems:
        print(f"::error::{problem}", file=sys.stderr)
    if problems:
        return 1
    print("study sources OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
