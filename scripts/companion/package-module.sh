#!/usr/bin/env bash
set -euo pipefail
command -v jq >/dev/null || { echo "jq is required" >&2; exit 1; }
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
MODULE_DIR="$ROOT_DIR/ops/companion/presenter"
DIST_DIR="$ROOT_DIR/ops/companion/releases"
mkdir -p "$DIST_DIR"
cd "$MODULE_DIR"
rm -f "$DIST_DIR"/presenter-companion-ws-*.tgz "$DIST_DIR"/latest.json
PACK_ORIG=$(npm pack --pack-destination "$DIST_DIR")
PACK_ORIG_PATH="$DIST_DIR/$PACK_ORIG"
TARGET_NAME="presenter-companion-ws-$(jq -r '.version' package.json).tgz"
TARGET_PATH="$DIST_DIR/$TARGET_NAME"
if [[ "$PACK_ORIG" != "$TARGET_NAME" ]]; then
  mv "$PACK_ORIG_PATH" "$TARGET_PATH"
fi
HASH=$(sha256sum "$TARGET_PATH" | cut -d' ' -f1)
cat <<META > "$DIST_DIR/latest.json"
{
  "version": $(jq -r '.version' package.json | jq -R .),
  "archive": "$TARGET_NAME",
  "sha256": "$HASH"
}
META
