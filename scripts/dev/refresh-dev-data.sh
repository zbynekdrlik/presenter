#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
REPO_PARENT="$(cd "${REPO_ROOT}/.." && pwd)"
DEFAULT_LIB_ROOT="${PRESENTER_LIBRARY_ROOT:-${REPO_PARENT}/presenter-libraries}"
export PRESENTER_DB_URL="${PRESENTER_DB_URL:-sqlite://$REPO_ROOT/var/data/dev/presenter_dev.db}"
ROOT_DIR="${1:-$DEFAULT_LIB_ROOT}"

# Use pre-built binaries if available (CI builds them first), otherwise fall back to cargo run.
# A pre-built binary OLDER than the importer's source tree is a trap (#559): it silently runs
# yesterday's code against today's schema. Detect staleness and rebuild via cargo instead.
binary_is_stale() {
  local binary="$1"
  find "${REPO_ROOT}/crates/presenter-importer" \
    "${REPO_ROOT}/crates/presenter-persistence" \
    "${REPO_ROOT}/crates/presenter-migration" \
    "${REPO_ROOT}/crates/presenter-core" \
    -name '*.rs' -newer "$binary" -print -quit 2>/dev/null | grep -q .
}

run_binary() {
  local bin_name="$1"
  shift
  local debug_binary="${REPO_ROOT}/target/debug/${bin_name}"
  local release_binary="${REPO_ROOT}/target/release/${bin_name}"
  local candidate=""

  if [[ -x "$release_binary" ]]; then
    candidate="$release_binary"
  elif [[ -x "$debug_binary" ]]; then
    candidate="$debug_binary"
  fi

  if [[ -n "$candidate" ]] && binary_is_stale "$candidate"; then
    echo "[refresh-dev-data] WARNING: $candidate is older than the importer sources — rebuilding via cargo run instead of trusting a stale binary" >&2
    candidate=""
  fi

  if [[ -n "$candidate" ]]; then
    "$candidate" "$@"
  else
    cargo run -p presenter-importer --bin "$bin_name" -- "$@"
  fi
}

if [[ "$PRESENTER_DB_URL" == sqlite://* ]]; then
  db_path="${PRESENTER_DB_URL#sqlite://}"
  echo "[refresh-dev-data] Removing existing SQLite database at $db_path"
  rm -f "$db_path" "$db_path-shm" "$db_path-wal"
  mkdir -p "$(dirname "$db_path")"
  touch "$db_path"
fi

echo "[refresh-dev-data] Importing ProPresenter libraries from '$ROOT_DIR'"
run_binary import_propresenter "--root" "$ROOT_DIR"

echo "[refresh-dev-data] Importing default Bible translations"
run_binary ingest_bibles
