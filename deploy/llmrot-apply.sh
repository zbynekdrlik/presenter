#!/usr/bin/env bash
set -euo pipefail
# llmrot-apply — restricted "llmrot" subscription channel root handler (#730).
#
# Installed as /usr/local/sbin/llmrot-apply (root:root 0755). Invoked ONLY
# through the `llmrot` account's ForceCommand authorized_keys, as root via
# `sudo -n /usr/local/sbin/llmrot-apply` with ZERO arguments (sudoers
# zero-arg lock). All input arrives via $SSH_ORIGINAL_COMMAND (the subcommand)
# and STDIN (the auth payload) — NEVER argv.
#
# Mirror of the odoo-erp #4697 gk design, adapted for presenter where
# CLIProxyAPI runs as a child of presenter-server. The reload mechanism is
# CLIProxyAPI's own fsnotify auth-dir watcher (verified live on dev2, binary
# 7.2.130): an atomically-written auth file is hot-reloaded incrementally
# within ~1s with NO process restart — so this script never restarts
# presenter-server or the proxy, and never causes a stage/operator outage.
#
# Subcommands (matched exactly):
#   list                 — llmrot-managed account names + mtime + proxy/completion probe (NO tokens)
#   apply <name> <STDIN> — atomically install llmrot-<name>.json; rollback if the proxy goes down
#   remove <name>        — print the current (rotated) auth JSON to stdout, then delete llmrot-<name>.json
#
# name := [a-z0-9_-]{1,32}
#
# Security: token content is ONLY ever read from STDIN / written to the 0600
# auth file / (for remove) printed to stdout back over the ssh channel — it is
# NEVER written to the log or passed on argv. The log is token-free by
# construction (only status strings, never payloads).

CONF=/etc/llmrot.conf
if [ ! -r "$CONF" ]; then
  echo "llmrot: missing or unreadable $CONF" >&2
  exit 78 # EX_CONFIG
fi
# shellcheck source=/dev/null
. "$CONF"
: "${AUTH_DIR:?llmrot: AUTH_DIR not set in /etc/llmrot.conf}"
: "${PROXY_URL:?llmrot: PROXY_URL not set in /etc/llmrot.conf}"
: "${FILE_OWNER:?llmrot: FILE_OWNER not set in /etc/llmrot.conf}"
PROBE_MODEL="${PROBE_MODEL:-claude-3-5-haiku-20241022}"

# LOG_DIR / LOCK default to the production paths. They are env-overridable ONLY
# for isolated testing — this is safe because the real invocation goes through
# sudo with `env_reset` (default) keeping ONLY SSH_ORIGINAL_COMMAND (per the
# sudoers env_keep), so a caller on the ssh channel cannot inject either.
LOG_DIR="${LLMROT_LOG_DIR:-/var/log/llmrot}"
LOG="$LOG_DIR/apply.log"
# The lock lives INSIDE the root-owned 0750 log dir — NOT the world-writable
# (1777) /var/lock (→ /run/lock), where a local unprivileged user could plant
# a symlink that this script's root `exec 9>"$LOCK"` (O_CREAT|O_TRUNC, follows
# symlinks) would then truncate to zero bytes as root. A root-only parent dir
# closes that vector.
LOCK="${LLMROT_LOCK:-$LOG_DIR/llmrot.lock}"
MAX_STDIN=$((64 * 1024))

# Temp/snapshot paths — script-global so the EXIT trap set in do_apply can clean
# them up on any abort (a `local` would be out of scope by the time the trap runs).
tmp=""
prev=""

log() {
  # Token-free by construction: callers only pass status strings, never payloads.
  printf '%s %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$*" >>"$LOG" 2>/dev/null || true
}

# Set owner on an auth file to the proxy user; try user:user then user (systems
# where the primary group name differs). Returns nonzero if BOTH fail so the
# caller can fail CLOSED rather than publishing a wrong-owner credential file.
chown_file() {
  chown "$FILE_OWNER:$FILE_OWNER" "$1" 2>/dev/null || chown "$FILE_OWNER" "$1" 2>/dev/null
}

