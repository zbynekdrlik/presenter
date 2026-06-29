---
name: presenter-ndi
description: >
  NDI pipeline architecture, SDK setup, WebRTC testing, and debugging for the presenter project.
  Use when working on NDI stage display, WebRTC fanout, or debugging NDI video issues.
triggers:
  - NDI
  - WebRTC
  - stage display
  - nvh264enc
  - gstreamer
  - StreamProducer
---

# Presenter NDI Skill

## SDK Setup

- NDI SDK v6.3.1 installed at `/usr/lib/ndi/libndi.so.6`
- `avahi-daemon` MUST be running for mDNS source discovery: `sudo systemctl start avahi-daemon`
- Without avahi-daemon, `discover_sources()` returns empty even when sources exist
- Known NDI source: `STREAM-SNV (stream)` at `10.77.9.204:5961`; sends BGRX pixel format
- After installing SDK, presenter service must be restarted to pick it up

## Per-Consumer Pipeline Architecture

Since 0.4.109/0.4.110, NDI→stage fanout uses the gst-plugin-rs `webrtcsink` recipe:
encoder pipeline ends in `appsink` wrapped by `gstreamer_utils::StreamProducer`; each WHEP
consumer gets its OWN fresh `appsrc → rtph264pay → webrtcbin` pipeline on the encoder's
clock+base_time. Code: `crates/presenter-ndi/src/pipeline/consumers.rs`.

### 3 Load-Bearing Invariants (do NOT simplify away)

1. `StreamProducer::configure_consumer(&appsrc)` BEFORE pipeline goes PLAYING — basesrc latches
   `is-live` at PAUSED→PLAYING; flipping it later parks the task forever.
2. Per-consumer-pipeline bus watch MUST service `Latency` messages with `recalculate_latency()`
   — webrtcbin builds transports DURING negotiation.
3. `await_media_caps()` (waits for ssrc caps on webrtcbin sink pad) MUST run before create-answer,
   else the answer lacks `a=ssrc` and the browser drops all RTP (transport bytes climb,
   inbound-rtp stays 0).

### Low-Latency Invariants (PR #378 — also do NOT simplify away)

4. `ndisrc timestamp-mode=receive-time` — Auto mode couples PTS to Resolume's clock with drift
   correction via DISCONT → "lag builds then jumps".
5. `StreamProducer::with(.., ProducerSettings { sync: false })` — default sync=true holds every
   encoded frame to its clock deadline (~40ms).
6. GOP 240 + `request_keyframe()` (upstream ForceKeyUnit) on consumer join — GOP 30 caused 1s
   IDR pulses; long GOP REQUIRES the join keyframe.
7. **Encoder pinned to `constrained-baseline` H264 (capsfilter "profile_caps")** — High profile
   (encoder default) is rejected by strict TV HW decoders (Vestel sd2-4, 1GB RAM):
   Chromium falls back to NullVideoDecoder → black while RTP flows + endless watchdog reconnect.
   Diagnostic: logcat `NullVideoDecoder doesn't support decoding` + server sessions with
   `buffers_pushed>0` deleted every 10-30s.
8. Stage UI sets `jitterBufferTarget=0` + `playoutDelayHint=0` per receiver.

### GStreamer Tee Fanout Rule

Link the consumer branch (tee→queue→…) while it is still NULL, THEN `sync_state_with_parent()`
to PLAYING, so the tee's sticky events (caps/segment) propagate during the transition. Linking
AFTER the branch is already PLAYING → the new pad never forwards a buffer (connected, but black).

### Dev Deploy Wipes video_sources

Every `deploy-dev` run intentionally replaces dev DB with prod snapshot, then DELETEs
`video_sources` and `android_stage_displays`. **This is by design, not data loss.** To test NDI
on dev after deploy: `POST /integrations/video-sources {"label":"dd","ndiName":"RESOLUME-SNV (SP-live)"}`
then activate. Audit table is `video_source` (singular).

## WebRTC Testing / Debugging

### Codec: Use Real Chrome, Not Playwright Chromium

Playwright's bundled Chromium has NO H264 (proprietary). Always use real Chrome:
`chromium.launch({ headless:false, channel:'chrome' })` or the `chrome-video` Playwright project
(`@video-codec` tag). CI runner has Chrome at `/usr/bin/google-chrome`.

### Multi-Consumer Testing Requires Different-IP Clients

