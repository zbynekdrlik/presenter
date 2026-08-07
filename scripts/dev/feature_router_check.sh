#!/usr/bin/env bash
set -euo pipefail

# ============================================================================
# Feature-router presence gate (Issue #605, extracted #625).
#
# quality-check.sh section 1 checks that the four feature routers (bible,
# libraries, playlists, presentations) exist. A feature module may be a FLAT
# FILE (`router/<name>.rs`) OR a DIRECTORY of submodules (`router/<name>/mod.rs`
# — e.g. `bible/`, split in #590 the same way `router/integrations/` already
# was) — either form satisfies the check (fixed in PR #604, commit 9521cf94;
# pinned by tests/ci/feature-router-gate.test.sh).
#
# Extracted into its own script (mirroring scripts/dev/placeholder_check.sh /
# fn_length_check.py) so the CI gate and the self-test run the EXACT SAME
# logic instead of two copies that can drift apart. #625: the self-test used
# to hand-copy this loop as its own bash function, so reverting the REAL gate
# here to the old flat-file-only assumption left the self-test green — the
# self-test was proving nothing about the production check it claimed to pin.
#
# Usage:   feature_router_check.sh [search_root]
#   search_root  Directory whose crates/presenter-server/src/router/ subtree
#                is checked. Defaults to the repository root (two levels up
#                from this script's own directory).
#
# Exit codes:
#   0  all four routers present (prints "OK")
#   1  a router is missing (prints "MISSING:<name>", the first one found)
# ============================================================================

SEARCH_ROOT="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"

for name in bible libraries playlists presentations; do
  flat="$SEARCH_ROOT/crates/presenter-server/src/router/${name}.rs"
  dir_mod="$SEARCH_ROOT/crates/presenter-server/src/router/${name}/mod.rs"
  if [[ ! -f "$flat" && ! -f "$dir_mod" ]]; then
    echo "MISSING:${name}"
    exit 1
  fi
done

echo "OK"
