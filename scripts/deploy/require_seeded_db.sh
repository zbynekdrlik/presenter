#!/usr/bin/env bash
# #918: a site deployed without the library sync (PRBA) gets its data ONLY from the one-time
# seed (scripts/ops/seed_prba_from_snv.sh). Its deploy must never create a fresh database —
# the new service would start empty and the seed would then refuse to run over it.
# Usage: require_seeded_db.sh <db_path>   (runs on the target host)
set -euo pipefail

DB_PATH="${1:?usage: require_seeded_db.sh <db_path>}"
if [ ! -s "$DB_PATH" ]; then
  echo "::error::database missing or empty at $DB_PATH — run scripts/ops/seed_prba_from_snv.sh first"
  exit 1
fi
echo "Seeded database present: $DB_PATH ($(stat -c %s "$DB_PATH") bytes)"
