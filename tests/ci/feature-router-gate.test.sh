#!/usr/bin/env bash
set -euo pipefail

# ============================================================================
# Self-test / regression guard for the feature-router presence gate
# (Issue #605 — pins the directory-aware fix shipped in PR #604, commit 9521cf94;
# #625 — made this test actually EXERCISE the real gate script instead of a
# hand-copied duplicate).
#
# quality-check.sh section 1 checks that the four feature routers (bible,
# libraries, playlists, presentations) exist. The original check hard-coded a
# FLAT-FILE assumption (`router/<name>.rs`). When #590 split `bible.rs` into a
# `bible/` DIRECTORY, the check false-failed. The fix generalized it to accept
# either `router/<name>.rs` OR `router/<name>/mod.rs`, and #625 extracted that
# logic into scripts/dev/feature_router_check.sh (the same script
# quality-check.sh itself calls) so THIS self-test invokes the EXACT SAME
# production code — not a copy that could silently drift out of sync with a
# revert to the real check (the bug #625 was filed to fix: assertions 1-3/5
# used to call a hand-copied `check_routers_present` reimplementation, so
# reverting quality-check.sh's real check to flat-file-only left every one of
# them green).
#
# What it proves:
#   1. GREEN — the real gate script PASSES when all four routers exist as
#      FLAT FILES (`<name>.rs`).
#   2. GREEN — the real gate script PASSES when all four routers exist as
#      DIRECTORIES (`<name>/mod.rs`) — the exact case the OLD flat-file-only
#      check false-failed on.
#   3. GREEN — MIXED shapes (some flat, some dir) PASS.
#   4. RED   — the OLD flat-file-only check (a deliberate HISTORICAL snapshot
#      of the pre-9521cf94 logic, not the live script) FAILS on the
#      directory-only fixture. This pins the FIX, not just current behavior.
#   5. RED   — the real gate script FAILS when one router is MISSING (the
#      core negative case — catches accidental deletion of a router).
#
# Run in CI by the Test job's "Run CI shell tests" step (alongside the other
# tests/ci/*.test.sh regression tests). Exits 0 only when all assertions pass.
# ============================================================================

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CHECKER="$ROOT_DIR/scripts/dev/feature_router_check.sh"

if [[ ! -f "$CHECKER" ]]; then
  echo "::error::feature-router gate self-test: feature_router_check.sh not found at $CHECKER" >&2
  exit 1
fi

pass_count=0
fail_count=0
ok()  { echo "  PASS: $*"; pass_count=$((pass_count + 1)); }
bad() { echo "::error::feature-router gate self-test FAILED: $*" >&2; fail_count=$((fail_count + 1)); }

# Extract ONLY the feature-router presence check (section 1) from
# quality-check.sh so the test exercises the REAL production logic in isolation,
# without triggering the heavyweight fmt/clippy/cargo-deny gates. #625: this
# now calls scripts/dev/feature_router_check.sh — the SAME script
# quality-check.sh section 1 invokes — instead of a hand-copied
# reimplementation. A reimplementation could silently drift from (or a
# revert could silently un-fix) the real gate while this self-test stayed
# green regardless; calling the actual script closes that gap.
check_routers_present() {
  local base="$1"
  bash "$CHECKER" "$base"
}

# The OLD flat-file-only check (pre-9521cf94) — for the differential pin.
check_routers_flat_only() {
  local base="$1"
  local missing=0
  local missing_name=""
  for name in bible libraries playlists presentations; do
    flat="$base/crates/presenter-server/src/router/${name}.rs"
    if [[ ! -f "$flat" ]]; then
      missing=1
      missing_name="$name"
      break
    fi
  done
  if (( missing == 1 )); then
    echo "MISSING:$missing_name"
    return 1
  fi
  echo "OK"
  return 0
}

ROUTER_BASE="crates/presenter-server/src/router"

