#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/common.sh"

NAME=""
PORT=""
OSC_PORT=""
COMPANION_PORT=""
IMPORT_ROOT="$DEFAULT_IMPORT_ROOT"
DISPLAY_NAME=""
LOG_LEVEL="presenter_server=info"
FORCE=0
COMPANION_ENABLED="${PRESENTER_COMPANION_ENABLED:-0}"
ADB_KEYS_DIR="${ADB_KEYS_DIR:-${XDG_CONFIG_HOME:-${HOME}/.config}/presenter/adb}"
PUBLISH_OSC=0
BIBLE_CACHE_DIR_DEFAULT="${XDG_CACHE_HOME:-${HOME}/.cache}/presenter/bibles"

usage() {
  cat <<USAGE
Usage: $(basename "$0") [options]
  --name NAME          Friendly name / slug for the demo (defaults to repo folder)
  --port PORT          Host port to publish (defaults to derived high port)
  --import-root PATH   Path to ProPresenter library (default: "$DEFAULT_IMPORT_ROOT")
  --display-name TEXT  Display name for landing page (defaults to NAME)
  --osc-port PORT      Host port to publish for OSC listener (default: 39051)
  --publish-osc        Publish OSC UDP/TCP ports (default: off)
  --companion-port PORT  Host port to publish for the Companion websocket (defaults to derived high port)
  --log-level LEVEL   RUST_LOG value for the presenter container (default: presenter_server=info)
  --enable-companion  Expose the /companion/ws socket inside the demo (default: disabled)
  --disable-companion Disable the /companion/ws socket explicitly
  --force              Rebuild the image even if it exists
  -h, --help           Show this help message
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --name)
      NAME="$2"; shift 2 ;;
    --port)
      PORT="$2"; shift 2 ;;
    --import-root)
      IMPORT_ROOT="$2"; shift 2 ;;
    --adb-keys)
      ADB_KEYS_DIR="$2"; shift 2 ;;
    --display-name)
      DISPLAY_NAME="$2"; shift 2 ;;
    --osc-port)
      OSC_PORT="$2"; shift 2 ;;
    --publish-osc)
      PUBLISH_OSC=1; shift ;;
    --companion-port)
      COMPANION_PORT="$2"; shift 2 ;;
    --log-level)
      LOG_LEVEL="$2"; shift 2 ;;
    --enable-companion)
      COMPANION_ENABLED="1"; shift ;;
    --disable-companion)
      COMPANION_ENABLED="0"; shift ;;
    --force)
      FORCE=1; shift ;;
    -h|--help)
      usage; exit 0 ;;
    *)
      echo "Unknown option: $1" >&2
      usage
      exit 1 ;;
  esac
done

PROJECT="$(derive_project_name "$NAME")"
if [[ -z "$DISPLAY_NAME" ]]; then
  DISPLAY_NAME="$PROJECT"
fi
HOST_HTTP_PORT="$(compute_port "$PROJECT" "$PORT")"
# Use static default for OSC host port unless explicitly overridden.
if [[ -n "$OSC_PORT" ]]; then
  HOST_OSC_PORT="$OSC_PORT"
else
  HOST_OSC_PORT="39051"
fi
HOST_COMPANION_PORT="$(compute_port "${PROJECT}-companion" "$COMPANION_PORT")"
DEMO_DATA_DIR="$DATA_ROOT/$PROJECT"

stop_conflicting_demos "$REPO_ROOT" "$PROJECT"

# Always stop the current stack before rebuilding so bind mounts refresh cleanly.
if ! DEMO_DATA_DIR="$DEMO_DATA_DIR" IMPORT_ROOT="$IMPORT_ROOT" "${DOCKER_CMD[@]}" compose -f "$REPO_ROOT/docker-compose.demo.yml" -p "$PROJECT" down >/dev/null 2>&1; then
  echo "[run-demo] (info) no existing stack to stop for $PROJECT"
fi

mkdir -p "$DEMO_DATA_DIR"
chmod 777 "$DEMO_DATA_DIR"

mkdir -p "$ADB_KEYS_DIR"
chmod 700 "$ADB_KEYS_DIR"

# Force regeneration of the SQLite database and imports so new settings/data
# are always reflected in the running demo.
rm -f "$DEMO_DATA_DIR/presenter.db" "$DEMO_DATA_DIR/.presenter_import_complete"

if [[ ! -d "$IMPORT_ROOT" ]]; then
  echo "[run-demo] Import root '$IMPORT_ROOT' not found" >&2
  exit 1
fi

export PROJECT_NAME="$PROJECT"
export HOST_HTTP_PORT
export HOST_OSC_PORT
export HOST_COMPANION_PORT
export DEMO_DATA_DIR
export IMPORT_ROOT
export PRESENTER_FORCE_IMPORT=1
export RUST_LOG="$LOG_LEVEL"
export PRESENTER_COMPANION_ENABLED="$COMPANION_ENABLED"
export PRESENTER_COMPANION_PORT="$HOST_COMPANION_PORT"
export ADB_KEYS_DIR
export BIBLE_CACHE_DIR="$BIBLE_CACHE_DIR_DEFAULT"

COMPOSE_FILES=("-f" "$REPO_ROOT/docker-compose.demo.yml")
if [[ "$PUBLISH_OSC" -eq 1 ]]; then
  COMPOSE_FILES+=("-f" "$REPO_ROOT/docker-compose.osc.yml")
fi
COMPOSE_ARGS=("${DOCKER_CMD[@]}" "compose" "${COMPOSE_FILES[@]}" "-p" "$PROJECT" up -d)
if [[ "$FORCE" -eq 1 ]]; then
  COMPOSE_ARGS+=("--build")
fi

if [[ "$COMPANION_ENABLED" == "1" ]]; then
  companion_status="enabled on $HOST_COMPANION_PORT"
else
  companion_status="disabled (reserved $HOST_COMPANION_PORT)"
fi
if [[ "$PUBLISH_OSC" -eq 1 ]]; then
  osc_status="published on $HOST_OSC_PORT"
else
  osc_status="not published (enable with --publish-osc)"
fi
printf '[run-demo] Launching %s on http://localhost:%s (OSC %s, Companion %s)\n' "$PROJECT" "$HOST_HTTP_PORT" "$osc_status" "$companion_status"
"${COMPOSE_ARGS[@]}"

# Wait for health endpoint
HEALTH_URL="http://127.0.0.1:${HOST_HTTP_PORT}/healthz"
printf '[run-demo] Waiting for %s to report healthy' "$HEALTH_URL"
HEALTH_OK=0
for _ in $(seq 1 180); do
  if curl -fsS --max-time 2 "$HEALTH_URL" >/dev/null 2>&1; then
    HEALTH_OK=1
    break
  fi
  printf '.'
  sleep 1
done
printf '\n'

if [[ "$HEALTH_OK" -eq 0 ]]; then
  echo "[run-demo] WARNING: demo did not become ready within 180s; check container logs"
fi

write_manifest "$PROJECT" "$DISPLAY_NAME" "$HOST_HTTP_PORT" "$REPO_ROOT"
printf '[run-demo] Manifest written to %s/%s.json\n' "$MANIFEST_DIR" "$PROJECT"

# Ensure the gateway is running so the branch demo appears on the landing page.
if ! "$SCRIPT_DIR/run-gateway.sh" >/dev/null 2>&1; then
  echo "[run-demo] Failed to launch gateway; check docker logs" >&2
fi
