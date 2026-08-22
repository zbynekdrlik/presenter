#!/usr/bin/env bash
set -euo pipefail
#
# llmrot-apply — restricted forced-command channel for the presenter prods.
#
# claudy (dev1) manages a fleet of Claude subscriptions and feeds dying-but-live
# accounts into the on-device CLIProxyAPI so the AI helper (verse parsing) never
# stalls on a hit rate limit. This script is the PRESENTER SIDE of that channel
# (issue #730, mirror of odoo-erp #4697), SIMPLIFIED per owner ruling
# (#730 comment 5382188113): NO dedicated user, NO sudoers, NO root script.
#
# It is installed into the presenter DEPLOY DIR (e.g. /opt/presenter[-dev]) and
# runs as the deploy user `newlevel` (which owns the auth-dir) WITHOUT sudo. The
# ONLY way in is the sshd forced command bound to claudy's key in newlevel's
# authorized_keys:
#
#   command="<deploy-dir>/llmrot-apply",no-pty,no-port-forwarding,\
#   no-agent-forwarding,no-X11-forwarding ssh-ed25519 AAAA... llmrot-claudy@dev1
#
# so the key can run ONLY this script — nothing else on the box.
#
# Wire contract (identical to gk / odoo-erp #4697, consumed by claudy #153),
# carried entirely via $SSH_ORIGINAL_COMMAND + STDIN/STDOUT — never argv, never
# logged, never in the repo:
#
#   list                 -> "<name>\t<mtime_epoch>" per llmrot account (no tokens)
#                           + summary "proxy=up|down completion=<200|limited|err|na> accounts=N"
#   apply <name> <STDIN>  -> validate + atomically install llmrot-<name>.json (0600),
#                           hot-reloaded by CLIProxyAPI's fsnotify watcher (no restart);
#                           rollback ONLY if the proxy goes DOWN.
#   remove <name>         -> print the CURRENT (rotated) auth JSON to STDOUT, THEN delete
#                           (claudy reclaims the rotated refresh token); missing = no-op.
#
# name = [a-z0-9_-]{1,32}. STDIN payload capped at 64 KiB. Reload is the
# CLIProxyAPI auth-dir fsnotify watcher — no service restart, zero stage/operator
# downtime (proven live on dev2, binary 7.2.130; see #730 design comment).

# --- locate our own deploy dir (works under the sshd forced command, where
#     $0 is the installed absolute path) --------------------------------------
SELF="$(readlink -f "$0")"
DEPLOY_DIR="$(cd "$(dirname "$SELF")" && pwd)"
AUTH_DIR="$DEPLOY_DIR/.cli-proxy-api"
CONFIG_YAML="$DEPLOY_DIR/cli-proxy-api-config.yaml"
LOG_FILE="$DEPLOY_DIR/llmrot.log"
LOCK_FILE="$DEPLOY_DIR/.llmrot.lock"
MAX_PAYLOAD=65536

# proxy port from the presenter-generated config; fall back to the default.
PORT="$(awk -F': *' '/^port:/ {gsub(/[^0-9]/,"",$2); print $2; exit}' "$CONFIG_YAML" 2>/dev/null || true)"
PORT="${PORT:-18787}"
BASE="http://127.0.0.1:${PORT}"

# token-free audit log (never logs STDIN / auth content).
log() { printf '%s [%s] %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "${MODE:-init}" "$*" >>"$LOG_FILE" 2>/dev/null || true; }
die() { echo "ERR: $*" >&2; log "ERROR: $*"; exit 1; }

# --- probes ------------------------------------------------------------------
# proxy liveness -> "up" | "down"
probe_proxy() {
  local code
  code="$(curl -s -o /dev/null -w '%{http_code}' --max-time 8 "$BASE/v1/models" 2>/dev/null || echo 000)"
  case "$code" in
    2*) echo up ;;
    *)  echo down ;;
  esac
}

# real completion -> "200" | "limited" | "err" | "na"
# Uses the cheapest available model (prefers a haiku), max_tokens:1.
probe_completion() {
  local models model resp code body
  models="$(curl -s --max-time 8 "$BASE/v1/models" 2>/dev/null || echo '')"
  [ -n "$models" ] || { echo na; return; }
  model="$(printf '%s' "$models" | python3 -c '
import sys, json
try:
    data = json.load(sys.stdin).get("data", [])
except Exception:
    sys.exit(0)
ids = [m.get("id") for m in data if m.get("id")]
if not ids:
    sys.exit(0)
haiku = [i for i in ids if "haiku" in i.lower()]
print((haiku or ids)[0])
' 2>/dev/null || true)"
  [ -n "$model" ] || { echo na; return; }
  resp="$(curl -s -w '\n%{http_code}' --max-time 30 -X POST "$BASE/v1/chat/completions" \
    -H 'Content-Type: application/json' \
    -d "{\"model\":\"$model\",\"messages\":[{\"role\":\"user\",\"content\":\"1\"}],\"max_tokens\":1}" \
    2>/dev/null || echo $'\n000')"
  code="$(printf '%s' "$resp" | tail -n1)"
  body="$(printf '%s' "$resp" | sed '$d')"
  case "$code" in
    200) echo 200 ;;
    429) echo limited ;;
    *)
      if printf '%s' "$body" | grep -qiE 'rate.?limit|limited|exhaust|quota|resource_exhausted'; then
        echo limited
      else
        echo err
      fi
      ;;
  esac
}

count_accounts() {
  local n=0 f
  shopt -s nullglob
  for f in "$AUTH_DIR"/llmrot-*.json; do n=$((n + 1)); done
  shopt -u nullglob
  echo "$n"
}

