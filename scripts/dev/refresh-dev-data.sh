#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
REPO_PARENT="$(cd "${REPO_ROOT}/.." && pwd)"
DEFAULT_LIB_ROOT="${PRESENTER_LIBRARY_ROOT:-${REPO_PARENT}/presenter-libraries}"
export PRESENTER_DB_URL="${PRESENTER_DB_URL:-sqlite://$REPO_ROOT/var/data/dev/presenter_dev.db}"
ROOT_DIR="${1:-$DEFAULT_LIB_ROOT}"

# Prebuilt binaries ONLY (#802, Tier-0: Rust compiles in CI only). CI places
# them in target/release/ before E2E; locally, download the CI artifact. A
# binary OLDER than the importer's source tree is a trap (#559): it silently
# runs yesterday's code against today's schema — so a stale binary is refused
# exactly like a missing one. Compiling locally is an explicit opt-in:
# PRESENTER_ALLOW_LOCAL_CARGO=1.
binary_is_stale() {
  local binary="$1"
  find "${REPO_ROOT}/crates/presenter-importer" \
    "${REPO_ROOT}/crates/presenter-persistence" \
    "${REPO_ROOT}/crates/presenter-migration" \
    "${REPO_ROOT}/crates/presenter-core" \
    -name '*.rs' -newer "$binary" -print -quit 2>/dev/null | grep -q .
}

refuse_local_compile() {
  local reason="$1"
  cat >&2 <<EOF
[refresh-dev-data] ERROR: ${reason}
[refresh-dev-data] Presenter is Tier-0: Rust compiles in CI only, this script never builds locally (#802).
[refresh-dev-data] Download the CI build artifact for your commit into target/release/:
[refresh-dev-data]   gh run download <run-id> -n build-artifacts -D _artifacts
[refresh-dev-data]   mkdir -p target/release && cp _artifacts/* target/release/ && chmod +x target/release/*
[refresh-dev-data] (find <run-id> with: gh run list -w pipeline.yml -b dev -L 5)
[refresh-dev-data] Explicit local-compile opt-in (Tier-1/2 boxes only): PRESENTER_ALLOW_LOCAL_CARGO=1
EOF
  exit 1
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

  if [[ -n "$candidate" ]] && ! binary_is_stale "$candidate"; then
    "$candidate" "$@"
    return
  fi

  if [[ "${PRESENTER_ALLOW_LOCAL_CARGO:-}" == "1" ]]; then
    echo "[refresh-dev-data] PRESENTER_ALLOW_LOCAL_CARGO=1 — compiling ${bin_name} locally (Tier-0 opt-in)" >&2
    cargo run -p presenter-importer --bin "$bin_name" -- "$@"
    return
  fi

  if [[ -n "$candidate" ]]; then
    refuse_local_compile "$candidate is older than the importer sources (stale, #559)"
  fi
  refuse_local_compile "prebuilt binary '${bin_name}' not found (expected ${release_binary} or ${debug_binary})"
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
