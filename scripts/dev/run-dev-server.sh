#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=scripts/dev/lib/local-cargo.sh
source "${SCRIPT_DIR}/lib/local-cargo.sh"

# Compiles and runs the server from source — Tier-0 opt-in only (#802). On this
# box use the deployed dev service or a CI-built target/release/presenter-server.
require_local_cargo run-dev-server "presenter-server (cargo run)" \
  "The deployed dev instance is http://10.77.8.134:8080 (presenter-dev.service)."

export PRESENTER_DB_URL="${PRESENTER_DB_URL:-sqlite://presenter_dev.db}"
export PRESENTER_PORT="${PRESENTER_PORT:-80}"

echo "▶ Launching presenter-server"
echo "  • Database: $PRESENTER_DB_URL"
echo "  • Port:     $PRESENTER_PORT"
echo "  • Operator UI: http://localhost:${PRESENTER_PORT}/ui/operator"
echo "  • Bible UI:    http://localhost:${PRESENTER_PORT}/ui/bible"
echo "  • Stage Output:http://localhost:${PRESENTER_PORT}/stage"
echo "  • Live feed:   ws://localhost:${PRESENTER_PORT}/live/ws"
echo "  • Menu:        http://localhost:${PRESENTER_PORT}/"
echo "    (binding to port 80 usually requires sudo or setcap)"

cargo run -p presenter-server "$@"
