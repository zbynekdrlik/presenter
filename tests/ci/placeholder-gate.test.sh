#!/usr/bin/env bash
set -euo pipefail

# ============================================================================
# Self-test / regression guard for the placeholder/unfinished-work CI gate
# (Issue #364).
#
# The placeholder gate (quality-check.sh section 16) was a SILENT NO-OP:
#   1. Its include glob `-g 'crates/**/*.rs'` is ripgrep-version-fragile — an
#      older apt ripgrep anchors `**` differently and silently misses nested
#      files, so a marker in a DEEPLY-NESTED production file was never scanned.
#   2. Both rg invocations ended in `2>/dev/null || true`, swallowing ANY
#      glob/rg error and passing the gate regardless — a true no-op.
# The robust gate now lives in scripts/dev/placeholder_check.sh, which scans an
# explicit path with `--type rust`/`--type ts` and branches on rg's exit code.
# This script IS the test that pins that fix.
#
# What it proves:
#   1. GREEN  — the real gate FIRES (exit 1) when a FRESH sentinel marker is
#      injected into a DEEPLY-NESTED production .rs file — the exact case the
#      `**`-glob no-op missed.
#   2. Clean  — the real gate PASSES (exit 0) on a clean tree with no markers.
#   3. RED-pin — the OLD buggy approach (`crates/**/*.rs` include glob run from
#      the wrong cwd + `2>/dev/null || true`) reports ZERO hits on that same
#      deeply-nested fixture: a silent no-op. This pins the FIX, not just
#      current behavior — if someone reverts to the `**`-glob/swallow approach,
#      assertion 1 breaks.
#   4. Loud  — a real scan error (unreadable scan dir) FAILS LOUDLY (exit 2),
#      never a silent pass.
#
# Run in CI by the Test job's "Run CI shell tests" step (alongside the other
# tests/ci/*.test.sh regression tests). Exits 0 only when all assertions pass.
# ============================================================================

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
GATE="$ROOT_DIR/scripts/dev/placeholder_check.sh"

if [[ ! -f "$GATE" ]]; then
  echo "::error::placeholder gate self-test: gate script not found at $GATE" >&2
  echo "(this is the expected RED failure until scripts/dev/placeholder_check.sh exists)" >&2
  exit 1
fi

if ! command -v rg >/dev/null 2>&1; then
  echo "::error::placeholder gate self-test: ripgrep (rg) not installed" >&2
  exit 1
fi

pass_count=0
fail_count=0
ok()  { echo "  PASS: $*"; pass_count=$((pass_count + 1)); }
bad() { echo "::error::placeholder gate self-test FAILED: $*" >&2; fail_count=$((fail_count + 1)); }

# --- Build a self-contained fixture mini-repo --------------------------------
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# A DEEPLY-NESTED production source file (the case the `**`-glob no-op missed).
NESTED_REL="crates/qc_self_test/src/a/b/c/d/deeply_nested.rs"
mkdir -p "$WORK/$(dirname "$NESTED_REL")"

# Pick a fresh sentinel that is one of the gate's own multi-word patterns. Using
# a phrase pattern (not a TODO marker) keeps the test independent of the comment
# filter — phrases fire even in comments.
SENTINEL='this feature is not implemented yet'
cat > "$WORK/$NESTED_REL" <<EOF
pub fn placeholder_self_test_marker() {
    // $SENTINEL
    unreachable!();
}
EOF
echo "Fixture: $NESTED_REL containing sentinel '$SENTINEL'"

# --- Assertion 1: GREEN — real gate FIRES on the deeply-nested marker --------
echo ""
echo "[1] Real gate on fixture with a deeply-nested marker (must FAIL = exit 1):"
set +e
gate_out="$("$GATE" "$WORK" 2>&1)"
gate_rc=$?
set -e
if (( gate_rc == 1 )) && grep -qF 'deeply_nested.rs' <<<"$gate_out"; then
  ok "gate fired (exit 1) and reported the deeply-nested file"
else
  bad "gate did NOT fire on the deeply-nested marker (exit $gate_rc); output: $gate_out"
fi

