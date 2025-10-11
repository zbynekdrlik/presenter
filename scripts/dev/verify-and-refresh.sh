#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

cd "$REPO_ROOT"

if [[ $EUID -ne 0 ]]; then
  echo "[verify] This helper must be launched via sudo -E ./scripts/dev/verify-and-refresh.sh" >&2
  exit 1
fi

if [[ -z "${SUDO_USER:-}" ]]; then
  echo "[verify] This helper must be launched via sudo -E ./scripts/dev/verify-and-refresh.sh" >&2
  echo "[verify] (Docker requires elevated access; tests still run as the original user.)" >&2
  exit 1
fi

ORIGINAL_USER="$SUDO_USER"
ORIGINAL_HOME="$(getent passwd "$ORIGINAL_USER" | cut -d: -f6)"
NVM_DIR_DEFAULT="${ORIGINAL_HOME}/.nvm"
ORIGINAL_PATH="$(sudo -H -u "$ORIGINAL_USER" HOME="$ORIGINAL_HOME" NVM_DIR="$NVM_DIR_DEFAULT" bash -lc 'source ~/.profile >/dev/null 2>&1; source ~/.bashrc >/dev/null 2>&1; if [ -s "$NVM_DIR/nvm.sh" ]; then source "$NVM_DIR/nvm.sh"; fi; echo $PATH')"

if [[ -z "${PRESENTER_ANDROID_ADB_BIN:-}" ]]; then
  if command -v adb >/dev/null 2>&1; then
    export PRESENTER_ANDROID_ADB_BIN="$(command -v adb)"
  else
    export PRESENTER_ANDROID_ADB_BIN="true"
  fi
fi

export ADB_KEYS_DIR="${ADB_KEYS_DIR:-${ORIGINAL_HOME}/.config/presenter/adb}"
RUN_AS_ORIGINAL() {
  local cmd=("$@")
  local quoted=$(printf '%q ' "${cmd[@]}")
  sudo -H -u "$ORIGINAL_USER" HOME="$ORIGINAL_HOME" PATH="$ORIGINAL_PATH" NVM_DIR="$NVM_DIR_DEFAULT" PRESENTER_ANDROID_ADB_BIN="$PRESENTER_ANDROID_ADB_BIN" ADB_KEYS_DIR="$ADB_KEYS_DIR" bash -lc "source ~/.profile >/dev/null 2>&1; source ~/.bashrc >/dev/null 2>&1; if [ -s "$NVM_DIR/nvm.sh" ]; then source "$NVM_DIR/nvm.sh"; fi; ${quoted}"
}

if ! docker info >/dev/null 2>&1; then
  echo "[verify] Docker daemon is not reachable even under sudo." >&2
  exit 1
fi

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
CUSTOM_DISPLAY_NAME=""

usage() {
  cat <<USAGE
Usage: $(basename "$0") [options]

Runs cargo tests, Playwright tests, then rebuilds and refreshes the branch demo and gateway.

Options:
  --display-name NAME  Friendly name for the demo card (defaults to current branch name)
  -h, --help           Show this help message
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --display-name)
      CUSTOM_DISPLAY_NAME="$2"; shift 2 ;;
    -h|--help)
      usage; exit 0 ;;
    *)
      echo "Unknown option: $1" >&2
      usage
      exit 1 ;;
  esac
done

DISPLAY_NAME="${CUSTOM_DISPLAY_NAME:-$DISPLAY_NAME_DEFAULT}"

# Ensure no misleading demo remains running during verification. If the process
# fails at any point, or until we explicitly (re)start the demo after all
# checks pass, the demo container for this repo stays stopped. The gateway is
# intentionally left running.
stop_demo_stack() {
  echo "[verify] Stopping demo stack for project '$REPO_SLUG' (if running)"
  # Provide required env vars so compose can parse volumes/ports even on 'down'.
  local STATE_ROOT_DEFAULT="${PRESENTER_STATE_DIR:-${XDG_DATA_HOME:-$ORIGINAL_HOME/.local/share}/presenter-demos}"
  local DATA_ROOT="$STATE_ROOT_DEFAULT/data"
  local DEMO_DATA_DIR="$DATA_ROOT/$REPO_SLUG"
  local REPO_PARENT
  REPO_PARENT="$(cd "$REPO_ROOT/.." && pwd)"
  local IMPORT_ROOT="${PRESENTER_LIBRARY_ROOT:-$REPO_PARENT/presenter-libraries}"
  local HOST_OSC_PORT="39051"
  local HOST_COMPANION_PORT="$((DEMO_PORT + 125))" # unused for down; stable filler

  if DEMO_DATA_DIR="$DEMO_DATA_DIR" IMPORT_ROOT="$IMPORT_ROOT" \
     HOST_HTTP_PORT="$DEMO_PORT" HOST_OSC_PORT="$HOST_OSC_PORT" \
     HOST_COMPANION_PORT="$HOST_COMPANION_PORT" ADB_KEYS_DIR="$ADB_KEYS_DIR" \
     docker compose -f "$REPO_ROOT/docker-compose.demo.yml" -p "$REPO_SLUG" down >/dev/null 2>&1; then
    echo "[verify] Demo stack stopped."
  else
    echo "[verify] No running demo stack to stop."
  fi
}

