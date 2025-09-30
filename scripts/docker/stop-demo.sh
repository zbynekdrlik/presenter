#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/common.sh"

NAME=""
DELETE_DATA=0

usage() {
  cat <<USAGE
Usage: $(basename "$0") [options]
  --name NAME     Demo name/slug (defaults to repo folder)
  --delete-data   Remove the persisted volume directory after stopping
  -h, --help      Show help
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --name)
      NAME="$2"; shift 2 ;;
    --delete-data)
      DELETE_DATA=1; shift ;;
    -h|--help)
      usage; exit 0 ;;
    *)
      echo "Unknown option: $1" >&2
      usage; exit 1 ;;
  esac
done

PROJECT="$(derive_project_name "$NAME")"

printf '[stop-demo] Stopping project %s\n' "$PROJECT"
docker compose -f "$REPO_ROOT/docker-compose.demo.yml" -p "$PROJECT" down

remove_manifest "$PROJECT"
printf '[stop-demo] Removed manifest %s/%s.json\n' "$MANIFEST_DIR" "$PROJECT"

if [[ "$DELETE_DATA" -eq 1 ]]; then
  rm -rf "$DATA_ROOT/$PROJECT"
  printf '[stop-demo] Deleted data dir %s/%s\n' "$DATA_ROOT" "$PROJECT"
fi
