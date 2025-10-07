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

echo "[verify] Running cargo test"
RUN_AS_ORIGINAL cargo test

echo "[verify] Running Companion module unit tests"
RUN_AS_ORIGINAL npm run test:companion

RUN_ARGS=("--force" "--name" "$REPO_SLUG" "--display-name" "$DISPLAY_NAME" "--port" "$DEMO_PORT" "--enable-companion")

echo "[verify] Refreshing Docker demo for project '$REPO_SLUG' (pre-tests)"
PRESENTER_ANDROID_ADB_BIN="$PRESENTER_ANDROID_ADB_BIN" ADB_KEYS_DIR="$ADB_KEYS_DIR" "$REPO_ROOT/scripts/docker/run-demo.sh" "${RUN_ARGS[@]}"

NEEDS_GATEWAY_REBUILD=0
if git rev-parse --verify origin/main >/dev/null 2>&1; then
  if ! git diff --quiet origin/main..HEAD -- gateway 2>/dev/null; then
    NEEDS_GATEWAY_REBUILD=1
  fi
else
  # Fallback: if any tracked files exist under gateway/, rebuild
  if git ls-files --quiet gateway | grep -q .; then
    NEEDS_GATEWAY_REBUILD=1
  fi
fi

if [[ "$PRESENTER_FORCE_GATEWAY" == "1" ]]; then
  NEEDS_GATEWAY_REBUILD=1
fi

if [[ "$NEEDS_GATEWAY_REBUILD" -eq 1 ]]; then
  echo "[verify] Rebuilding gateway (changes detected under gateway/)"
  PRESENTER_ANDROID_ADB_BIN="$PRESENTER_ANDROID_ADB_BIN" ADB_KEYS_DIR="$ADB_KEYS_DIR" "$REPO_ROOT/scripts/docker/run-gateway.sh" --force
else
  echo "[verify] Rebuilding gateway (always)"
PRESENTER_ANDROID_ADB_BIN="$PRESENTER_ANDROID_ADB_BIN" ADB_KEYS_DIR="$ADB_KEYS_DIR" "$REPO_ROOT/scripts/docker/run-gateway.sh" --force

echo "[verify] Running Playwright suite"
RUN_AS_ORIGINAL npm run test:playwright

echo "[verify] Refreshing Docker demo for project '$REPO_SLUG' (post-tests)"
PRESENTER_ANDROID_ADB_BIN="$PRESENTER_ANDROID_ADB_BIN" ADB_KEYS_DIR="$ADB_KEYS_DIR" "$REPO_ROOT/scripts/docker/run-demo.sh" "--force" "--name" "$REPO_SLUG" "--display-name" "$DISPLAY_NAME" "--port" "$DEMO_PORT" "--enable-companion"

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