# Proxy up == GET /v1/models returns HTTP 200. This is the ONLY rollback
# trigger for apply: a proxy that stopped serving after a write.
proxy_up() {
  local code
  code=$(curl -s -o /dev/null -w '%{http_code}' --max-time 5 "$PROXY_URL/v1/models" 2>/dev/null || echo 000)
  [ "$code" = "200" ]
}

# Diagnostic completion probe. Prints one of: 200 | limited | err:<code> | na
# NEVER logs or prints the response body (which could carry provider text).
# A rate-limited (429 / "limited") result is EXPECTED for a dying account and
# is NOT a failure — the account is loaded, just throttled.
completion_probe() {
  local body code
  body=$(curl -s -w $'\n%{http_code}' --max-time 20 -X POST "$PROXY_URL/v1/chat/completions" \
    -H 'Content-Type: application/json' \
    -d "{\"model\":\"${PROBE_MODEL}\",\"max_tokens\":1,\"messages\":[{\"role\":\"user\",\"content\":\"hi\"}]}" 2>/dev/null) || {
    echo "na"
    return 0
  }
  code=$(printf '%s' "$body" | tail -n1)
  case "$code" in
  200) echo "200" ;;
  429) echo "limited" ;;
  *)
    if printf '%s' "$body" | grep -qiE 'rate.?limit|"limited"|exhausted|overloaded|too many requests'; then
      echo "limited"
    else
      echo "err:${code}"
    fi
    ;;
  esac
}

validate_name() {
  # bash `[[ =~ ]]` anchors ^/$ to the whole STRING (no REG_NEWLINE), so an
  # embedded newline cannot smuggle a second line past this — unlike the
  # line-oriented `grep -qE '^...$'` this replaced, which matched if only the
  # FIRST line was clean (log-injection / fragile-traversal vector).
  [[ "$1" =~ ^[a-z0-9_-]{1,32}$ ]]
}

