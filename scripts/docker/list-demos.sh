#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/common.sh"

if [[ ! -d "$MANIFEST_DIR" ]]; then
  exit 0
fi

python3 - <<PY
import json
import os
from pathlib import Path

manifest_dir = Path("${MANIFEST_DIR}")
rows = []
for entry in sorted(manifest_dir.glob("*.json")):
    try:
        data = json.loads(entry.read_text())
    except Exception:
        continue
    rows.append(
        (
            data.get("project", entry.stem),
            str(data.get("port", "?")),
            data.get("updatedAt", "?"),
            data.get("url", ""),
        )
    )

if not rows:
    exit()

print(f"{'PROJECT':30} {'PORT':8} {'UPDATED':30} URL")
for project, port, updated, url in rows:
    print(f"{project:30} {port:8} {updated:30} {url}")
PY
