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

**Encoder selection probes LOADABILITY, not name-presence (#443).** `hw_h264_encoder()`
(`presenter-ndi/src/lib.rs`) uses the pure helper `pick_h264_encoder(candidates, can_load)`
with the real probe `|name| ElementFactory::make(name).build().is_ok()` — NOT
`ElementFactory::find(name).is_some()`. Reason: a boot-race registry-cache drift (#333/#339)
can ADVERTISE `nvh264enc` while the plugin can't be instantiated → `find()` returns `Some` but
`make().build()` fails → picking on name-presence chose an unloadable encoder and the pipeline
build (+ the `pipeline::tests` skip-guards keyed on `hw_h264_encoder().is_none()`) failed with
`Failed to load element factory nvh264enc`. **When you need "is element X usable?", always
`make(X).build().is_ok()`, never `find(X).is_some()`.** It's cheap + side-effect-free
(construction only allocates the GObject; hardware opens at the READY transition), so it's safe
even on the 30s NDI-reconnect tick and is intentionally re-probed (un-memoized) so a self-healed
registry resumes without a restart. Diagnose locally with `gst-inspect-1.0 nvh264enc` (→ "No such
element" = not loadable). Unit-test the selection via the pure helper's injected `can_load` closure
— never depend on the machine's live registry.

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

## TURN relay — remote / Tailscale clients (#502)

WebRTC media needs a path the client can reach. The server gathers **LAN host candidates**
(10.77.x); a client off-LAN — OR on-LAN but with a **Tailscale subnet route hijacking 10.77.x
through DERP** — can't reach them → **black preview + reconnect spiral** (NOT the #500 cover bug;
that was the gray overlay). Proven by prod RTCP: affected client lost 519–4005 pkts vs a wired
client's 11. Fix = Cloudflare Realtime TURN relay.

- **Server** reads `PRESENTER_TURN_KEY_ID` + `PRESENTER_TURN_KEY_API_TOKEN` (unset → TURN off,
  LAN-only, unchanged). `crates/presenter-server/src/turn.rs` mints short-lived ICE creds from the
  Cloudflare key (12h cache, 10s mint timeout, 60s failure-throttle, stale-but-valid on error),
  exposed at **`GET /ndi/ice-servers`**.
- **Browser** (`ndi_video.rs`): fetches `/ndi/ice-servers` once/page (re-fetch >6h, before the 24h
  cred TTL), sets them on the `RtcPeerConnection`. `iceTransportPolicy` stays DEFAULT (all) — direct
  wins on LAN (no added latency), relay is the fallback.
- **Server `webrtcbin`** (`consumers.rs`) also gets `turn-server` so BOTH sides have a relay candidate.
- **Secrets:** GitHub Actions secrets `TURN_KEY_ID`/`TURN_KEY_API_TOKEN`; deploy writes a **0600
  root-only EnvironmentFile** `/etc/presenter/turn.env` (NOT a drop-in `Environment=`, which
  `systemctl show` exposes); unit has `EnvironmentFile=-/etc/presenter/turn.env`. NEVER commit values.
  Full setup + the Cloudflare token gotchas (API needs **Calls Read/Write**, NOT "Realtime Admin";
  product must be **activated via dashboard** first) are in local memory `project_cloudflare_turn.md`.

### Verify TURN works (relay-only probe — the definitive check)
On LAN the direct path wins, so TURN is never exercised by a normal load. To PROVE the relay path
carries video, force relay-only with REAL chrome (bundled Chromium has no H264):
```js
const ice = await (await fetch(origin+'/ndi/ice-servers')).json();
const pc = new RTCPeerConnection({ iceServers: ice, iceTransportPolicy: 'relay' }); // RELAY ONLY
// addTransceiver('video',{direction:'recvonly'}) → offer → ICE-gather → POST /ndi/whep/<src> → setRemote
// then getStats(): nominated candidate-pair's localCandidate.candidateType MUST be 'relay',
// inbound-rtp framesDecoded>0, and a canvas pixel-variance of the <video> >~500 (real pixels, not black).
```
Verified 2026-06-29 on prod: `selectedLocalCandidateType=relay`, 256 frames, variance 3875.
(Full script: scratchpad `relay_probe.mjs`.)

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

### Surfacing a per-frame signal to a StageContext UI signal (#479, #500)

To drive any UI state from "is video actually presenting / what is its rVFC metadata", REUSE the
setter-threading pattern — do NOT re-derive it:

1. Add an `RwSignal<…>` to `StageContext` (owned by `StagePage`, so it survives `NdiVideo`
   mount/unmount; safe to clear from `on_cleanup`).
2. In `NdiVideo`, build a `Rc<dyn Fn(…)>` setter from that signal (`VideoLatencySetter` /
   `FramesLiveSetter` in `ndi_frame_stats.rs`) and thread it through `Watchdog::install` into
   `start_rvfc_frame_observer` (per presented frame) AND, if it must also react to STALLS,
   `start_health_ticker` (the 1s tick is the ONLY place that can mark "no longer flowing", and the
   `approximate_frame_from_current_time` proxy is the rVFC-less browsers' frame signal — wire BOTH).
3. Transition-guard the reactive write with a per-session `Cell` on `FrameStats` (rVFC fires ~30×/s
   — write the signal only on the false→true / true→false edge, never every frame).
4. `Watchdog::install` already has `#[allow(clippy::too_many_arguments)]`; `start_rvfc_frame_observer`
   needs it too once you add a setter (it hits 8 args). Clone the setter to share it across the rVFC
   observer + the ticker (`Option<Rc<…>>` is Clone).
5. Reset the signal on `NdiVideo` `on_cleanup` AND in `pages/stage.rs` on `NdiSourceDeactivated` /
   `NdiSourceActivated` / `sync_ndi_source_state` (no-source + changed-source) so it never carries a
   stale value across a source change.

**#500 cover gate:** the ndi-fullscreen neutral covering placeholder (`stage-ndi__placeholder--cover`)
is gated on `should_show_neutral_cover(ndi_active, status, frames_live)` = `ndi_active &&
ndi_overlay_kind(status)==Neutral && !frames_live`. ONLY `ndi_fullscreen.rs` has this cover —
`api_stage.rs` / `timer_layout.rs` draw NDI as a BACKGROUND and render NOTHING for a neutral state
(only their red Error overlay), so there is no cover to gate there. The Error overlay is a separate,
unchanged gate (a failed source has no frames → errors still show).

### Deterministic stage-NDI E2E on the GH-hosted (no-NDI/GPU) lane

The GitHub-hosted `e2e` lane has NO NDI SDK, so `POST /integrations/video-sources` +
`…/{id}/activate` SUCCEEDS without starting a pipeline and the client holds the neutral
`connecting`/`no-signal` state — exactly the late-join / not-producing UI state, with NO live source
needed (the `ndi-webrtc.spec.ts` #448 cover test + `stage-ndi-frames-live-cover.spec.ts` #500 test
both rely on this). To drive a WASM-internal signal a server WS event can't reach deterministically,
expose a `__presenterStageSet*` global in `pages/stage.rs` (mirror `__presenterStageSetVideoLatency`
/ `__presenterStageSetNdiFramesLive`) and call it from the spec — these globals are always compiled
(not feature-gated) and never called in production. Allow-list the expected 503/204 WHEP-backoff
console lines (TIGHT regexes) so console-zero still catches real errors.