# --- modes -------------------------------------------------------------------
do_list() {
  local f base name mtime
  shopt -s nullglob
  for f in "$AUTH_DIR"/llmrot-*.json; do
    base="$(basename "$f" .json)"   # llmrot-<name>
    name="${base#llmrot-}"
    mtime="$(stat -c %Y "$f" 2>/dev/null || echo 0)"
    printf '%s\t%s\n' "$name" "$mtime"
  done
  shopt -u nullglob
  local proxy comp accts
  proxy="$(probe_proxy)"
  accts="$(count_accounts)"
  if [ "$proxy" = up ]; then comp="$(probe_completion)"; else comp=na; fi
  echo "proxy=$proxy completion=$comp accounts=$accts"
  log "list: proxy=$proxy completion=$comp accounts=$accts"
}

do_apply() {
  local name="$1"
  local target="$AUTH_DIR/llmrot-${name}.json"
  mkdir -p "$AUTH_DIR"

  # read STDIN into an in-dir temp (same filesystem -> atomic rename later).
  # temp name does NOT end in .json so the fsnotify watcher ignores it.
  local tmp
  umask 077
  tmp="$(mktemp "$AUTH_DIR/.llmrot-${name}.tmp.XXXXXX")"
  # trap cleans up the temp on any early exit; the backup is cleaned inline.
  trap 'rm -f "$tmp"' EXIT
  head -c $((MAX_PAYLOAD + 1)) >"$tmp"

  local size
  size="$(stat -c %s "$tmp" 2>/dev/null || echo 0)"
  [ "$size" -le "$MAX_PAYLOAD" ] || die "apply $name: payload > ${MAX_PAYLOAD} bytes"
  [ "$size" -gt 0 ] || die "apply $name: empty payload"

  # validate JSON shape: object with string type + non-empty access_token + refresh_token.
  python3 - "$tmp" <<'PY' || die "apply $name: invalid auth JSON shape (need type + access_token + refresh_token)"
import sys, json
try:
    with open(sys.argv[1], "rb") as fh:
        obj = json.load(fh)
except Exception as e:
    sys.stderr.write("json parse: %s\n" % e); sys.exit(1)
if not isinstance(obj, dict):
    sys.stderr.write("not a json object\n"); sys.exit(1)
for k in ("type", "access_token", "refresh_token"):
    v = obj.get(k)
    if not isinstance(v, str) or not v.strip():
        sys.stderr.write("missing/empty field: %s\n" % k); sys.exit(1)
sys.exit(0)
PY

  # snapshot the current file for rollback (bak name never ends in .json).
  local bak="" had_old=0
  if [ -f "$target" ]; then
    had_old=1
    bak="$(mktemp "$AUTH_DIR/.llmrot-${name}.bak.XXXXXX")"
    cp -p "$target" "$bak"
  fi

  chmod 600 "$tmp"
  mv -f "$tmp" "$target"          # atomic install -> watcher hot-reloads
  trap - EXIT                     # temp is now the live file; nothing to clean

  sleep 1.5                       # give the fsnotify watcher time to reload

  local proxy
  proxy="$(probe_proxy)"
  if [ "$proxy" != up ]; then
    # proxy went DOWN after apply -> roll back and fail (per #4697 contract).
    if [ "$had_old" -eq 1 ]; then
      mv -f "$bak" "$target"
      log "apply $name: proxy DOWN after apply -> rolled back to previous file"
    else
      rm -f "$target"
      log "apply $name: proxy DOWN after apply -> removed new file (no previous)"
    fi
    sleep 1.0
    die "apply $name: proxy is DOWN after applying the account (rolled back)"
  fi
  [ "$had_old" -eq 1 ] && rm -f "$bak"

  # proxy up. Completion is DIAGNOSTIC only: a dying account is legitimately
  # rate-limited (429/limited) -> log + exit 0, never roll back.
  local comp accts
  comp="$(probe_completion)"
  accts="$(count_accounts)"
  log "apply $name: installed (${size}B) proxy=up completion=$comp accounts=$accts"
  echo "OK apply $name: proxy=up completion=$comp accounts=$accts"
}

do_remove() {
  local name="$1"
  local target="$AUTH_DIR/llmrot-${name}.json"
  if [ -f "$target" ]; then
    cat "$target"                 # STDOUT = current (rotated) auth JSON -> claudy reclaim
    rm -f "$target"               # watcher unregisters the account
    log "remove $name: printed current auth JSON to stdout, then deleted"
  else
    log "remove $name: no such account (idempotent no-op)"
  fi
}

# --- dispatch (all input via SSH_ORIGINAL_COMMAND; STDIN only for apply) ------
CMD="${SSH_ORIGINAL_COMMAND:-}"
# collapse any accidental surrounding whitespace but keep it a single line.
CMD="$(printf '%s' "$CMD" | tr -d '\r' | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//')"

MODE="reject"
exec 9>"$LOCK_FILE"
if ! flock -w 30 9; then
  die "could not acquire lock within 30s"
fi

if [ "$CMD" = "list" ]; then
  MODE=list
  do_list
elif [[ "$CMD" =~ ^apply\ ([a-z0-9_-]{1,32})$ ]]; then
  MODE=apply
  do_apply "${BASH_REMATCH[1]}"
elif [[ "$CMD" =~ ^remove\ ([a-z0-9_-]{1,32})$ ]]; then
  MODE=remove
  do_remove "${BASH_REMATCH[1]}"
else
  log "rejected command: $(printf '%s' "$CMD" | cut -c1-40)"
  die "unrecognized command; allowed: 'list' | 'apply <name>' | 'remove <name>' (name=[a-z0-9_-]{1,32})"
fi
