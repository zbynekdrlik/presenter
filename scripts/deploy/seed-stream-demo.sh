#!/usr/bin/env bash
set -euo pipefail
# Seed the owner's stream-graphics demo scene set (#739) into a Presenter
# instance over the REST API. IDEMPOTENT: a scene that already exists (by name,
# case-insensitive) is skipped, so re-running is safe.
#
# WHY this exists (durable source of truth — the #739 finding):
#   The dev deploy REPLACES the whole dev DB with a fresh PROD snapshot on every
#   deploy (pipeline.yml step "Replace dev database with production snapshot").
#   PROD carries no stream demo scenes, so any scenes created by hand on the dev
#   instance are WIPED on the very next deploy. This script is the durable,
#   dev-only source of truth: the deploy runs it AFTER the DB replace so the
#   owner's demo set is always present on the dev/demo instance without touching
#   PROD. It is deliberately NOT a migration (that would seed the scenes onto
#   PROD too, which still runs Resolume for the live walls).
#
# Usage: seed-stream-demo.sh [BASE_URL]   (default http://127.0.0.1:8080)

BASE_URL="${1:-http://127.0.0.1:8080}"
SLUG="stream"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LOGO="${SCRIPT_DIR}/stream-demo-logo.png"

log() { printf '[seed-stream-demo] %s\n' "$*"; }

api() { curl -sf --max-time 30 "$@"; }

# The def, fetched once per call; used for idempotency checks.
def_json() { api "${BASE_URL}/stream/api/outputs/${SLUG}/def"; }

# Print an existing scene's id by name (case-insensitive), or nothing.
scene_id_by_name() {
  local name="$1"
  def_json | python3 -c "
import json,sys
want=sys.argv[1].strip().lower()
d=json.load(sys.stdin)
for s in d.get('scenes',[]):
    if str(s.get('name','')).strip().lower()==want:
        print(s['id']); break
" "$name"
}

create_scene() { # name kind -> id
  api -X POST "${BASE_URL}/stream/api/outputs/${SLUG}/scenes" \
    -H 'Content-Type: application/json' \
    -d "$(python3 -c 'import json,sys;print(json.dumps({"name":sys.argv[1],"kind":sys.argv[2]}))' "$1" "$2")" \
    | python3 -c "import json,sys;print(json.load(sys.stdin)['id'])"
}

add_element() { # scene_id json_props -> id
  api -X POST "${BASE_URL}/stream/api/scenes/$1/elements" \
    -H 'Content-Type: application/json' -d "$2" \
    | python3 -c "import json,sys;print(json.load(sys.stdin)['id'])"
}

upload_asset() { # -> asset_id (dedup by sha256 server-side)
  api -X POST "${BASE_URL}/stream/assets" \
    -F "file=@${LOGO};type=image/png" \
    | python3 -c "import json,sys;print(json.load(sys.stdin)['id'])"
}

# Create a scene only if absent; on create, run the element-builder callback.
# $1 name, $2 kind, $3 builder-fn (receives the new scene id).
ensure_scene() {
  local name="$1" kind="$2" builder="$3" existing
  existing="$(scene_id_by_name "$name" || true)"
  if [ -n "$existing" ]; then
    log "scene '$name' already exists (id $existing) — skip"
    return 0
  fi
  local id
  id="$(create_scene "$name" "$kind")"
  log "created scene '$name' ($kind) id $id"
  "$builder" "$id"
}

# ── element builders (props per #717 / #739 parity spec) ────────────────────
ASSET_ID=""
CD_SIZE=""
CD_Y=""
CD_H=""

ytfast_elems() {
  local sid="$1"
  add_element "$sid" "$(cat <<JSON
{"kind":"image","asset_id":${ASSET_ID},"fit":"contain","frame":{"xPct":10,"yPct":15,"wPct":45,"hPct":30},"opacity":1.0}
JSON
)" >/dev/null
  add_element "$sid" "$(cat <<JSON
{"kind":"countdown","timer_id":1,"style":{"fontFamily":"Oswald","sizePct":10,"color":"#ffffff","weight":700,"align":"center","lineHeight":1.2,"shadow":{"xPx":2,"yPx":2,"blurPx":4,"color":"#000000"}},"frame":{"xPct":10,"yPct":60,"wPct":80,"hPct":20},"content_transition":{"mode":"cut"}}
JSON
)" >/dev/null
}

