#!/usr/bin/env bash
# Install the NDI runtime library on a remote presenter host.
#
# Background: presenter-ndi loads libndi.so.6 at startup via libloading.
# Without it the binary still runs, but `/ndi/status` reports
# `{"available":false}` and source discovery returns []. The NDI SDK is
# proprietary and must be obtained from NewTek; we do not redistribute it.
#
# This script copies an existing NDI runtime (typically /usr/lib/ndi) from
# the local machine to the remote host. Idempotent: skips the copy if the
# library is already present, and never restarts a service unless asked.
#
# Usage:
#   scripts/ops/install-ndi-runtime.sh <ssh-target> [--service NAME] [--source DIR]
#
# Examples:
#   scripts/ops/install-ndi-runtime.sh newlevel@companion-pp.lan --service presenter
#   scripts/ops/install-ndi-runtime.sh deploy-target --source /usr/lib/ndi

set -euo pipefail

SOURCE_DIR="/usr/lib/ndi"
TARGET=""
SERVICE=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --source)
      SOURCE_DIR="$2"; shift 2 ;;
    --service)
      SERVICE="$2"; shift 2 ;;
    -h|--help)
      grep -E '^# ' "$0" | sed 's/^# //'; exit 0 ;;
    *)
      if [[ -z "$TARGET" ]]; then
        TARGET="$1"; shift
      else
        echo "[install-ndi] unexpected argument: $1" >&2; exit 2
      fi
      ;;
  esac
done

if [[ -z "$TARGET" ]]; then
  echo "[install-ndi] missing <ssh-target>; run with --help for usage." >&2
  exit 2
fi

if [[ ! -f "$SOURCE_DIR/libndi.so.6" ]]; then
  echo "[install-ndi] $SOURCE_DIR/libndi.so.6 not found on local machine." >&2
  echo "[install-ndi] Install the NDI Advanced SDK v6 for Linux first (https://ndi.video/sdk/)." >&2
  exit 1
fi

if ssh "$TARGET" 'test -f /usr/lib/ndi/libndi.so.6' 2>/dev/null; then
  echo "[install-ndi] /usr/lib/ndi/libndi.so.6 already present on $TARGET; skipping copy."
else
  echo "[install-ndi] Copying $SOURCE_DIR -> $TARGET:/usr/lib/ndi"
  STAGE="$(mktemp -d)"
  trap 'rm -rf "$STAGE"' EXIT
  tar -C "$(dirname "$SOURCE_DIR")" -czf "$STAGE/ndi-runtime.tar.gz" "$(basename "$SOURCE_DIR")"
  scp -q "$STAGE/ndi-runtime.tar.gz" "$TARGET:/tmp/ndi-runtime.tar.gz"
  ssh "$TARGET" 'sudo tar xzf /tmp/ndi-runtime.tar.gz -C /usr/lib/ && sudo ldconfig && rm /tmp/ndi-runtime.tar.gz'
  echo "[install-ndi] Library installed."
fi

if [[ -n "$SERVICE" ]]; then
  echo "[install-ndi] Restarting $SERVICE on $TARGET to pick up the library."
  ssh "$TARGET" "sudo systemctl restart $SERVICE"
fi

echo "[install-ndi] Done. Verify with: curl http://<host>/ndi/status"
