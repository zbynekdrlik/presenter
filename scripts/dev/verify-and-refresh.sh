#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

cd "$REPO_ROOT"

BRANCH_NAME="$(git rev-parse --abbrev-ref HEAD)"
BRANCH_SLUG="$(echo "$BRANCH_NAME" | tr '[:upper:]' '[:lower:]' | sed -E 's/[^a-z0-9]+/-/g; s/^-+|-+$//g')"
REPO_SLUG="$(basename "$REPO_ROOT")"
REPO_SLUG="$(echo "$REPO_SLUG" | tr '[:upper:]' '[:lower:]' | sed -E 's/[^a-z0-9]+/-/g; s/^-+|-+$//g')"
PROJECT_SLUG="$REPO_SLUG"
PORT_HASH=$(printf '%s' "$REPO_SLUG" | md5sum | cut -c1-4)
DEMO_PORT=$((0x$PORT_HASH))
DEMO_PORT=$((18000 + (DEMO_PORT % 1000)))
DEMO_ORIGIN="http://127.0.0.1:${DEMO_PORT}"
GATEWAY_URL="http://127.0.0.1/"

DISPLAY_NAME_DEFAULT="${BRANCH_NAME}"
FORCE_REBUILD=0
CUSTOM_DISPLAY_NAME=""

usage() {
  cat <<USAGE
Usage: $(basename "$0") [options]

Runs cargo tests, Playwright tests, then rebuilds and refreshes the branch demo and gateway.

Options:
  --display-name NAME  Friendly name for the demo card (defaults to current branch name)
  --force              Force Docker image rebuild even if unchanged
  -h, --help           Show this help message
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --display-name)
      CUSTOM_DISPLAY_NAME="$2"; shift 2 ;;
    --force)
      FORCE_REBUILD=1; shift ;;
    -h|--help)
      usage; exit 0 ;;
    *)
      echo "Unknown option: $1" >&2
      usage
      exit 1 ;;
  esac
done

DISPLAY_NAME="${CUSTOM_DISPLAY_NAME:-$DISPLAY_NAME_DEFAULT}"

echo "[verify] Running cargo test"
cargo test

RUN_ARGS=("--name" "$REPO_SLUG" "--display-name" "$DISPLAY_NAME" "--port" "$DEMO_PORT")
if [[ "$FORCE_REBUILD" -eq 1 ]]; then
  RUN_ARGS=("--force" "${RUN_ARGS[@]}")
fi

echo "[verify] Refreshing Docker demo for project '$REPO_SLUG' (pre-tests)"
"$REPO_ROOT/scripts/docker/run-demo.sh" "${RUN_ARGS[@]}"

echo "[verify] Restarting gateway"
"$REPO_ROOT/scripts/docker/run-gateway.sh"

echo "[verify] Running Playwright suite"
npm run test:playwright

echo "[verify] Refreshing Docker demo for project '$REPO_SLUG' (post-tests)"
"$REPO_ROOT/scripts/docker/run-demo.sh" "--name" "$REPO_SLUG" "--display-name" "$DISPLAY_NAME" "--port" "$DEMO_PORT"

echo "[verify] Restarting gateway (post-tests)"
"$REPO_ROOT/scripts/docker/run-gateway.sh"

echo "[verify] Checking demo operator UI at ${DEMO_ORIGIN}/ui/operator"
curl --fail --silent --show-error "${DEMO_ORIGIN}/ui/operator" >/dev/null

echo "[verify] Checking gateway card for ${PROJECT_SLUG}"
ATTEMPTS=0
until curl --fail --silent --show-error "${GATEWAY_URL}" | grep -q "data-project=\"${PROJECT_SLUG}\""; do
  ATTEMPTS=$((ATTEMPTS + 1))
  if [[ $ATTEMPTS -ge 10 ]]; then
    echo "[verify] Gateway card for ${PROJECT_SLUG} not found" >&2
    exit 1
  fi
  sleep 1
  echo "[verify] Waiting for gateway card ${PROJECT_SLUG} (attempt ${ATTEMPTS})"
done

echo "[verify] ✔ Completed. Demo card should now reflect: $DISPLAY_NAME"
