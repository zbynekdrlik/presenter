#!/usr/bin/env bash
set -euo pipefail
# llmrot-provision — idempotent provisioning of the restricted "llmrot"
# subscription channel on a presenter host (#730). Mirror of odoo-erp #4697.
#
# Run as the normal project user (it sudo's as needed), from a CLEAN committed
# tree (deploy-from-clean-tree discipline for prod hosts). Re-running is a
# no-op / in-place update — never duplicates account, keys, or sudoers.
#
# Creates: a nologin `llmrot` system account whose ONLY entry point is a
# ForceCommand authorized_keys running `sudo -n /usr/local/sbin/llmrot-apply`;
# a zero-arg sudoers lock; the llmrot-apply handler; /etc/llmrot.conf; and the
# token-free log dir. Blast radius is limited to the local CLIProxyAPI auth
# pool — the account can do nothing else on the box.
#
# Usage:
#   deploy/llmrot-provision.sh --deploy-dir <dir> --pubkey-file <file> [--probe-model <name>]
#   deploy/llmrot-provision.sh --deploy-dir <dir> --pubkey "<ssh-ed25519 ...>"
#
# Examples (per host):
#   dev2 : --deploy-dir /opt/presenter-dev
#   SNV  : --deploy-dir /opt/presenter
#   PP   : --deploy-dir /opt/presenter

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if [[ ${EUID} -eq 0 ]]; then
  echo "[llmrot] Run as the normal project user (it sudo's as needed), not root." >&2
  exit 1
fi

DEPLOY_DIR=""
PUBKEY=""
PUBKEY_FILE=""
PROBE_MODEL="claude-3-5-haiku-20241022"
PROXY_PORT="18787"

while [[ $# -gt 0 ]]; do
  case "$1" in
  --deploy-dir) DEPLOY_DIR="${2:?}"; shift 2 ;;
  --pubkey) PUBKEY="${2:?}"; shift 2 ;;
  --pubkey-file) PUBKEY_FILE="${2:?}"; shift 2 ;;
  --probe-model) PROBE_MODEL="${2:?}"; shift 2 ;;
  --proxy-port) PROXY_PORT="${2:?}"; shift 2 ;;
  *) echo "[llmrot] unknown arg: $1" >&2; exit 2 ;;
  esac
done

[[ -n "$DEPLOY_DIR" ]] || { echo "[llmrot] --deploy-dir is required" >&2; exit 2; }
[[ -d "$DEPLOY_DIR" ]] || { echo "[llmrot] deploy dir does not exist: $DEPLOY_DIR" >&2; exit 2; }

if [[ -n "$PUBKEY_FILE" ]]; then
  [[ -r "$PUBKEY_FILE" ]] || { echo "[llmrot] pubkey file not readable: $PUBKEY_FILE" >&2; exit 2; }
  PUBKEY="$(cat "$PUBKEY_FILE")"
fi
[[ -n "$PUBKEY" ]] || { echo "[llmrot] --pubkey or --pubkey-file is required" >&2; exit 2; }
# Sanity: a single ssh public key line.
if ! printf '%s' "$PUBKEY" | grep -qE '^(ssh-ed25519|ssh-rsa|ecdsa-sha2-\S+) '; then
  echo "[llmrot] pubkey does not look like an ssh public key" >&2
  exit 2
fi
PUBKEY="$(printf '%s' "$PUBKEY" | head -n1)"

AUTH_DIR="${DEPLOY_DIR%/}/.cli-proxy-api"
# The proxy (a child of presenter-server) runs as, and its auth files are owned
# by, the deploy-dir owner. Derive it so this works on any host.
FILE_OWNER="$(stat -c %U "$DEPLOY_DIR")"
PROXY_URL="http://127.0.0.1:${PROXY_PORT}"
APPLY_SRC="${SCRIPT_DIR}/llmrot-apply.sh"
APPLY_DST="/usr/local/sbin/llmrot-apply"

[[ -r "$APPLY_SRC" ]] || { echo "[llmrot] missing handler source: $APPLY_SRC" >&2; exit 2; }

echo "[llmrot] host=$(hostname) deploy-dir=$DEPLOY_DIR auth-dir=$AUTH_DIR owner=$FILE_OWNER proxy=$PROXY_URL"

