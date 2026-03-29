#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
UI_DIR="$PROJECT_ROOT/crates/presenter-ui"

cd "$UI_DIR"

# Step 1: Normal Trunk build
echo "==> Building WASM UI with Trunk..."
trunk build --release

# Step 2: Transpile JS glue to Safari 12 compatible syntax
# The WASM binary is untouched — only the JS bootstrap is rewritten.
echo "==> Transpiling JS for Safari 12 compatibility..."
esbuild dist/presenter-ui-*.js \
  --target=safari12 \
  --outdir=dist/ \
  --allow-overwrite

# Step 3: Fix index.html for Safari 12 compatibility
# Trunk generates an inline <script type="module"> with:
#   - Static import: import init, * as bindings from '...js';
#   - Top-level await: const wasm = await init({...});
#
# Safari 12 doesn't support top-level await (ES2022).
# Static imports can't be inside functions, so we convert to dynamic import().
# We also need to update SRI integrity hashes since esbuild changed the JS.
echo "==> Patching index.html for Safari 12 (top-level await + SRI hashes)..."

# Get the JS filename (without path prefix)
JS_FILE=$(ls dist/presenter-ui-*.js | grep -v '_bg' | head -1)
JS_BASENAME=$(basename "$JS_FILE")

# Compute new SRI hash for the transpiled JS file
NEW_JS_HASH=$(openssl dgst -sha384 -binary "$JS_FILE" | openssl base64 -A)
NEW_JS_INTEGRITY="sha384-$NEW_JS_HASH"

# Rewrite the inline script: static import → dynamic import(), wrap in async IIFE
# Before: import init, * as bindings from '/ui-pkg/xxx.js';
#          const wasm = await init({...});
# After:  (async () => {
#            const { default: init, ...bindings } = await import('/ui-pkg/xxx.js');
#            const wasm = await init({...});
#          })();
sed -i "s|<script type=\"module\">|<script type=\"module\">(async () => {|" dist/index.html
sed -i "s|import init, \* as bindings from '\([^']*\)';|const { default: init, ...bindings } = await import('\1');|" dist/index.html
sed -i "s|</script>|})();</script>|" dist/index.html

# Update the SRI integrity hash for the modulepreload link (JS file changed by esbuild)
sed -i "s|integrity=\"sha384-[A-Za-z0-9+/=]*\"\(>.*${JS_BASENAME}\)|integrity=\"${NEW_JS_INTEGRITY}\"\1|" dist/index.html
# Also update the modulepreload link's integrity attribute
sed -i "s|\(href=\"/ui-pkg/${JS_BASENAME}\"[^>]*\)integrity=\"sha384-[A-Za-z0-9+/=]*\"|\1integrity=\"${NEW_JS_INTEGRITY}\"|" dist/index.html

echo "==> UI build complete."
