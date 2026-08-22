#!/usr/bin/env bash
set -euo pipefail
#
# llmrot-provision — one-time, idempotent provisioning of the llmrot channel on
# a single presenter host (issue #730, simplified design — #730 comment
# 5382188113). Run MANUALLY from a clean committed tree per host, as the deploy
# user `newlevel` (NO sudo needed — everything it touches is newlevel-owned):
#
#   ./deploy/llmrot-provision.sh [<deploy-dir>]
#
# It (idempotently):
#   1. installs `llmrot-apply.sh` (sibling of this script) -> <deploy-dir>/llmrot-apply
#   2. ensures the CLIProxyAPI auth-dir exists
#   3. writes/updates the forced-command line for claudy's PUBLIC key in
#      ~/.ssh/authorized_keys, so that key can run ONLY llmrot-apply.
#
# The per-push deploy workflows ALSO refresh <deploy-dir>/llmrot-apply on every
# deploy; this script owns the ONE-TIME authorized_keys line (kept out of CI to
# avoid an sshd-key edit on every push — see #730 design comment).
#
# claudy's key is PUBLIC (safe to commit). It is pinned here by fingerprint so a
# wrong key can never be authorized.

# --- claudy public key (reuse of ~newlevel/.ssh/llmrot_gk on dev1) ------------
EXPECTED_PUBKEY="ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGZqjw6NbG6FmneYNRPUzPcL+k9Twy+9w5p+zRX78zLm llmrot-claudy@dev1"
EXPECTED_FP="SHA256:HurwrqiI6BI7zY0JgWosplvO0Y1X2UGE3fT86IQ6r9M"

SRC_DIR="$(cd "$(dirname "$(readlink -f "$0")")" && pwd)"
APPLY_SRC="$SRC_DIR/llmrot-apply.sh"

err() { echo "ERROR: $*" >&2; exit 1; }

# --- resolve deploy dir ------------------------------------------------------
DEPLOY_DIR="${1:-}"
if [ -z "$DEPLOY_DIR" ]; then
  if [ -d /opt/presenter-dev ] && [ ! -d /opt/presenter ]; then
    DEPLOY_DIR=/opt/presenter-dev
  elif [ -d /opt/presenter ] && [ ! -d /opt/presenter-dev ]; then
    DEPLOY_DIR=/opt/presenter
  elif [ -d /opt/presenter-dev ] && [ -d /opt/presenter ]; then
    err "both /opt/presenter and /opt/presenter-dev exist — pass the deploy dir explicitly"
  else
    err "no presenter deploy dir found — pass it explicitly (e.g. /opt/presenter)"
  fi
fi
[ -d "$DEPLOY_DIR" ] || err "deploy dir does not exist: $DEPLOY_DIR"
[ -f "$APPLY_SRC" ] || err "cannot find llmrot-apply.sh next to this script ($APPLY_SRC)"

# --- verify the pinned pubkey ------------------------------------------------
FP="$(printf '%s\n' "$EXPECTED_PUBKEY" | ssh-keygen -lf - 2>/dev/null | awk '{print $2}')"
[ "$FP" = "$EXPECTED_FP" ] || err "claudy pubkey fingerprint mismatch (got '$FP', expected '$EXPECTED_FP')"

# --- 1) install llmrot-apply -------------------------------------------------
install -m 0755 "$APPLY_SRC" "$DEPLOY_DIR/llmrot-apply"
echo "installed: $DEPLOY_DIR/llmrot-apply"

# --- 2) ensure auth-dir ------------------------------------------------------
mkdir -p "$DEPLOY_DIR/.cli-proxy-api"
chmod 700 "$DEPLOY_DIR/.cli-proxy-api"   # account file NAMES/mtimes not world-visible (contents are already 0600)
echo "auth-dir ready: $DEPLOY_DIR/.cli-proxy-api"

# --- 3) authorized_keys forced-command line (idempotent) ---------------------
SSH_DIR="$HOME/.ssh"
AK="$SSH_DIR/authorized_keys"
mkdir -p "$SSH_DIR"
chmod 700 "$SSH_DIR"
touch "$AK"
chmod 600 "$AK"

KEYBLOB="$(awk '{print $2}' <<<"$EXPECTED_PUBKEY")"
[ -n "$KEYBLOB" ] || err "could not extract key blob from pinned pubkey"
# `restrict` = all current + future no-* restrictions (covers no-user-rc too);
# the explicit no-* list is kept for readability.
LINE="command=\"$DEPLOY_DIR/llmrot-apply\",restrict,no-pty,no-port-forwarding,no-agent-forwarding,no-X11-forwarding $EXPECTED_PUBKEY"

TMP_AK="$(mktemp "$SSH_DIR/.authorized_keys.llmrot.XXXXXX")"
# drop any prior line carrying this exact key blob (so re-runs UPDATE, never
# duplicate). grep exit 1 = "no lines matched" (benign); exit >=2 = a real
# read/I-O error — that must NOT silently truncate authorized_keys, which would
# drop every OTHER key of the deploy user (incl. CI's deploy key -> lockout).
if [ -s "$AK" ]; then
  set +e
  grep -vF "$KEYBLOB" "$AK" >"$TMP_AK"
  rc=$?
  set -e
  [ "$rc" -le 1 ] || { rm -f "$TMP_AK"; err "grep failed reading $AK (exit $rc) — refusing to rewrite authorized_keys"; }
fi
printf '%s\n' "$LINE" >>"$TMP_AK"
chmod 600 "$TMP_AK"
mv -f "$TMP_AK" "$AK"
echo "authorized_keys: claudy key authorized with forced command -> $DEPLOY_DIR/llmrot-apply"

echo "llmrot channel provisioned on $(hostname) for $DEPLOY_DIR"