# 1. llmrot system account, idempotent. The account has NO password (useradd
#    --system leaves it locked → no password login) and its ONLY key-auth entry
#    point is the ForceCommand below. A REAL shell (/bin/bash) is REQUIRED, not
#    nologin: sshd runs a forced `command="..."` via the user's login shell
#    (`$SHELL -c "<cmd>"`), and /usr/sbin/nologin refuses to exec anything
#    ("This account is currently not available."), which breaks the channel.
#    Security comes from the ForceCommand + restrictions + locked password +
#    zero-arg sudoers, never from the shell being nologin.
LLMROT_SHELL=/bin/bash
if ! id llmrot >/dev/null 2>&1; then
  echo "[llmrot] creating system account 'llmrot'"
  sudo useradd --system --create-home --home-dir /home/llmrot --shell "$LLMROT_SHELL" llmrot
else
  echo "[llmrot] account 'llmrot' already exists"
fi
# Ensure the shell is correct even on a re-run of an account created earlier.
CURRENT_SHELL="$(getent passwd llmrot | cut -d: -f7)"
if [[ "$CURRENT_SHELL" != "$LLMROT_SHELL" ]]; then
  echo "[llmrot] fixing shell: $CURRENT_SHELL -> $LLMROT_SHELL"
  sudo usermod --shell "$LLMROT_SHELL" llmrot
fi
LLMROT_HOME="$(getent passwd llmrot | cut -d: -f6)"
LLMROT_HOME="${LLMROT_HOME:-/home/llmrot}"
sudo mkdir -p "$LLMROT_HOME/.ssh"

# 2. ForceCommand authorized_keys — fully managed (single line).
AUTHK="$LLMROT_HOME/.ssh/authorized_keys"
AK_LINE='command="sudo -n /usr/local/sbin/llmrot-apply",no-pty,no-port-forwarding,no-agent-forwarding,no-X11-forwarding '"$PUBKEY"
printf '%s\n' "$AK_LINE" | sudo tee "$AUTHK" >/dev/null
sudo chown -R llmrot:llmrot "$LLMROT_HOME/.ssh"
sudo chmod 700 "$LLMROT_HOME/.ssh"
sudo chmod 600 "$AUTHK"
echo "[llmrot] authorized_keys installed (ForceCommand)"

# 3. Install the handler (root:root 0755).
sudo install -m 0755 -o root -g root "$APPLY_SRC" "$APPLY_DST"
echo "[llmrot] installed $APPLY_DST"

# 4. /etc/llmrot.conf (root:root 0644).
sudo tee /etc/llmrot.conf >/dev/null <<EOF
# Managed by presenter deploy/llmrot-provision.sh (#730) — do not edit.
AUTH_DIR="$AUTH_DIR"
PROXY_URL="$PROXY_URL"
FILE_OWNER="$FILE_OWNER"
PROBE_MODEL="$PROBE_MODEL"
EOF
sudo chmod 0644 /etc/llmrot.conf
echo "[llmrot] wrote /etc/llmrot.conf"

# 5. sudoers zero-arg lock (validated before install).
SUDO_TMP="$(mktemp)"
cat >"$SUDO_TMP" <<'EOF'
# Managed by presenter deploy/llmrot-provision.sh (#730) — do not edit.
Defaults:llmrot env_keep+=SSH_ORIGINAL_COMMAND
llmrot ALL=(root) NOPASSWD: /usr/local/sbin/llmrot-apply ""
EOF
if sudo visudo -cf "$SUDO_TMP" >/dev/null; then
  sudo install -m 0440 -o root -g root "$SUDO_TMP" /etc/sudoers.d/llmrot
  echo "[llmrot] installed /etc/sudoers.d/llmrot (validated)"
else
  echo "[llmrot] sudoers validation FAILED — not installing" >&2
  rm -f "$SUDO_TMP"
  exit 1
fi
rm -f "$SUDO_TMP"

# 6. Token-free log dir + ensure auth-dir exists (owned by proxy user, 0700 so
#    no other local user can plant a symlink among the credential files).
sudo mkdir -p /var/log/llmrot
sudo chmod 0750 /var/log/llmrot
sudo mkdir -p "$AUTH_DIR"
sudo chown "$FILE_OWNER:$FILE_OWNER" "$AUTH_DIR" 2>/dev/null || true
sudo chmod 0700 "$AUTH_DIR"

# 7. Smoke test: run the handler locally exactly as the sudoers path would
#    (root, zero-arg, SSH_ORIGINAL_COMMAND=list). Proves conf + proxy reachability.
echo "[llmrot] smoke test: list"
if sudo SSH_ORIGINAL_COMMAND=list "$APPLY_DST"; then
  echo "[llmrot] provisioning OK on $(hostname)"
else
  echo "[llmrot] smoke test returned nonzero (check proxy up / conf)" >&2
  exit 1
fi
