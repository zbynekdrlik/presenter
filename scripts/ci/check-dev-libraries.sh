#!/usr/bin/env bash
# Verify that the dev server's database contains a library row for every
# library directory on disk. Fails the build when the counts disagree, so a
# seed-only DB or a half-failed import can no longer pass the smoke test
# (regression guard for issue #229).
set -euo pipefail

if [ "$#" -ne 2 ]; then
    echo "usage: $0 <libraries-disk-path> <summary-url-or-json-file>" >&2
    exit 2
fi

LIB_PATH="$1"
SUMMARY_SOURCE="$2"

if [ ! -d "$LIB_PATH" ]; then
    echo "::error::Libraries path does not exist: $LIB_PATH" >&2
    exit 1
fi

DISK_COUNT=$(find "$LIB_PATH" -mindepth 1 -maxdepth 1 -type d ! -name 'LibraryData' | wc -l)

if [ -f "$SUMMARY_SOURCE" ]; then
    JSON=$(cat "$SUMMARY_SOURCE")
else
    JSON=$(curl -sf "$SUMMARY_SOURCE")
fi

DB_COUNT=$(printf '%s' "$JSON" | python3 -c "import json,sys; data=json.load(sys.stdin); print(len([l for l in data if l.get('name','').strip().lower()!='bible']))")

if [ "$DISK_COUNT" -ne "$DB_COUNT" ]; then
    echo "::error::Library count mismatch: disk=$DISK_COUNT db=$DB_COUNT (DB may be seed-only or import failed)" >&2
    exit 1
fi

echo "Library count check passed: $DB_COUNT libraries match disk"