do_list() {
  local p comp="na" n=0 f base nm mt
  if proxy_up; then p=up; comp=$(completion_probe); else p=down; fi
  shopt -s nullglob
  for f in "$AUTH_DIR"/llmrot-*.json; do
    base=${f##*/}
    nm=${base#llmrot-}
    nm=${nm%.json}
    mt=$(stat -c %Y "$f" 2>/dev/null || echo 0)
    printf '%s\t%s\n' "$nm" "$mt"
    n=$((n + 1))
  done
  printf 'proxy=%s completion=%s accounts=%s\n' "$p" "$comp" "$n"
  log "list proxy=$p completion=$comp accounts=$n"
}

do_apply() {
  local name="$1" payload target comp
  # tmp/prev are script-global (see top) so the EXIT trap can clean them up.
  # Clean up any temp/snapshot on ANY exit (abort mid-write, rollback, success).
  trap 'rm -f "$tmp" "$prev" 2>/dev/null || true' EXIT
  # STDIN, hard-capped at MAX_STDIN bytes (read one extra byte to detect overflow).
  payload=$(head -c "$((MAX_STDIN + 1))")
  if [ "$(printf '%s' "$payload" | wc -c)" -gt "$MAX_STDIN" ]; then
    echo "llmrot: payload too large (> ${MAX_STDIN} bytes)" >&2
    log "apply $name REJECT oversize"
    exit 1
  fi
  # JSON shape validation: object with type/access_token/refresh_token present.
  if ! printf '%s' "$payload" | python3 -c '
import sys, json
d = json.load(sys.stdin)
assert isinstance(d, dict), "not an object"
for k in ("type", "access_token", "refresh_token"):
    assert k in d and isinstance(d[k], str) and d[k], "missing/empty " + k
' 2>/dev/null; then
    echo "llmrot: invalid auth JSON shape (need object with type/access_token/refresh_token)" >&2
    log "apply $name REJECT bad-shape"
    exit 1
  fi

  target="$AUTH_DIR/llmrot-${name}.json"
  # Snapshot the previous file so a re-apply can roll back to it. The snapshot
  # lives in the SAME dir (same filesystem → the rollback mv is an atomic
  # rename, never a cross-fs copy+unlink with a wrong-owner window), dot+random
  # so the *.json watcher glob ignores it, and never in /tmp (no credential
  # leak to a fourth sink; the EXIT trap removes it on any abort).
  if [ -f "$target" ]; then
    prev=$(mktemp "$AUTH_DIR/.llmrot-prev.XXXXXX")
    cp -p "$target" "$prev"
  fi
  # Atomic write: temp in the SAME dir (so mv is a rename on one filesystem),
  # dot+random so the *.json watcher glob ignores it, correct perms/owner set
  # BEFORE the rename so the watcher never sees a half-written or wrong-perms file.
  tmp=$(mktemp "$AUTH_DIR/.llmrot-${name}.XXXXXX")
  printf '%s' "$payload" >"$tmp"
  chmod 600 "$tmp"
  # Fail CLOSED: if the owner cannot be set, the proxy (running as FILE_OWNER)
  # could not read the file — never rename a wrong-owner credential into place.
  if ! chown_file "$tmp"; then
    rm -f "$tmp"
    tmp=""
    log "apply $name ABORT chown-failed"
    echo "llmrot: could not set owner ($FILE_OWNER) on auth file — aborted" >&2
    exit 1
  fi
  mv -f "$tmp" "$target"
  tmp="" # renamed away; nothing left for the trap to clean

  sleep 2 # let the fsnotify watcher pick up the change

  if ! proxy_up; then
    # Rollback: restore previous content (atomic same-fs rename), or remove.
    if [ -n "$prev" ]; then
      mv -f "$prev" "$target"
      prev=""
      chmod 600 "$target"
      chown_file "$target" || true
    else
      rm -f "$target"
    fi
    sleep 2
    log "apply $name ROLLBACK proxy-down"
    echo "llmrot: proxy down after apply — rolled back" >&2
    exit 1
  fi

  comp=$(completion_probe)
  [ -n "$prev" ] && { rm -f "$prev"; prev=""; }
  log "apply $name OK proxy=up completion=$comp"
  echo "applied llmrot-${name} proxy=up completion=${comp}"
}

do_remove() {
  local name="$1"
  local target="$AUTH_DIR/llmrot-${name}.json"
  # Regular file AND not a symlink (never `cat` a symlink to an arbitrary file
  # out over the ssh channel as root — defense in depth on top of the 0700
  # proxy-owned auth dir).
  if [ -f "$target" ] && [ ! -L "$target" ]; then
    # Print the CURRENT (CLIProxyAPI may have rotated the refresh token) auth
    # JSON to stdout so claudy can reclaim the live token, THEN delete it.
    cat "$target"
    rm -f "$target"
    sleep 2
    log "remove $name OK (current auth returned on stdout)"
  else
    # Idempotent: already absent — empty stdout, success.
    log "remove $name NOOP (absent)"
  fi
}

main() {
  mkdir -p "$LOG_DIR" 2>/dev/null || true
  local cmd="${SSH_ORIGINAL_COMMAND:-}" action name=""
  case "$cmd" in
  list) action=list ;;
  "apply "*)
    action=apply
    name="${cmd#apply }"
    ;;
  "remove "*)
    action=remove
    name="${cmd#remove }"
    ;;
  *)
    echo "llmrot: usage: list | apply <name> | remove <name>" >&2
    log "REJECT bad-cmd"
    exit 2
    ;;
  esac
  if [ "$action" != list ]; then
    if ! validate_name "$name"; then
      echo "llmrot: invalid name (must match [a-z0-9_-]{1,32})" >&2
      log "$action REJECT bad-name"
      exit 2
    fi
  fi
  case "$action" in
  list) do_list ;;
  apply) do_apply "$name" ;;
  remove) do_remove "$name" ;;
  esac
}

# Serialize concurrent invocations. The lock dir (LOG_DIR) is root-owned 0750
# (created by provisioning); ensure it exists before opening the lock so the
# open never has to create a path a non-root user could have pre-planted.
mkdir -p "$LOG_DIR" 2>/dev/null || true
exec 9>"$LOCK"
if ! flock -w 30 9; then
  echo "llmrot: busy (could not acquire lock)" >&2
  exit 75 # EX_TEMPFAIL
fi
main
