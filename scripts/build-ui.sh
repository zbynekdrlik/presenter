#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
UI_DIR="$PROJECT_ROOT/crates/presenter-ui"

cd "$UI_DIR"

# Step 1: Build WASM targeting Safari 12 (iOS 12) compatibility.
# Uses nightly + -Zbuild-std (via .cargo/config.toml) to recompile std with
# target-cpu=mvp, disabling post-MVP WASM features (bulk-memory, sign-ext,
# nontrapping-fptoint) that Safari 12 cannot execute.
echo "==> Building WASM UI with Trunk (MVP WASM for Safari 12)..."
RUSTUP_TOOLCHAIN=nightly trunk build --release

# Step 2: Transpile JS glue code to ES2017 (Safari 12 compatible syntax).
echo "==> Transpiling JS for Safari 12..."
esbuild dist/presenter-ui-*.js \
  --target=es2017 \
  --outdir=dist/ \
  --allow-overwrite

# Step 3: Patch index.html for Safari 12.
# - Wrap top-level await (ES2022) in async IIFE, keeping static import at top
# - Remove SRI integrity attrs (hash mismatch after esbuild; filenames are hashed)
# - Remove modulepreload/preload hints (unsupported or buggy in Safari 12)
echo "==> Patching index.html for Safari 12..."
sed -i "/^import init/a (async () => {" dist/index.html
sed -i "s|</script>|})();</script>|" dist/index.html
sed -i 's| integrity="sha384-[A-Za-z0-9+/=]*"||g' dist/index.html
sed -i 's| crossorigin="anonymous"||g' dist/index.html
sed -i '/<link rel="modulepreload"/d' dist/index.html
sed -i '/<link rel="preload"/d' dist/index.html

echo "==> UI build complete."
