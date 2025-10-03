#!/usr/bin/env bash
set -euo pipefail

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
