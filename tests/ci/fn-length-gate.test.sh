#!/usr/bin/env bash
set -euo pipefail

# ============================================================================
# Self-test / regression guard for the function-length CI gate (#374).
#
# The function-length gate (quality-check.sh section 7 -> scripts/dev/
# fn_length_check.py) was a SILENT NO-OP in every scoped CI run because it
# compared os.walk's ABSOLUTE paths against `git diff --name-only`'s RELATIVE
# targets — the filter `continue`d on every file, so the >120 hard-fail never
# fired. The bug was fixed in 5f5fe9f (compare `os.path.relpath(path, root)`),
# but there was NO test that would catch it if the abs/rel comparison silently
# regressed again. This script IS that test.
#
# What it proves:
#   1. GREEN  — the real, current checker FIRES (>=1 violation) when given a
#      RELATIVE-path target containing an over-cap (>120 line) function — the
#      exact way CI scopes the gate via `git diff --name-only`.
#   2. RED    — a copy of the checker with the abs/rel bug REINTRODUCED reports
#      ZERO violations on that same relative-path fixture (a silent no-op). This
#      pins the FIX, not just current behavior: if someone reverts to the
#      absolute-path comparison, assertion (1) breaks.
#   3. Sanity — the real checker no-ops on an ABSOLUTE-path target (confirming
#      relative comparison is what makes scoping work) and still fires unscoped.
#
# Run in CI by the Test job's "Run CI shell tests" step (alongside the other
# tests/ci/*.test.sh regression tests). Exits 0 only when all assertions pass.
# ============================================================================

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CHECKER="$ROOT_DIR/scripts/dev/fn_length_check.py"

if [[ ! -f "$CHECKER" ]]; then
  echo "::error::fn-length gate self-test: checker not found at $CHECKER" >&2
  exit 1
fi

pass_count=0
fail_count=0

ok()   { echo "  PASS: $*"; pass_count=$((pass_count + 1)); }
bad()  { echo "::error::fn-length gate self-test FAILED: $*" >&2; fail_count=$((fail_count + 1)); }

# --- Build a self-contained fixture mini-repo --------------------------------
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

FIXTURE_REL="crates/qc_self_test/src/over_cap.rs"
mkdir -p "$WORK/$(dirname "$FIXTURE_REL")"
{
  echo 'pub fn known_over_cap_function() {'
  # 130 body lines -> total function length is clearly > 120 hard cap
  for i in $(seq 1 130); do
    echo "    let _v$i = $i;"
  done
  echo '}'
} > "$WORK/$FIXTURE_REL"

fn_lines="$(wc -l < "$WORK/$FIXTURE_REL")"
if (( fn_lines <= 120 )); then
  bad "fixture function is only $fn_lines lines; must exceed the 120-line hard cap"
  echo "Result: $pass_count passed, $fail_count failed"; exit 1
fi
echo "Fixture: $FIXTURE_REL ($fn_lines lines, > 120 hard cap)"

# Count violations for a given checker + QC_TARGETS value.
# Args: <checker_path> <root> <qc_targets>
count_violations() {
  local checker="$1" root="$2" targets="$3"
  QC_TARGETS="$targets" QC_FN_ADVISORY=0 python3 "$checker" "$root" \
    | python3 -c 'import json,sys; print(len(json.load(sys.stdin)["violations"]))'
}

# --- Assertion 1: GREEN — real checker FIRES on a RELATIVE-path target -------
echo ""
echo "[1] Real (fixed) checker, RELATIVE target (CI style):"
rel_violations="$(count_violations "$CHECKER" "$WORK" "$FIXTURE_REL")"
if (( rel_violations >= 1 )); then
  ok "gate fired ($rel_violations violation(s)) on relative-path over-cap fixture"
else
  bad "gate did NOT fire on relative-path fixture (got $rel_violations violations) — the abs/rel no-op regression is back"
fi

# --- Assertion 2: RED — reintroduce the abs/rel bug, expect a silent no-op ---
# Reproduce the OLD buggy logic: compare the ABSOLUTE walk path against the
# (relative) targets, exactly as before 5f5fe9f. The fixed line is:
#     if targets and rel not in targets:
# The buggy line was:
#     if targets and path not in targets:
echo ""
echo "[2] Buggy checker (abs/rel bug reintroduced), RELATIVE target:"
BUGGY="$WORK/fn_length_check_buggy.py"
sed 's/if targets and rel not in targets:/if targets and path not in targets:/' "$CHECKER" > "$BUGGY"
if ! grep -q 'if targets and path not in targets:' "$BUGGY"; then
  bad "could not reintroduce the abs/rel bug into the checker copy (the fixed line moved?) — update this self-test"
else
  buggy_violations="$(count_violations "$BUGGY" "$WORK" "$FIXTURE_REL")"
  if (( buggy_violations == 0 )); then
    ok "buggy (absolute-path) logic no-ops as expected ($buggy_violations violations) — the test distinguishes fixed from broken"
  else
    bad "buggy logic still reported $buggy_violations violations; the self-test does NOT actually pin the abs/rel fix"
  fi
fi

# --- Assertion 3: Sanity — real checker no-ops on an ABSOLUTE target ----------
echo ""
echo "[3] Real checker, ABSOLUTE target (must not match -> no-op):"
abs_violations="$(count_violations "$CHECKER" "$WORK" "$WORK/$FIXTURE_REL")"
if (( abs_violations == 0 )); then
  ok "absolute-path target does not match relative scope (correct — confirms relative comparison drives scoping)"
else
  bad "absolute-path target unexpectedly matched ($abs_violations violations)"
fi

# --- Assertion 4: Sanity — unscoped run still finds the fixture ---------------
echo ""
echo "[4] Real checker, NO targets (whole-tree scan) finds the fixture:"
unscoped_violations="$(count_violations "$CHECKER" "$WORK" "")"
if (( unscoped_violations >= 1 )); then
  ok "unscoped scan reports the over-cap function ($unscoped_violations violation(s))"
else
  bad "unscoped scan missed the over-cap fixture (got $unscoped_violations)"
fi

# --- Summary ----------------------------------------------------------------
echo ""
echo "Result: $pass_count passed, $fail_count failed"
if (( fail_count > 0 )); then
  exit 1
fi
echo "fn-length gate self-test: all assertions passed."
