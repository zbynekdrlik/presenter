#!/usr/bin/env bash
set -euo pipefail

log() {
  echo "[demo-entrypoint] $*"
}

DATA_DIR="${PRESENTER_DATA_DIR:-/data}"
IMPORT_ROOT="${PRESENTER_IMPORT_ROOT:-/imports}"
DB_URL="${PRESENTER_DB_URL:-sqlite:///data/presenter.db}"
PORT="${PRESENTER_PORT:-8080}"
FORCE_IMPORT="${PRESENTER_FORCE_IMPORT:-0}"
SKIP_IMPORT="${PRESENTER_SKIP_IMPORT:-0}"
MARKER_FILE="${DATA_DIR}/.presenter_import_complete"

if [[ "$DB_URL" == sqlite://* ]]; then
  DB_PATH="${DB_URL#sqlite://}"
  mkdir -p "$(dirname "$DB_PATH")"
  touch "$DB_PATH"
fi

if [[ "$SKIP_IMPORT" != "1" ]]; then
  if [[ "$FORCE_IMPORT" == "1" || ! -f "$MARKER_FILE" ]]; then
    if [[ -d "$IMPORT_ROOT" ]]; then
      log "Importing ProPresenter libraries from $IMPORT_ROOT"
      import_propresenter --root "$IMPORT_ROOT"
    else
      log "Import root $IMPORT_ROOT missing; skipping slide import"
    fi

    log "Importing default Bible translations"
    ingest_bibles

    mkdir -p "$(dirname "$MARKER_FILE")"
    date --iso-8601=seconds > "$MARKER_FILE"
  else
    log "Dataset already imported (marker present). Use PRESENTER_FORCE_IMPORT=1 to refresh."
  fi
else
  log "Skipping import per PRESENTER_SKIP_IMPORT=1"
fi

log "Starting presenter-server on port ${PORT}"
exec presenter-server "$@"
