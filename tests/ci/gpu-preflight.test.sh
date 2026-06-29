#!/usr/bin/env bash
# Regression tests for scripts/ci/gpu-preflight.sh (#445).
#
# The e2e-ndi lane on the dev2 self-hosted runner fails opaquely when the shared
# RTX 5050 wedges: nvcodec stops registering `nvh264enc`, presenter-ndi's encoder
# build returns 500, and all 6 @synthetic-ndi tests fail with no hint it's a GPU
# wedge. The preflight detects the wedge and fails the lane FAST with an
# actionable message (run the recover-hung-gpu skill).
#
# A real GPU wedge can't be produced in CI, so the preflight reasons about three
# injectable facts (GPU_PREFLIGHT_FAKE=1 enables mock mode — no live tool calls):
#   GPU_PREFLIGHT_FAKE_NVENC      = "ok" | "missing"   (fresh gst-inspect result)
#   GPU_PREFLIGHT_FAKE_SMI_UTILMEM= "<util>, <mem_mib>" (nvidia-smi query)
#   GPU_PREFLIGHT_FAKE_SMI_PROCS  = newline list of compute-app PIDs (empty=none)
#   GPU_PREFLIGHT_FAKE_DMESG      = dmesg text
# This test feeds wedged-vs-healthy mocks and asserts exit non-zero on wedged,
# zero on healthy — a real RED->GREEN proof of the detection logic.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="$SCRIPT_DIR/../../scripts/ci/gpu-preflight.sh"

fail=0

# Helper: run the preflight in mock mode. Args: nvenc utilmem procs dmesg
# Captures combined output into $OUT and exit code into $RC.
run_preflight() {
    set +e
    OUT="$(GPU_PREFLIGHT_FAKE=1 \
        GPU_PREFLIGHT_FAKE_NVENC="$1" \
        GPU_PREFLIGHT_FAKE_SMI_UTILMEM="$2" \
        GPU_PREFLIGHT_FAKE_SMI_PROCS="$3" \
        GPU_PREFLIGHT_FAKE_DMESG="$4" \
        bash "$SCRIPT" 2>&1)"
    RC=$?
    set -e
}

# Case 1 — HEALTHY busy GPU: NVENC registers, real compute process running.
# This is the live state during a normal e2e-ndi run (bakerion inference up).
# The preflight MUST pass so it never false-reds a good PR.
run_preflight "ok" "76, 1193" "995493" ""
if [ "$RC" -ne 0 ]; then
    echo "FAIL case1: healthy busy GPU (nvenc ok, process running) must pass; rc=$RC" >&2
    echo "  output: $OUT" >&2
    fail=1
fi

# Case 2 — HEALTHY IDLE GPU: NVENC registers, no process, low util, low mem.
# An idle GPU is NOT a wedge; the wedge signature requires HIGH util.
run_preflight "ok" "0, 2" "" ""
if [ "$RC" -ne 0 ]; then
    echo "FAIL case2: healthy idle GPU must pass (idle != wedge); rc=$RC" >&2
    echo "  output: $OUT" >&2
    fail=1
fi

# Case 3 — CLASSIC WEDGE (the #445 bug): NVENC gone, 100% util, ~2 MiB mem, NO
# compute process, dmesg shows NV_ERR_RESET_REQUIRED. Must FAIL with a message
# that names the recovery skill and the reset reason.
run_preflight "missing" "100, 2" "" \
    "NVRM: nvAssertOkFailedNoLog: Assertion failed: Reset required [NV_ERR_RESET_REQUIRED] (0x00000062)"
if [ "$RC" -eq 0 ]; then
    echo "FAIL case3: classic GPU wedge must FAIL the preflight; rc=$RC" >&2
    fail=1
fi
if ! printf '%s' "$OUT" | grep -q "recover-hung-gpu"; then
    echo "FAIL case3: wedge message must reference the recover-hung-gpu skill" >&2
    echo "  output: $OUT" >&2
    fail=1
fi
if ! printf '%s' "$OUT" | grep -qiE "reset required|NV_ERR_RESET_REQUIRED|wedged"; then
    echo "FAIL case3: wedge message must mention the reset/wedge reason" >&2
    echo "  output: $OUT" >&2
    fail=1
fi

# Case 4 — PURE-WEDGE SIGNATURE even if nvcodec still reports nvh264enc (stale):
# 100% util + tiny mem + NO compute process is an unambiguous wedge. Must FAIL.
run_preflight "ok" "100, 2" "" ""
if [ "$RC" -eq 0 ]; then
    echo "FAIL case4: pure nvidia-smi wedge signature must FAIL even if nvenc probes ok; rc=$RC" >&2
    echo "  output: $OUT" >&2
    fail=1
fi

# Case 5 — NVENC unavailable for some other reason (no clear wedge signature):
# still fail, still point at the recovery skill so the failure is actionable.
run_preflight "missing" "50, 500" "995493" ""
if [ "$RC" -eq 0 ]; then
    echo "FAIL case5: missing nvh264enc must FAIL the preflight; rc=$RC" >&2
    fail=1
fi
if ! printf '%s' "$OUT" | grep -q "recover-hung-gpu"; then
    echo "FAIL case5: missing-NVENC message must reference the recover-hung-gpu skill" >&2
    echo "  output: $OUT" >&2
    fail=1
fi

# Case 6 — STALE-DMESG FALSE-POSITIVE GUARD (observed live on 2026-06-29): the
# dmesg ring buffer still holds OLD NV_ERR_RESET_REQUIRED lines from BEFORE a
# driver reload/recovery, while the GPU is now healthy (nvenc ok, process up).
# dmesg ALONE must NOT fail the preflight, or every run after a recovery reds.
run_preflight "ok" "76, 1193" "995493" \
    "NVRM: nvCheckOkFailedNoLog: ... [NV_ERR_RESET_REQUIRED] (0x00000062) (stale, pre-recovery)"
if [ "$RC" -ne 0 ]; then
    echo "FAIL case6: stale dmesg reset line with healthy GPU must NOT fail (false-positive guard); rc=$RC" >&2
    echo "  output: $OUT" >&2
    fail=1
fi

if [ "$fail" -ne 0 ]; then
    echo "Shell tests for scripts/ci/gpu-preflight.sh: FAILED" >&2
    exit 1
fi

echo "Shell tests for scripts/ci/gpu-preflight.sh: all passed"