Two browsers on the same machine produce the same host ICE candidate → 2nd consumer media gets
misrouted → falsely looks like server bug. Use two different-IP clients:
1. Run Chrome on dev1: `--headless=new --remote-debugging-port=9222`
2. SSH-tunnel to dev2: `ssh -L 9222:localhost:9222 dev1`
3. Connect from dev2: `chromium.connectOverCDP('http://localhost:9222')`

### Media Flow Probe

- `getStats framesDecoded/bytesReceived` works headless — reliable.
- `<video>.videoWidth` is unreliable headless — use `xvfb-run` headed + canvas pixel variance.
- Synthetic SMPTE colorbars source (`ndi_test_sender`, name "PRESENTER-TEST") → high pixel
  variance when rendering, ~1 color when black.

### Offer Must Include Audio

A video-ONLY offer is a false guard — the deferred-tee-link bug delivered frames for video-only
but ZERO for video+audio (what every real client sends). Always:
`addTransceiver('video') + addTransceiver('audio')`.

### Debugging "Connected But Black"

Check transport-level `bytesReceived` vs `inbound-rtp`:
- High transport + 0 inbound = SSRC/demux issue
- 0 transport = no RTP sent (latency issue)

### Dev Encoder Note

Dev encoder is `nvh264enc` (RTX 5060); `vah264enc` is NOT registered on this NVIDIA box.
`GST_PLUGIN_FEATURE_RANK=nvh264enc:NONE` only affects autoplug rank, not `ElementFactory::find`.

## Cleanup After NDI/Stage Debug Sessions

After any NDI / stage-display debug session, clean up BEFORE ending:

### What Piles Up (silently, for days)

- **Test senders** `ndi_test_sender` / `ndi_clock_sender` (in `./target/debug/`) — run orphaned
  (PPID 1) at ~35% CPU EACH. `ndi_clock_sender` no longer exists in source (merged into
  `ndi_test_sender`) — any running one is a stale binary.
- **`/tmp` dumps** — `sd[1-4]*.png`, `ndi_*.png`, `*stage*.png`, `*.diff`, `stage-timings-*.log`
  (was 601MB!).
- **Playwright MCP chrome profiles** in `~/.cache/ms-playwright-mcp/` (~100-400MB each) — stale
  once owning Claude session dies.

### Cleanup Recipe

```bash
# Kill test senders by EXACT PID — NEVER pkill -f <binary-path> (kills your own shell)
kill <pid>

# Remove presenter /tmp dumps (leave other projects' files: bakerion/codex/card/torch-cuda)
rm /tmp/sd*.png /tmp/ndi_*.png /tmp/*stage*.png /tmp/*.diff /tmp/stage-timings*.log

# Prune stale playwright profiles (check owner via /proc/<pid>/cwd before deleting)
ls ~/.cache/ms-playwright-mcp/
```

**Map sessions to projects:** `readlink /proc/<pid>/cwd` to find each `claude` session's project.
Never kill another project's live session or its mcp/chrome. The heavy GPU load on dev2
(`backend-inference`, `ffmpeg`, `python3`) is the user's inference job — NOT mess, never touch.

## Observability

- `/ndi/snapshot/{id}` — per-session `buffersPushed/Dropped` + RTCP rtt/jitter/loss
- Stage UI beacons `getStats` to `POST /ndi/client-stats` every 15s (→ journald)
- Regression guards: `tests/e2e/ndi-webrtc-synthetic.spec.ts` + `tests/e2e/ndi-latency.spec.ts`
  (glass-to-glass median ≤350ms, p95 ≤600ms; measured dev 173/190ms, CI 168/192ms)

### Stage status-bar readouts (#479)

The stage status bar (`crates/presenter-ui/src/components/stage/status_bar.rs`) renders TWO
separate latency readouts: `CONNECTED · N ms` (WS RTT, ALWAYS present once the WS is up) and
`video · N ms` (rVFC decode→render latency). **The `video · N ms` readout only renders on an
NDI layout while frames are actually flowing** (it derives from the rVFC metadata observer in
`ndi_frame_stats.rs`). On a lyrics/worship layout, or with broadcast off, it is correctly
ABSENT — do NOT treat its absence as a regression when post-deploy-verifying a non-NDI stage.
To see it live, switch the stage to an NDI layout (`ndi_fullscreen`/`worship_snv`/`api_stage`)
with an active stream; otherwise its behavior is proven by the green NDI WebRTC E2E + the
`status_bar` Playwright spec on the deployed tree.