on_error() {
  echo "[verify] Failure detected; ensuring demo is stopped to avoid confusion."
  stop_demo_stack || true
}
trap on_error ERR

# Always stop the demo first so a failing verify never leaves an older demo up.
stop_demo_stack

echo "[verify] Running cargo test"
echo "[verify] Running quality-check (strict)"
RUN_AS_ORIGINAL ./scripts/dev/quality-check.sh --strict --against origin/main
echo "[verify] Quality-check passed"
RUN_AS_ORIGINAL cargo test

echo "[verify] Running Companion module unit tests"
RUN_AS_ORIGINAL npm run test:companion

# Derive a stable but conflict-aware OSC host port. Prefer 39051; if busy, pick the next free.
OSC_PORT_BASE=39051
HOST_OSC_PORT="$OSC_PORT_BASE"
if ss -lntup 2>/dev/null | grep -q ":${HOST_OSC_PORT}\\b"; then
  for p in $(seq $((OSC_PORT_BASE+1)) $((OSC_PORT_BASE+200))); do
    if ! ss -lntup 2>/dev/null | grep -q ":${p}\\b"; then
      HOST_OSC_PORT="$p"
      break
    fi
  done
fi

RUN_ARGS=("--force" "--name" "$REPO_SLUG" "--display-name" "$DISPLAY_NAME" "--port" "$DEMO_PORT" "--osc-port" "$HOST_OSC_PORT" "--enable-companion")

echo "[verify] Refreshing Docker demo for project '$REPO_SLUG' (pre-tests)"
PRESENTER_ANDROID_ADB_BIN="$PRESENTER_ANDROID_ADB_BIN" ADB_KEYS_DIR="$ADB_KEYS_DIR" "$REPO_ROOT/scripts/docker/run-demo.sh" "${RUN_ARGS[@]}"

# Always rebuild gateway so all dev cards reflect the current branch
echo "[verify] Rebuilding gateway (always)"
PRESENTER_ANDROID_ADB_BIN="$PRESENTER_ANDROID_ADB_BIN" ADB_KEYS_DIR="$ADB_KEYS_DIR" "$REPO_ROOT/scripts/docker/run-gateway.sh" --force

echo "[verify] Running Playwright suite"
echo "[verify] Running Playwright suite"
RUN_AS_ORIGINAL npm run test:playwright

echo "[verify] Refreshing Docker demo for project '$REPO_SLUG' (post-tests)"
PRESENTER_ANDROID_ADB_BIN="$PRESENTER_ANDROID_ADB_BIN" ADB_KEYS_DIR="$ADB_KEYS_DIR" "$REPO_ROOT/scripts/docker/run-demo.sh" "--force" "--name" "$REPO_SLUG" "--display-name" "$DISPLAY_NAME" "--port" "$DEMO_PORT" "--osc-port" "$HOST_OSC_PORT" "--enable-companion"

echo "[verify] Restarting gateway (post-tests)"
PRESENTER_ANDROID_ADB_BIN="$PRESENTER_ANDROID_ADB_BIN" ADB_KEYS_DIR="$ADB_KEYS_DIR" "$REPO_ROOT/scripts/docker/run-gateway.sh" --force

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

echo "[verify] Validating gateway stage links"
HTML="$(curl --fail --silent --show-error "${GATEWAY_URL}")"
if echo "$HTML" | grep -qE ">Stage SNV<|>Stage PP<|>Stage Timer<|>Stage Preach<"; then
  echo "[verify] Gateway validation failed: legacy stage links detected" >&2
  exit 1
fi
if ! echo "$HTML" | grep -q ">Stage<"; then
  echo "[verify] Gateway validation failed: missing Stage link" >&2
  exit 1
fi
echo "[verify] ✔ Completed. Demo card should now reflect: $DISPLAY_NAME"
