#!/usr/bin/env bash
set -euo pipefail

REPORT_DIR="playwright-report"

usage() {
  cat <<'EOF'
Usage: scripts/dev/show-playwright-report.sh [report-directory]

Opens the Playwright HTML report on a random free localhost port. The script
kills any orphaned `playwright show-report` Node processes before launching so
we never collide with a leftover server.
EOF
}

if [[ "${1-}" == "-h" || "${1-}" == "--help" ]]; then
  usage
  exit 0
fi

if [[ $# -gt 0 ]]; then
  REPORT_DIR="$1"
fi

# Clean up any previous stuck show-report instances.
if pgrep -f "playwright show-report" >/dev/null 2>&1; then
  echo "[show-report] terminating orphaned playwright show-report processes"
  pkill -f "playwright show-report" || true
fi

# Find a free localhost port.
PORT=$(python3 - <<'PY'
import socket
sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
sock.bind(("127.0.0.1", 0))
port = sock.getsockname()[1]
sock.close()
print(port)
PY
)

echo "[show-report] launching on http://127.0.0.1:${PORT} (report: ${REPORT_DIR})"

# Run detached so the CLI is never blocked by the viewer server.
nohup npx playwright show-report "${REPORT_DIR}" --host 127.0.0.1 --port "${PORT}" \
  >/dev/null 2>&1 &
echo "[show-report] viewer started in background (pid=$!). Use: pkill -f 'playwright show-report' to stop."