# ---------------------------------------------------------------------------
# Fixture builder: creates a temp repo with the four feature routers in a
# given shape. Usage: build_fixture <shape>  where shape = flat | dir | mixed
# ---------------------------------------------------------------------------
build_fixture() {
  local shape="$1"
  local work
  work="$(mktemp -d)"
  mkdir -p "$work/$ROUTER_BASE"
  case "$shape" in
    flat)
      for name in bible libraries playlists presentations; do
        echo "// $name router (flat)" > "$work/$ROUTER_BASE/${name}.rs"
      done
      ;;
    dir)
      for name in bible libraries playlists presentations; do
        mkdir -p "$work/$ROUTER_BASE/${name}"
        echo "// $name router (dir)" > "$work/$ROUTER_BASE/${name}/mod.rs"
      done
      ;;
    mixed)
      # bible + playlists as directories (the real #590 split shape), the rest flat
      for name in bible playlists; do
        mkdir -p "$work/$ROUTER_BASE/${name}"
        echo "// $name router (dir)" > "$work/$ROUTER_BASE/${name}/mod.rs"
      done
      for name in libraries presentations; do
        echo "// $name router (flat)" > "$work/$ROUTER_BASE/${name}.rs"
      done
      ;;
    *)
      echo "::error::unknown fixture shape: $shape" >&2
      return 1
      ;;
  esac
  echo "$work"
}

# --- Assertion 1: GREEN — all flat files -------------------------------------
echo "[1] All four routers as FLAT FILES (must PASS = OK):"
WORK="$(build_fixture flat)"
trap 'rm -rf "$WORK"' EXIT
set +e
res="$(check_routers_present "$WORK")"
rc=$?
set -e
if (( rc == 0 )) && [[ "$res" == "OK" ]]; then
  ok "flat-file fixture accepted (exit 0)"
else
  bad "flat-file fixture rejected (exit $rc, res='$res')"
fi
rm -rf "$WORK"
trap - EXIT

# --- Assertion 2: GREEN — all directories (the #590 split case) --------------
echo ""
echo "[2] All four routers as DIRECTORIES (must PASS = OK — the #590 case):"
WORK="$(build_fixture dir)"
trap 'rm -rf "$WORK"' EXIT
set +e
res="$(check_routers_present "$WORK")"
rc=$?
set -e
if (( rc == 0 )) && [[ "$res" == "OK" ]]; then
  ok "directory fixture accepted (exit 0) — the #590 split case passes"
else
  bad "directory fixture rejected (exit $rc, res='$res')"
fi
rm -rf "$WORK"
trap - EXIT

# --- Assertion 3: GREEN — mixed shapes ---------------------------------------
echo ""
echo "[3] MIXED shapes (two flat, two dir) — must PASS = OK:"
WORK="$(build_fixture mixed)"
trap 'rm -rf "$WORK"' EXIT
set +e
res="$(check_routers_present "$WORK")"
rc=$?
set -e
if (( rc == 0 )) && [[ "$res" == "OK" ]]; then
  ok "mixed-shape fixture accepted (exit 0)"
else
  bad "mixed-shape fixture rejected (exit $rc, res='$res')"
fi
rm -rf "$WORK"
trap - EXIT

# --- Assertion 4: RED differential — OLD flat-only check FAILS on dirs -------
echo ""
echo "[4] DIFFERENTIAL pin — OLD flat-only check on directory fixture (must FAIL):"
WORK="$(build_fixture dir)"
trap 'rm -rf "$WORK"' EXIT
set +e
old_res="$(check_routers_flat_only "$WORK")"
old_rc=$?
set -e
if (( old_rc == 1 )); then
  ok "old flat-only check correctly FAILED on directory fixture (pins the bug the fix closes)"
else
  bad "old flat-only check accepted directory fixture (exit $old_rc) — differential broken"
fi
rm -rf "$WORK"
trap - EXIT

# --- Assertion 5: RED — generalized check FAILS when one router is missing ---
echo ""
echo "[5] Generalized check with one MISSING router (must FAIL):"
WORK="$(build_fixture dir)"
trap 'rm -rf "$WORK"' EXIT
# Remove one router entirely — the generalized check must fire.
rm -rf "$WORK/$ROUTER_BASE/bible"
set +e
res="$(check_routers_present "$WORK")"
rc=$?
set -e
if (( rc == 1 )) && [[ "$res" == "MISSING:bible" ]]; then
  ok "missing-router detected (exit 1, reported 'bible')"
else
  bad "missing-router NOT detected (exit $rc, res='$res')"
fi
rm -rf "$WORK"
trap - EXIT

# --- Summary ----------------------------------------------------------------
echo ""
echo "Result: $pass_count passed, $fail_count failed"
if (( fail_count > 0 )); then
  exit 1
fi
echo "feature-router gate self-test: all assertions passed."
