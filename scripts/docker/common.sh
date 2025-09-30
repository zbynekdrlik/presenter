#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
MANIFEST_DIR="${PRESENTER_MANIFEST_DIR:-$REPO_ROOT/var/docker/demos}"
DATA_ROOT="${PRESENTER_DATA_ROOT:-$REPO_ROOT/var/docker/data}"
DEFAULT_IMPORT_ROOT="$REPO_ROOT/Propresenter library"

mkdir -p "$MANIFEST_DIR" "$DATA_ROOT"

stop_conflicting_demos() {
  local repo="$1"
  local keep_project="$2"

  if [[ ! -d "$MANIFEST_DIR" ]]; then
    return
  fi

  while IFS=$'\t' read -r project repo_path port; do
    [[ -z "$project" ]] && continue
    if [[ "$repo_path" != "$repo" ]]; then
      continue
    fi
    if [[ "$project" == "$keep_project" ]]; then
      continue
    fi

    printf '[run-demo] Stopping existing demo %s for %s\n' "$project" "$repo_path"
    local compose_file="$repo_path/docker-compose.demo.yml"
    local data_dir="$repo_path/var/docker/data/$project"
    local import_root="$repo_path/Propresenter library"
    local env=("DEMO_DATA_DIR=$data_dir" "IMPORT_ROOT=$import_root" "HOST_HTTP_PORT=${port:-8080}" "PROJECT_NAME=$project")
    if [[ -f "$compose_file" ]]; then
      (
        cd "$repo_path"
        "${env[@]}" docker compose -f "$compose_file" -p "$project" down || true
      )
    fi
    remove_manifest "$project"
  done < <(MANIFEST_DIR="$MANIFEST_DIR" REPO="$repo" KEEP="$keep_project" python3 - <<'PY'
import json
import os
import sys

manifest_dir = os.environ["MANIFEST_DIR"]
repo = os.environ["REPO"]
keep = os.environ["KEEP"]

if not os.path.isdir(manifest_dir):
    raise SystemExit

for name in sorted(os.listdir(manifest_dir)):
    if not name.endswith('.json'):
        continue
    path = os.path.join(manifest_dir, name)
    try:
        with open(path, 'r', encoding='utf-8') as fh:
            data = json.load(fh)
    except Exception:
        continue
    project = data.get('project') or name[:-5]
    repo_path = data.get('repoPath')
    port = data.get('port')
    if repo_path == repo and project and project != keep:
        print(f"{project}\t{repo_path}\t{port or ''}")
PY
  )
}

slugify() {
  local raw="$1"
  echo "$raw" | tr '[:upper:]' '[:lower:]' | sed -E 's/[^a-z0-9]+/-/g; s/^-+|-+$//g'
}

current_repo_slug() {
  local base
  base="$(basename "$REPO_ROOT")"
  slugify "$base"
}

derive_project_name() {
  local name="$1"
  if [[ -z "$name" ]]; then
    name="$(current_repo_slug)"
  fi
  slugify "$name"
}

compute_port() {
  local project="$1"
  if [[ -n "${2:-}" ]]; then
    echo "$2"
    return
  fi
  local hash
  hash=$(echo -n "$project" | md5sum | cut -c1-4)
  local base=$((0x$hash))
  local port=$((18000 + (base % 1000)))
  echo "$port"
}

write_manifest() {
  local project="$1"
  local name="$2"
  local port="$3"
  local repo_path="$4"
  local manifest_file="$MANIFEST_DIR/${project}.json"
  MANIFEST_PROJECT="$project" \
  MANIFEST_NAME="$name" \
  MANIFEST_PORT="$port" \
  MANIFEST_REPO="$repo_path" \
  python3 - <<'PY' > "$manifest_file"
import json, os, sys, datetime
port = int(os.environ["MANIFEST_PORT"])
manifest = {
    "project": os.environ["MANIFEST_PROJECT"],
    "displayName": os.environ["MANIFEST_NAME"],
    "port": port,
    "repoPath": os.environ["MANIFEST_REPO"],
    "updatedAt": datetime.datetime.now(datetime.timezone.utc).isoformat()
}
json.dump(manifest, sys.stdout, indent=2)
PY
}

remove_manifest() {
  local project="$1"
  rm -f "$MANIFEST_DIR/${project}.json"
}
