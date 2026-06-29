#!/usr/bin/env bash
# GPU / NVENC preflight for the e2e-ndi CI lane (#445).
#
# WHY: the e2e-ndi lane runs on the dev2 self-hosted runner whose shared RTX 5050
# periodically wedges (NV_ERR_RESET_REQUIRED). When wedged, the GStreamer nvcodec
# plugin stops registering `nvh264enc`, presenter-ndi's encoder build returns 500,
# and all 6 @synthetic-ndi tests fail OPAQUELY with no hint it is a GPU wedge.
# This preflight detects the wedge and fails the lane FAST with an actionable
# message instead, pointing at the recover-hung-gpu skill (PCIe FLR, no reboot).
#
# DETECTION (any one => fail):
#   1. A fresh `gst-inspect-1.0 nvh264enc` probe is missing. This is exactly the
#      element presenter-ndi needs to build the encoder, so it is the
#      authoritative signal. The GStreamer registry cache is cleared first: a
#      stale cache left over from an earlier wedge can hide nvh264enc even after
#      the GPU recovers (false-negative) — and, conversely, report it after it is
#      genuinely gone. Clearing forces the probe to reflect the CURRENT GPU.
#   2. The classic pure-wedge nvidia-smi signature: high util (>=90%) with NO
#      CUDA compute process AND tiny memory used (<=64 MiB) — wedged clocks. (A
#      real NVENC encode offloads to the encoder ASIC and keeps utilization.gpu
#      LOW, so a legit graphics/encode consumer does not reach the >=90% gate.)
#
# dmesg is used ONLY to enrich the failure message (name the reset reason), NEVER
# as a standalone trigger: the kernel ring buffer retains pre-recovery
# NV_ERR_RESET_REQUIRED lines after a driver reload, so a dmesg-alone trigger
# would false-red every run after a recovery (observed live 2026-06-29).
#
# TESTABILITY: a real wedge can't be produced in CI. Set GPU_PREFLIGHT_FAKE=1
# (exactly "1") to enter mock mode (no live tool calls); facts are injected via:
#   GPU_PREFLIGHT_FAKE_NVENC       = "ok" | "missing"
#   GPU_PREFLIGHT_FAKE_SMI_UTILMEM = "<util>, <mem_mib>"  (nvidia-smi query form)
#   GPU_PREFLIGHT_FAKE_SMI_PROCS   = newline-separated compute-app PIDs ("" = none)
#   GPU_PREFLIGHT_FAKE_DMESG       = dmesg text
# See tests/ci/gpu-preflight.test.sh.
set -euo pipefail

# Mock mode is enabled ONLY by the exact value "1" — so GPU_PREFLIGHT_FAKE=0
# does NOT silently enter mock mode (which would default to a bogus wedge).
MOCK="${GPU_PREFLIGHT_FAKE:-}"

# Every live tool call is wrapped in `timeout` so a hard GPU hang (card off the
# bus) fails the preflight FAST instead of blocking until the job timeout.
TIMEOUT="timeout 10"

# --- fact gathering (live unless mock mode) --------------------------------

# Returns "ok" or "missing".
probe_nvenc() {
    if [ "$MOCK" = "1" ]; then
        printf '%s' "${GPU_PREFLIGHT_FAKE_NVENC:-missing}"
        return
    fi
    # Clear a possibly-stale registry so the probe reflects the current GPU.
    # Honor XDG_CACHE_HOME and any arch tuple so the clear can't silently no-op.
    rm -f "${XDG_CACHE_HOME:-$HOME/.cache}"/gstreamer-1.0/registry.*.bin 2>/dev/null || true
    if $TIMEOUT gst-inspect-1.0 nvh264enc >/dev/null 2>&1; then
        printf 'ok'
    else
        printf 'missing'
    fi
}

# Returns "<util>, <mem_mib>".
read_smi_utilmem() {
    if [ "$MOCK" = "1" ]; then
        printf '%s' "${GPU_PREFLIGHT_FAKE_SMI_UTILMEM:-0, 0}"
        return
    fi
    $TIMEOUT nvidia-smi --query-gpu=utilization.gpu,memory.used --format=csv,noheader,nounits 2>/dev/null | head -1 || true
}

