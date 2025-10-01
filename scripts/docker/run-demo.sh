#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/common.sh"

NAME=""
PORT=""
IMPORT_ROOT="$DEFAULT_IMPORT_ROOT"
DISPLAY_NAME=""
FORCE=0

usage() {
  cat <<USAGE
Usage: $(basename "$0") [options]
  --name NAME          Friendly name / slug for the demo (defaults to repo folder)
  --port PORT          Host port to publish (defaults to derived high port)
  --import-root PATH   Path to ProPresenter library (default: "$DEFAULT_IMPORT_ROOT")
  --display-name TEXT  Display name for landing page (defaults to NAME)
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
    --display-name)
      DISPLAY_NAME="$2"; shift 2 ;;
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
DEMO_DATA_DIR="$DATA_ROOT/$PROJECT"

stop_conflicting_demos "$REPO_ROOT" "$PROJECT"

# Always stop the current stack before rebuilding so bind mounts refresh cleanly.
if ! DEMO_DATA_DIR="$DEMO_DATA_DIR" IMPORT_ROOT="$IMPORT_ROOT" "${DOCKER_CMD[@]}" compose -f "$REPO_ROOT/docker-compose.demo.yml" -p "$PROJECT" down >/dev/null 2>&1; then
  echo "[run-demo] (info) no existing stack to stop for $PROJECT"
fi

mkdir -p "$DEMO_DATA_DIR"
chmod 777 "$DEMO_DATA_DIR"

# Force regeneration of the SQLite database and imports so new settings/data
# are always reflected in the running demo.
rm -f "$DEMO_DATA_DIR/presenter.db" "$DEMO_DATA_DIR/.presenter_import_complete"

if [[ ! -d "$IMPORT_ROOT" ]]; then
  echo "[run-demo] Import root '$IMPORT_ROOT' not found" >&2
  exit 1
fi

export PROJECT_NAME="$PROJECT"
export HOST_HTTP_PORT
export DEMO_DATA_DIR
export IMPORT_ROOT
export PRESENTER_FORCE_IMPORT=1

COMPOSE_ARGS=("${DOCKER_CMD[@]}" "compose" "-f" "$REPO_ROOT/docker-compose.demo.yml" "-p" "$PROJECT" up -d)
if [[ "$FORCE" -eq 1 ]]; then
  COMPOSE_ARGS+=("--build")
fi

printf '[run-demo] Launching %s on http://localhost:%s\n' "$PROJECT" "$HOST_HTTP_PORT"
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
