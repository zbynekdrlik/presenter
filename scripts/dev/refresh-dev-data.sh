#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
export PRESENTER_DB_URL="${PRESENTER_DB_URL:-sqlite://$REPO_ROOT/var/data/dev/presenter_dev.db}"
ROOT_DIR="${1:-Propresenter library}"

if [[ "$PRESENTER_DB_URL" == sqlite://* ]]; then
  db_path="${PRESENTER_DB_URL#sqlite://}"
  echo "[refresh-dev-data] Removing existing SQLite database at $db_path"
  rm -f "$db_path" "$db_path-shm" "$db_path-wal"
  mkdir -p "$(dirname "$db_path")"
  touch "$db_path"
fi

echo "[refresh-dev-data] Importing ProPresenter libraries from '$ROOT_DIR'"
cargo run -p presenter-importer --bin import_propresenter -- "--root" "$ROOT_DIR"

echo "[refresh-dev-data] Importing default Bible translations"
cargo run -p presenter-importer --bin ingest_bibles
