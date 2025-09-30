#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/common.sh"

FORCE=0

usage() {
  cat <<USAGE
Usage: $(basename "$0") [options]
  --force      Rebuild the gateway image
  -h, --help   Show this help
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
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

export PRESENTER_MANIFEST_DIR="$MANIFEST_DIR"

CMD=("${DOCKER_CMD[@]}" compose -f "$REPO_ROOT/docker-compose.gateway.yml" up -d)
if [[ "$FORCE" -eq 1 ]]; then
  CMD+=("--build")
fi

printf '[run-gateway] Launching gateway with manifests in %s\n' "$PRESENTER_MANIFEST_DIR"
"${CMD[@]}"
printf '[run-gateway] Gateway available at http://localhost:80/\n'