# Returns newline-separated compute-app PIDs ("" = none).
read_smi_procs() {
    if [ "$MOCK" = "1" ]; then
        printf '%s' "${GPU_PREFLIGHT_FAKE_SMI_PROCS:-}"
        return
    fi
    $TIMEOUT nvidia-smi --query-compute-apps=pid --format=csv,noheader 2>/dev/null || true
}

read_dmesg() {
    if [ "$MOCK" = "1" ]; then
        printf '%s' "${GPU_PREFLIGHT_FAKE_DMESG:-}"
        return
    fi
    # dmesg may require sudo (kernel.dmesg_restrict); tolerate failure — it only
    # enriches the message and is never a standalone trigger.
    { $TIMEOUT sudo -n dmesg 2>/dev/null || $TIMEOUT dmesg 2>/dev/null || true; }
}

# --- gather ----------------------------------------------------------------

NVENC="$(probe_nvenc)"
UTILMEM="$(read_smi_utilmem)"
PROCS="$(read_smi_procs)"
DMSG="$(read_dmesg)"

# Parse util / mem as integers (default 0 if unparseable). The two fields are
# parsed independently so a non-numeric util (e.g. "[N/A]") doesn't corrupt the
# mem diagnostic: util = first integer; mem = integer after the comma.
UTIL="$(printf '%s' "$UTILMEM" | sed -n 's/^[[:space:]]*\([0-9]\+\).*/\1/p')"
MEM="$(printf '%s' "$UTILMEM" | sed -n 's/.*,[[:space:]]*\([0-9]\+\).*/\1/p')"
UTIL="${UTIL:-0}"
MEM="${MEM:-0}"

# Count non-empty compute-app PID lines.
PROC_COUNT="$(printf '%s' "$PROCS" | grep -c '[0-9]' || true)"
PROC_COUNT="${PROC_COUNT:-0}"

DMESG_RESET=0
if printf '%s' "$DMSG" | grep -qiE 'NV_ERR_RESET_REQUIRED|reset required'; then
    DMESG_RESET=1
fi

# Pure-wedge signature: high util + no process + tiny memory (wedged clocks).
WEDGE_SIG=0
if [ "$PROC_COUNT" -eq 0 ] && [ "$UTIL" -ge 90 ] && [ "$MEM" -le 64 ]; then
    WEDGE_SIG=1
fi

# --- decide ----------------------------------------------------------------

if [ "$NVENC" = "ok" ] && [ "$WEDGE_SIG" -eq 0 ]; then
    echo "GPU preflight OK: nvh264enc (NVENC) available; util=${UTIL}% mem=${MEM}MiB compute_procs=${PROC_COUNT}."
    exit 0
fi

if [ "$DMESG_RESET" -eq 1 ] || [ "$WEDGE_SIG" -eq 1 ]; then
    REASON="GPU wedged on dev2 (NV_ERR_RESET_REQUIRED / nvh264enc unavailable)"
else
    REASON="nvh264enc (NVENC) unavailable on dev2"
fi

# GitHub Actions reads workflow commands from STDOUT, so the ::error:: annotation
# must go to stdout to surface at the run level; the human diagnostics go to stderr.
echo "::error::${REASON} — run the recover-hung-gpu skill (PCIe FLR, no reboot) then re-run this job."

{
    echo ""
    echo "GPU preflight FAILED: ${REASON}"
    echo "  -> run the recover-hung-gpu skill (PCIe FLR, no reboot) then re-run this job."
    echo ""
    echo "Diagnostics:"
    echo "  nvh264enc (NVENC) probe : ${NVENC}"
    echo "  GPU utilization          : ${UTIL}%"
    echo "  GPU memory used          : ${MEM}MiB"
    echo "  compute processes        : ${PROC_COUNT}"
    echo "  pure-wedge signature     : $([ "$WEDGE_SIG" -eq 1 ] && echo yes || echo no)"
    echo "  dmesg reset-required     : $([ "$DMESG_RESET" -eq 1 ] && echo yes || echo no)"
} >&2

exit 1
