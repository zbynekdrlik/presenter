#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
cd "$REPO_ROOT"

: "${PRESENTER_DB_URL:=sqlite://$REPO_ROOT/var/data/dev/presenter_dev.db}"
: "${PRESENTER_PORT:=80}"

refresh() {
  "$REPO_ROOT/scripts/dev/refresh-dev-data.sh"
}

export PRESENTER_DB_URL
export PRESENTER_PORT

refresh

if command -v cargo-watch >/dev/null 2>&1; then
  CARGO_WATCH="cargo-watch"
else
  CARGO_WATCH="cargo watch"
fi

exec $CARGO_WATCH \
  --why \
  --clear \
  --ignore target \
  --ignore "var/data" \
  -s "PRESENTER_DB_URL=$PRESENTER_DB_URL PRESENTER_PORT=$PRESENTER_PORT $REPO_ROOT/scripts/dev/refresh-dev-data.sh" \
  -x "run -p presenter-server"
