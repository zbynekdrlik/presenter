#!/usr/bin/env bash
set -euo pipefail
# Encoder-ready gate for presenter.service / presenter-dev.service (#339, #539, #541, #547).
#
# WHY IT EXISTS (#339): `After=dev-dri-renderD128.device` fires as soon as udev creates
# the node, but the i915 / VA-API features can take another 30-50s to register. Starting
# presenter into that window left it with no encoder and NDI down until a manual restart
# (prod incident 2026-05-24). So the unit waits here until the encoder the server will
# actually use can really encode on this host.
#
# WHY IT SKIPS (#539): the old inline `until gst-inspect-1.0 --exists vah264enc || …`
# burned the FULL 30s timeout on any host that can never satisfy it — a host with no GPU,
# or (the PP case, #540) one where the service user cannot open the render node. So we
# check that a render node exists AND is openable first, and skip the wait outright when
# it is not: there is nothing to wait for.
#
# WHY THE ENCODER LIST IS WHAT IT IS (#541): NVIDIA 595 killed the legacy `nvh264enc`
# ("Selected preset not supported"), so a current NVIDIA host registers only the modern
# nvcodec elements. Waiting for the legacy name alone would burn the timeout on a
# perfectly healthy box.
#
# WHY IT PROBES THIS WAY (#547) — the two traps this gate fell into before:
#
#   1. `gst-inspect-1.0 --exists <name>` is a REGISTRY-CACHE LOOKUP, not a plugin load
#      (measured: 14ms warm vs 916ms cold, and no vaInitialize on the warm path). So a
#      poll loop built on it performs ONE real scan and then re-reads that same verdict
#      every second — it can never see a late-registering encoder, which is the entire
#      #339 scenario it exists for. Worse, a start that INHERITS a warm registry (reboot,
#      Restart=always; the deploy-time delete of #544 only covers the first start after a
#      deploy) scans nothing at all. So: DELETE the registry before every poll. The
#      probe rebuilds it, and the last one leaves a correct registry for the server.
#
#   2. Name-presence is not the server's question. Since #541 the server decides with a
#      real one-frame encode (`presenter_ndi::probe_encoder_can_encode`) — an element can
#      be registered and still be rejected by the driver. A name-only gate therefore
#      green-lit encoders the server then refused. So we run the SAME encode here.
#
# EVERY external call is bounded by `timeout`: a gst call blocking in a wedged driver
# (#445) would otherwise hang ExecStartPre until TimeoutStartSec and FAIL the unit —
# taking lyrics, Bible and timers down over a missing encoder. The unit bounds the whole
# script as well (belt and braces).
#
# ALWAYS EXITS 0. A missing encoder is not fatal — the server starts and serves lyrics,
# Bible, timers and the stage display without NDI, and logs why (see presenter_ndi::gpu).
#
# Testable without a GPU: `PRESENTER_RENDER_NODE`, `PRESENTER_ENCODER_WAIT_SECS` and
# `PRESENTER_ENCODER_PROBE_TIMEOUT_SECS` are injectable and the probe goes through
# `gst-launch-1.0` on PATH. See tests/deploy/deploy-config.test.sh.

RENDER_NODE="${PRESENTER_RENDER_NODE:-/dev/dri/renderD128}"
WAIT_SECS="${PRESENTER_ENCODER_WAIT_SECS:-30}"
PROBE_TIMEOUT_SECS="${PRESENTER_ENCODER_PROBE_TIMEOUT_SECS:-8}"
REGISTRY="${GST_REGISTRY:-}"

# Priority order mirrors presenter_ndi::H264_ENCODER_CANDIDATES (minus the software
# fallback, which needs no GPU and therefore no wait).
ENCODERS=(vah264enc nvcudah264enc nvautogpuh264enc nvh264enc)

# The same one-frame encode the server runs before it trusts an encoder (#541). Bounded,
# silent, and side-effect-free — no hardware is held after it returns.
probe_encoder() {
  local encoder="$1"
  timeout "$PROBE_TIMEOUT_SECS" gst-launch-1.0 -q --gst-debug-level=0 \
    videotestsrc num-buffers=1 \
    ! video/x-raw,width=320,height=240,framerate=30/1 \
    ! videoconvert ! "$encoder" ! fakesink sync=false >/dev/null 2>&1
}

if [ ! -e "$RENDER_NODE" ]; then
  echo "encoder-gate: no render node at $RENDER_NODE — no GPU on this host, not waiting"
  exit 0
fi

if [ ! -r "$RENDER_NODE" ] || [ ! -w "$RENDER_NODE" ]; then
  echo "encoder-gate: $RENDER_NODE is not openable by $(id -un) — the service user needs" \
    "the 'render' group (systemd: SupplementaryGroups=render, see #540). Not waiting:" \
    "no encoder can register without access to the device."
  exit 0
fi

deadline=$((SECONDS + WAIT_SECS))
while :; do
  # #547: force a REAL plugin scan. Every gst tool answers from this cache, so without
  # the delete the poll below just re-reads the previous verdict — including a stale one
  # inherited across a reboot.
  if [ -n "$REGISTRY" ]; then
    rm -f "$REGISTRY"
  fi

  for encoder in "${ENCODERS[@]}"; do
    if probe_encoder "$encoder"; then
      echo "encoder-gate: $encoder encodes on this host — starting presenter"
      exit 0
    fi
  done

  if [ "$SECONDS" -ge "$deadline" ]; then
    break
  fi
  sleep 1
done

echo "encoder-gate: no H264 encoder could encode within ${WAIT_SECS}s (tried: ${ENCODERS[*]})." \
  "Starting presenter anyway — NDI will be unavailable until an encoder appears."
exit 0
