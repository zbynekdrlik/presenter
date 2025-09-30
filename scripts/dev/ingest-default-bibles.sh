#!/usr/bin/env bash
set -euo pipefail

export PRESENTER_DB_URL="${PRESENTER_DB_URL:-sqlite://presenter_dev.db}"

if [[ "$PRESENTER_DB_URL" == sqlite://* ]]; then
  db_path="${PRESENTER_DB_URL#sqlite://}"
  echo "[ingest-default-bibles] Removing existing SQLite database at $db_path"
  rm -f "$db_path" "$db_path-shm" "$db_path-wal"
  mkdir -p "$(dirname "$db_path")"
  touch "$db_path"
fi

cargo run -p presenter-importer --bin ingest_bibles "$@"
