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
# `--supported:import-meta=true` overrides the es2017 default (#533): the
# trunk-generated wasm loader resolves the .wasm URL via
# `new URL('...wasm', import.meta.url)`, and the module IS loaded as an ES
# module at runtime (Safari 12.1+/iOS 12.2+, our actual target, supports
# import.meta), so esbuild must NOT down-level import.meta to `{}` — doing so
# empties the URL base and can break wasm loading. We keep every other es2017
# syntax down-level; only import.meta is declared runtime-supported.
echo "==> Transpiling JS for Safari 12..."
esbuild_out=$(esbuild dist/presenter-ui-*.js \
  --target=es2017 \
  --supported:import-meta=true \
  --outdir=dist/ \
  --allow-overwrite 2>&1)
echo "$esbuild_out"
# Regression guard (#533): the empty-import-meta warning must not reappear — it
# means the wasm URL base was silently emptied. Fail the build if it does.
if echo "$esbuild_out" | grep -q 'empty-import-meta'; then
  echo "ERROR: esbuild emptied import.meta (#533 regression) — wasm URL base broken." >&2
  exit 1
fi

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

# Regression guard (#533), positive form: the wasm loader MUST still resolve
# its URL via import.meta.url in the shipped dist. Catches a future esbuild/
# trunk change that empties it even without printing the warning above.
if ! grep -q 'import\.meta\.url' dist/presenter-ui-*.js; then
  echo "ERROR: dist wasm loader lost import.meta.url (#533 regression) — wasm URL base broken." >&2
  exit 1
fi

echo "==> UI build complete."
