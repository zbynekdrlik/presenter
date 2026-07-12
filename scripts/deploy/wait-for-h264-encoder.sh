#!/usr/bin/env bash
set -euo pipefail
# Encoder-ready gate for presenter.service / presenter-dev.service (#339, #539, #541).
#
# WHY IT EXISTS (#339): `After=dev-dri-renderD128.device` fires as soon as udev creates
# the node, but the i915 / VA-API features can take another 30-50s to register. Starting
# presenter into that window left it with no encoder and NDI down until a manual restart
# (prod incident 2026-05-24). So the unit waits here until the encoder the server will
# actually use is visible to the SAME probe the server uses.
#
# WHY IT CHANGED (#539): the old inline `until gst-inspect-1.0 --exists vah264enc || …`
# burned the FULL 30s timeout on any host that can never satisfy it — a host with no GPU,
# or (the PP case, #540) one where the service user cannot open the render node. Every
# start paid 30s for nothing. Now we check that a render node exists AND is openable
# first, and skip the wait outright when it is not: there is nothing to wait for.
#
# WHY THE ENCODER LIST CHANGED (#541): NVIDIA 595 killed the legacy `nvh264enc`
# ("Selected preset not supported"), so a current NVIDIA host registers only the modern
# nvcodec elements. Waiting for the legacy name alone would burn the timeout on a
# perfectly healthy box.
#
# ALWAYS EXITS 0. A missing encoder is not fatal — the server starts and serves lyrics,
# Bible, timers and the stage display without NDI, and logs why (see presenter_ndi::gpu).
#
# Testable without a GPU: `PRESENTER_RENDER_NODE` / `PRESENTER_ENCODER_WAIT_SECS` are
# injectable and the encoder probe goes through `gst-inspect-1.0` on PATH.
# See tests/deploy/deploy-config.test.sh.

RENDER_NODE="${PRESENTER_RENDER_NODE:-/dev/dri/renderD128}"
WAIT_SECS="${PRESENTER_ENCODER_WAIT_SECS:-30}"

# Priority order mirrors presenter_ndi::H264_ENCODER_CANDIDATES (minus the software
# fallback, which needs no GPU and therefore no wait).
ENCODERS=(vah264enc nvcudah264enc nvautogpuh264enc nvh264enc)

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
while [ "$SECONDS" -lt "$deadline" ]; do
  for encoder in "${ENCODERS[@]}"; do
    if gst-inspect-1.0 --exists "$encoder"; then
      echo "encoder-gate: $encoder registered — starting presenter"
      exit 0
    fi
  done
  sleep 1
done

echo "encoder-gate: no H264 encoder registered within ${WAIT_SECS}s (tried: ${ENCODERS[*]})." \
  "Starting presenter anyway — NDI will be unavailable until an encoder appears."
exit 0
