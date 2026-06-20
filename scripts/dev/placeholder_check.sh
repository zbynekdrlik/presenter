#!/usr/bin/env bash
set -euo pipefail

# ============================================================================
# Placeholder / unfinished-work detection (Issue #200; robustness fix #364).
#
# Scans PRODUCTION Rust/TypeScript source for placeholder / unfinished-work
# markers and FAILS (exit 1) if any are found in a non-comment context.
#
# This logic was previously inlined in quality-check.sh section 16 and was a
# SILENT NO-OP (Issue #364):
#   1. The include glob `-g 'crates/**/*.rs'` is ripgrep-version-fragile — an
#      older apt ripgrep anchors `**` differently and silently misses nested
#      files, so deeply-nested markers were never scanned.
#   2. Both rg invocations ended in `2>/dev/null || true`, which swallowed ANY
#      glob/rg error and let the gate pass — a true no-op.
#
# Fixes (all in this script):
#   a. Scan an EXPLICIT search path (crates/, passed as $1) and select files by
#      content type via `--type rust` / `--type ts` instead of a `**`-anchored
#      include glob. This is robust across ripgrep versions.
#   b. Branch explicitly on ripgrep's exit code so a real rg/glob error FAILS
#      LOUDLY (exit 2) while an honest zero-match is treated as clean:
#        rg exit 0 -> matches found
#        rg exit 1 -> no matches (the legitimate clean case)
#        rg exit 2 -> a real error (bad glob, unreadable path, ...) -> FAIL
#
# Extracted into its own script (mirroring scripts/dev/fn_length_check.py for
# the #374 fn-length gate) so the gate can be exercised directly by the
# self-test tests/ci/placeholder-gate.test.sh.
#
# Usage:   placeholder_check.sh [search_root]
#   search_root  Directory whose `crates/` subtree is scanned. Defaults to the
#                repository root (two levels up from this script).
#
# Exit codes:
#   0  no placeholder markers found (clean)
#   1  placeholder markers found (prints "file:line:text" hits to stdout)
#   2  a real rg/glob/scan error occurred (fails loudly — never a silent no-op)
# ============================================================================

if ! command -v rg >/dev/null 2>&1; then
  echo "::error::placeholder gate: ripgrep (rg) not found — cannot scan" >&2
  exit 2
fi

ROOT_DIR="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
# Scan the `crates` subtree by its RELATIVE name (rg is run from ROOT_DIR), so
# match output paths are `crates/...` regardless of where the repo is checked
# out. SCAN_DIR is the absolute form, used only for existence checks/messages.
SCAN_REL="crates"
SCAN_DIR="$ROOT_DIR/$SCAN_REL"

if [[ ! -d "$SCAN_DIR" ]]; then
  echo "::error::placeholder gate: scan dir not found: $SCAN_DIR" >&2
  exit 2
fi

# Shared exclusions (test/vendor/migration/css are not production code here).
EXCLUDES=(
  -g '!**/tests.rs'
  -g '!**/tests/*.rs'
  -g '!**/test_*.rs'
  -g '!**/vendor/**'
  -g '!**/presenter-migration/**'
  -g '!**/*.css'
)

# Run ripgrep with explicit exit-code handling. Echoes matches to stdout and
# RETURNS a status the caller checks (run in a command substitution, so a global
# flag would be lost in the subshell — the status is the only reliable signal):
#   0  matches found (printed to stdout)
#   0  no matches (rg exit 1 — the legitimate clean case)
#   2  a real rg/glob/scan error (printed to stderr) — NEVER swallowed
run_rg() {
  local out rc
  # Disable errexit around rg so we can branch on its exit code (rg exits 1 on
  # no-match, which is the normal clean case and must NOT abort the script).
  # Run from ROOT_DIR and scan the RELATIVE `crates` path so match output is
  # `crates/...:line:text` (never the absolute checkout prefix). A checkout path
  # containing a space would otherwise inject a space into the filename field
  # that the comment filter's `\S+` cannot span — a false positive. The
  # subshell `cd` keeps the caller's cwd untouched.
  set +e
  out=$(cd "$ROOT_DIR" && rg "$@" --type rust --type ts "$SCAN_REL" "${EXCLUDES[@]}")
  rc=$?
  set -e
  case "$rc" in
    0) printf '%s\n' "$out"; return 0 ;;   # matches found
    1) return 0 ;;                          # no matches — legitimate clean case
    *) echo "::error::placeholder gate: ripgrep failed (exit $rc) scanning $SCAN_DIR" >&2
       return 2 ;;
  esac
}

placeholder_hits=""

# Pattern group 1: multi-word phrases (always placeholder, even in comments).
set +e
phrase_hits=$(run_rg -in '(coming soon|not implemented)')
phrase_rc=$?
set -e
if (( phrase_rc != 0 )); then
  exit 2  # real scan error — fail loudly, never silent
fi
if [[ -n "$phrase_hits" ]]; then
  placeholder_hits+="$phrase_hits"$'\n'
fi

# Pattern group 2: marker words — exclude comment lines (// or * or #).
set +e
marker_hits=$(run_rg -n '\b(TODO|FIXME|HACK)\b')
marker_rc=$?
set -e
if (( marker_rc != 0 )); then
  exit 2  # real scan error — fail loudly, never silent
fi
if [[ -n "$marker_hits" ]]; then
  filtered_markers=$(printf '%s\n' "$marker_hits" | grep -vP '^\S+:\d+:\s*(//|/?\*|#)' || true)
  if [[ -n "$filtered_markers" ]]; then
    placeholder_hits+="$filtered_markers"$'\n'
  fi
fi

placeholder_hits=$(printf '%s\n' "$placeholder_hits" | sed '/^$/d')
if [[ -n "$placeholder_hits" ]]; then
  printf '%s\n' "$placeholder_hits"
  exit 1
fi

exit 0
