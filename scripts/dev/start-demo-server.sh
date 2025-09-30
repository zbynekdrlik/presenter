#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
PID_FILE="$REPO_ROOT/.presenter-demo.pid"
LOG_DIR="$REPO_ROOT/logs"
LOG_FILE="$LOG_DIR/presenter-demo.log"

usage() {
  cat <<USAGE
Usage: $(basename "$0") [--stop]
  --stop    Stop the running Presenter demo server (if any)
USAGE
}

if [[ "${1-}" == "--help" ]]; then
  usage
  exit 0
fi

if [[ "${1-}" == "--stop" ]]; then
  if [[ -f "$PID_FILE" ]]; then
    pid=$(cat "$PID_FILE")
    if kill -0 "$pid" >/dev/null 2>&1; then
      sudo kill "$pid" || kill "$pid" || true
      echo "Stopped presenter demo server (PID $pid)."
    else
      echo "Presenter demo server not running (stale PID file)."
    fi
    rm -f "$PID_FILE"
  else
    echo "No presenter demo server PID file found."
  fi
  exit 0
fi

if [[ -f "$PID_FILE" ]]; then
  pid=$(cat "$PID_FILE")
  if kill -0 "$pid" >/dev/null 2>&1; then
    echo "Presenter demo server already running (PID $pid)."
    exit 0
  else
    echo "Removing stale PID file (PID $pid)."
    rm -f "$PID_FILE"
  fi
fi

mkdir -p "$LOG_DIR"
export PRESENTER_DB_URL="${PRESENTER_DB_URL:-sqlite://$REPO_ROOT/var/data/dev/presenter_dev.db}"
export PRESENTER_PORT="${PRESENTER_PORT:-80}"

cat <<MSG
▶ Starting Presenter demo server (background)
  • Database: $PRESENTER_DB_URL
  • Port:     $PRESENTER_PORT
  • URL:      http://localhost:${PRESENTER_PORT}/
  • Log:      $LOG_FILE
MSG

nohup env PRESENTER_DB_URL="$PRESENTER_DB_URL" PRESENTER_PORT="$PRESENTER_PORT" \
  "$REPO_ROOT/scripts/ops/run-env.sh" dev >"$LOG_FILE" 2>&1 &
PID=$!
echo $PID > "$PID_FILE"

echo "Presenter demo server running with PID $PID"
echo "Tail logs with: sudo tail -f $LOG_FILE"