countdown_scene_elems() { # uses CD_SIZE / CD_Y / CD_H globals
  local sid="$1"
  add_element "$sid" "$(cat <<JSON
{"kind":"countdown","timer_id":1,"style":{"fontFamily":"Oswald","sizePct":${CD_SIZE},"color":"#ffffff","weight":700,"align":"center","lineHeight":1.2,"shadow":{"xPx":2,"yPx":2,"blurPx":4,"color":"#000000"}},"frame":{"xPct":10,"yPct":${CD_Y},"wPct":80,"hPct":${CD_H}},"content_transition":{"mode":"cut"}}
JSON
)" >/dev/null
}

chvaly_elems() {
  local sid="$1"
  add_element "$sid" "$(cat <<JSON
{"kind":"lyrics","show_main":true,"show_translation":true,"main_style":{"fontFamily":"Inter","sizePct":8,"color":"#ffffff","weight":700,"align":"center","lineHeight":1.2},"translation_style":{"fontFamily":"Inter","sizePct":5,"color":"#cccccc","weight":700,"align":"center","lineHeight":1.2},"frame":{"xPct":5,"yPct":15,"wPct":90,"hPct":70},"content_transition":{"mode":"fade","duration_ms":400}}
JSON
)" >/dev/null
}

logo_overlay_elems() {
  local sid="$1"
  add_element "$sid" "$(cat <<JSON
{"kind":"image","asset_id":${ASSET_ID},"fit":"contain","frame":{"xPct":70,"yPct":5,"wPct":25,"hPct":18},"opacity":1.0}
JSON
)" >/dev/null
}

verse_with_translation_elems() {
  local sid="$1"
  add_element "$sid" "$(cat <<JSON
{"kind":"verse","show_secondary":true,"text_style":{"fontFamily":"Inter","sizePct":6,"color":"#ffffff","weight":700,"align":"center","lineHeight":1.2},"secondary_style":{"fontFamily":"Inter","sizePct":4,"color":"#dddddd","weight":700,"align":"center","lineHeight":1.2},"reference_style":{"fontFamily":"Bebas Neue","sizePct":3,"color":"#aaaaaa","weight":400,"align":"center","lineHeight":1.2},"frame":{"xPct":10,"yPct":62,"wPct":80,"hPct":34},"content_transition":{"mode":"fade","duration_ms":400}}
JSON
)" >/dev/null
}

verse_no_translation_elems() {
  local sid="$1"
  add_element "$sid" "$(cat <<JSON
{"kind":"verse","show_secondary":false,"text_style":{"fontFamily":"Inter","sizePct":6,"color":"#ffffff","weight":700,"align":"center","lineHeight":1.2},"secondary_style":{"fontFamily":"Inter","sizePct":4,"color":"#dddddd","weight":700,"align":"center","lineHeight":1.2},"reference_style":{"fontFamily":"Bebas Neue","sizePct":3,"color":"#aaaaaa","weight":400,"align":"center","lineHeight":1.2},"frame":{"xPct":10,"yPct":62,"wPct":80,"hPct":34},"content_transition":{"mode":"fade","duration_ms":400}}
JSON
)" >/dev/null
}

main() {
  log "seeding demo scenes into ${BASE_URL} (output '${SLUG}')"
  if ! def_json >/dev/null; then
    log "ERROR: stream output '${SLUG}' not reachable at ${BASE_URL} — is the server up?"
    exit 1
  fi
  ASSET_ID="$(upload_asset)"
  log "logo asset id ${ASSET_ID}"

  ensure_scene "ytfast" base ytfast_elems

  CD_SIZE=14; CD_Y=38; CD_H=25
  ensure_scene "5 min" base countdown_scene_elems
  CD_SIZE=18; CD_Y=35; CD_H=30
  ensure_scene "1 min" base countdown_scene_elems

  ensure_scene "chvaly" base chvaly_elems

  ensure_scene "Logo" overlay logo_overlay_elems
  ensure_scene "Verš s prekladom" overlay verse_with_translation_elems
  ensure_scene "Verš bez prekladu" overlay verse_no_translation_elems

  log "done"
}

main "$@"