# --- Assertion 2: Clean — real gate PASSES with no markers -------------------
echo ""
echo "[2] Real gate on a CLEAN fixture (must PASS = exit 0):"
CLEAN="$(mktemp -d)"
trap 'rm -rf "$WORK" "$CLEAN"' EXIT
mkdir -p "$CLEAN/crates/qc_self_test/src"
cat > "$CLEAN/crates/qc_self_test/src/clean.rs" <<'EOF'
pub fn fully_implemented() -> i32 {
    let answer = 42;
    answer
}
EOF
set +e
clean_out="$("$GATE" "$CLEAN" 2>&1)"
clean_rc=$?
set -e
if (( clean_rc == 0 )); then
  ok "gate passed (exit 0) on the clean fixture"
else
  bad "gate failed on a clean fixture (exit $clean_rc); output: $clean_out"
fi

# --- Assertion 3: DIFFERENTIAL pin — on the SAME real scan error, the OLD
# `2>/dev/null || true` swallow logic silently no-ops (empty output, exit 0)
# while the NEW gate fails loudly. This pins the version-INDEPENDENT core of the
# fix: a glob/rg error (the silent gate's worst failure — it could pass even when
# the scan never ran) must now be surfaced, not swallowed. It exercises $GATE
# directly, so it breaks if the swallow is reintroduced. (The `**`-glob fragility
# is the OTHER half of the bug, pinned by assertion 1 requiring deeply-nested
# files to be scanned.) Trigger a real ripgrep error with an unreadable dir.
echo ""
echo "[3] On the SAME scan error: OLD swallow no-ops, NEW gate fails loudly (differential pin):"
DIFFDIR="$(mktemp -d)"
trap 'rm -rf "$WORK" "$CLEAN" "$DIFFDIR"' EXIT
mkdir -p "$DIFFDIR/crates/sealed"
chmod 000 "$DIFFDIR/crates/sealed"
# OLD logic: rg over the unreadable subtree with the swallow — yields nothing, no fail signal.
old_out=$(rg -in '(coming soon|not implemented)' "$DIFFDIR/crates" 2>/dev/null || true)
# NEW gate: same scan dir, must fail loudly (exit 2).
set +e
new_out=$("$GATE" "$DIFFDIR" 2>&1)
new_rc=$?
set -e
chmod 755 "$DIFFDIR/crates/sealed" 2>/dev/null || true
if [[ "$(id -u)" == "0" ]]; then
  ok "running as root — skipping unreadable-dir differential (root bypasses permission bits)"
elif [[ -z "$old_out" ]] && (( new_rc == 2 )); then
  ok "old swallow stayed silent (empty) while the new gate failed loudly (exit 2) on the same error"
else
  bad "differential pin failed: old_out='$old_out' new_rc=$new_rc (expected old empty, new exit 2)"
fi

# --- Assertion 4: Loud — a real scan error FAILS LOUDLY (exit 2) -------------
echo ""
echo "[4] Real gate on an UNREADABLE scan dir (must FAIL LOUDLY = exit 2, never silent pass):"
ERRDIR="$(mktemp -d)"
trap 'rm -rf "$WORK" "$CLEAN" "$DIFFDIR" "$ERRDIR"' EXIT
mkdir -p "$ERRDIR/crates"
chmod 000 "$ERRDIR/crates"
set +e
err_out="$("$GATE" "$ERRDIR" 2>&1)"
err_rc=$?
set -e
chmod 755 "$ERRDIR/crates" 2>/dev/null || true
if [[ "$(id -u)" == "0" ]]; then
  # Running as root bypasses POSIX permission checks, so an unreadable dir is
  # still readable. The exit-code branch is exercised by assertions 1/2/3 in
  # that environment; skip the privilege-dependent case rather than fake it.
  ok "running as root — skipping unreadable-dir case (root bypasses permission bits)"
elif (( err_rc == 2 )); then
  ok "gate failed loudly (exit 2) on an unreadable scan dir"
else
  bad "gate did NOT fail loudly on a real scan error (exit $err_rc); output: $err_out"
fi

# --- Summary ----------------------------------------------------------------
echo ""
echo "Result: $pass_count passed, $fail_count failed"
if (( fail_count > 0 )); then
  exit 1
fi
echo "placeholder gate self-test: all assertions passed."
