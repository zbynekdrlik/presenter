#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

cd "$REPO_ROOT"

BRANCH_NAME="$(git rev-parse --abbrev-ref HEAD)"
BRANCH_SLUG="$(echo "$BRANCH_NAME" | tr '[:upper:]' '[:lower:]' | sed -E 's/[^a-z0-9]+/-/g; s/^-+|-+$//g')"

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

RUN_ARGS=("--display-name" "$DISPLAY_NAME")
if [[ "$FORCE_REBUILD" -eq 1 ]]; then
  RUN_ARGS=("--force" "${RUN_ARGS[@]}")
fi

echo "[verify] Refreshing Docker demo for project '$BRANCH_SLUG' (pre-tests)"
"$REPO_ROOT/scripts/docker/run-demo.sh" "${RUN_ARGS[@]}"

echo "[verify] Restarting gateway"
"$REPO_ROOT/scripts/docker/run-gateway.sh"

echo "[verify] Running Playwright suite"
npm run test:playwright

echo "[verify] Refreshing Docker demo for project '$BRANCH_SLUG' (post-tests)"
"$REPO_ROOT/scripts/docker/run-demo.sh" "--display-name" "$DISPLAY_NAME"

echo "[verify] Restarting gateway (post-tests)"
"$REPO_ROOT/scripts/docker/run-gateway.sh"

echo "[verify] ✔ Completed. Demo card should now reflect: $DISPLAY_NAME"
