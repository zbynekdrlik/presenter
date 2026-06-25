#!/usr/bin/env bash
set -euo pipefail

# ============================================================================
# Self-test / regression guard for the file-size CI gate (#407).
#
# The gate counts a file's PRODUCTION lines as everything before its inline
# `#[cfg(test)]` test module. The old logic stopped at the FIRST `#[cfg(test)]`
# of any kind — so a file with an EARLY `#[cfg(test)] mod tests;` declaration
# (test code in a SEPARATE, exempt file) had ALL its real production code after
# that line counted as "test" and silently undercounted: 1100-line files slid
# past the 1000-line hard cap. The fix (scripts/dev/count_prod_lines.sh) stops
# ONLY at an inline `mod <name> {` module BODY. This script proves it.
#
# What it proves:
#   1. GREEN — count_prod_lines.sh counts a >1000-line file that has an EARLY
#      `#[cfg(test)] mod tests;` declaration at its FULL size (gate would FIRE).
#   2. RED   — the OLD buggy awk reports a tiny count on that same fixture (a
#      silent bypass). Pins the FIX, not just current behavior.
#   3. Inline test module is still excluded (production count stops at the body).
# ============================================================================

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
COUNTER="$ROOT_DIR/scripts/dev/count_prod_lines.sh"

if [[ ! -f "$COUNTER" ]]; then
  echo "::error::file-size gate self-test: counter not found at $COUNTER" >&2
  exit 1
fi

pass_count=0
fail_count=0
ok()  { echo "  PASS: $*"; pass_count=$((pass_count + 1)); }
bad() { echo "::error::file-size gate self-test FAILED: $*" >&2; fail_count=$((fail_count + 1)); }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# --- Fixture 1: early `mod tests;` declaration + 1100 production lines --------
FIX_EARLY="$WORK/early_mod_tests.rs"
{
  echo '#[cfg(test)]'
  echo 'mod tests;'
  echo ''
  for i in $(seq 1 1100); do echo "pub fn prod_${i}() -> u32 { ${i} }"; done
} > "$FIX_EARLY"

# (1) GREEN: the fixed counter sees the real production size (>1000).
got=$(bash "$COUNTER" "$FIX_EARLY")
if (( got > 1000 )); then
  ok "early-mod-tests file counted at full prod size ($got > 1000) — gate fires"
else
  bad "early-mod-tests file undercounted: got $got, expected > 1000"
fi

# (2) RED: the OLD buggy awk (stop at first #[cfg(test)]) undercounts → bypass.
old_test_line=$(awk '/^\s*#\[cfg\(test\)\]/{print NR; exit}' "$FIX_EARLY")
old_count=$(( old_test_line - 1 ))
if (( old_count <= 1000 )); then
  ok "old buggy logic undercounts the same file ($old_count) — confirms the bug the fix closes"
else
  bad "old logic unexpectedly counted $old_count — fixture does not reproduce #407"
fi

# --- Fixture 2: inline `#[cfg(test)] mod tests { ... }` must still be excluded -
FIX_INLINE="$WORK/inline_mod_tests.rs"
{
  for i in $(seq 1 40); do echo "pub fn prod_${i}() -> u32 { ${i} }"; done
  echo '#[cfg(test)]'
  echo 'mod tests {'
  for i in $(seq 1 60); do echo "    #[test] fn t_${i}() { assert_eq!(1, 1); }"; done
  echo '}'
} > "$FIX_INLINE"
got_inline=$(bash "$COUNTER" "$FIX_INLINE")
# 40 prod fns + the `#[cfg(test)]` boundary at line 41 → 40 production lines.
if (( got_inline == 40 )); then
  ok "inline test module excluded (production count = $got_inline)"
else
  bad "inline test module mis-counted: got $got_inline, expected 40"
fi

echo "file-size gate self-test: $pass_count passed, $fail_count failed"
(( fail_count == 0 )) || exit 1
