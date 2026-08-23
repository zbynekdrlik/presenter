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

### Exercising a REAL Ok(Connected(_)) WHEP Session With No NDI Hardware (#670)

The default GitHub-hosted `e2e` lane has no NDI SDK, so `/ndi/whep/<id>` always answers 503/204
and `connect_whep()` (`ndi_video.rs`) never reaches `Ok(ConnectOutcome::Connected(_))` for real —
most NDI tests on this lane either stay at the "not-producing placeholder" level or `test.skip()`
themselves (`ndi-webrtc-recovery.spec.ts`). To test logic that only runs on a genuine Connected
session (e.g. per-reconnect state, teardown/dispose paths) WITHOUT needing the `@synthetic-ndi`
self-hosted GPU lane, mock the WHEP handshake at the network layer with `page.route()` instead of
faking data at the DOM layer:

1. Intercept the WHEP POST and, INSIDE `page.evaluate`, create an ephemeral `RTCPeerConnection` in
   the page, `setRemoteDescription` it with the real offer SDP the client posted, `createAnswer()` +
   `setLocalDescription()`, and hand that REAL Chrome-negotiated answer back as the fulfilled
   201 body (+ a fake `Location` header). Because this "fake server" pc runs in the SAME browser as
   the client, it always speaks a codec/payload-type set the client can actually parse —
   hand-crafting SDP text by hand is unnecessary and fragile by comparison. `setRemoteDescription`
   on the client then genuinely succeeds and `Ok(Connected(_))` fires for real, with ZERO actual
   media flow (no track ever attached server-side) — fine for testing session lifecycle, useless
   for testing decode/frames.
2. To force a DETERMINISTIC reconnect (never wait on real ICE-failure timing — unbounded and
   flaky), patch `RTCPeerConnection.prototype.oniceconnectionstatechange`'s setter (via
   `page.addInitScript`, walking the prototype chain for the descriptor defensively) to capture
   every `pc` Rust assigns a handler to, into a global array. Then, on demand,
   `Object.defineProperty(pc, "iceConnectionState", { get: () => "failed" })` on the target instance
   and `pc.dispatchEvent(new Event("iceconnectionstatechange"))` — this invokes the SAME real
   listener `install_ice_failure_listener` (`ndi_watchdog.rs`) registered, exactly as a genuine ICE
   failure would, on your own schedule.
3. No `@video-codec`/`@synthetic-ndi` tag needed — since no real media/H264 decode ever happens,
   this runs fine on the default `chromium` project/lane.

Full worked example (net add/remove-listener-count regression test, reused the #637
`EventTarget.prototype` instrumentation shape): `tests/e2e/stage-ndi-pagehide-teardown.spec.ts`.

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

### Swapping what a readout DISPLAYS while keeping the underlying signal alive (#532)

When a ticket says "replace the on-screen X with Y, but keep X's plumbing" (X = a signal other code
might still rely on, e.g. `dropped_frames` feeding server-side per-TV telemetry): DO exactly that —
keep the OLD `RwSignal`/setter/beacon-population code fully intact and just stop READING it in the
render closure (`status_bar.rs`'s `video_latency_text` switched from `ctx.dropped_frames` to the new
`ctx.stage_health`). Don't delete the old signal "because nothing renders it now" — a review pass
will flag it as write-only, but that's the CORRECT shape when the issue explicitly asks for it; note
the decision in the PR/commit rather than reverting under review pressure.

**Beacon-classification pattern:** if the new signal can be computed from data the beacon ALREADY
gathers synchronously (e.g. `snapshot_present_gaps`'s `(max_gap, over100, fps)`, no getStats/network
round-trip needed), classify it in `post_stats_beacon` BEFORE the `spawn_local` async block — it's
both simpler and lands independent of getStats succeeding.

**Multi-source-fps gotcha:** `stage_health`-style classifiers keyed on presented-fps must account for
BOTH beacon-trigger paths: (a) the two ~15s-cadenced rVFC-count and 1s-ticker paths share one
`FrameStats` and drift into near-coincidence, leaving one a sub-second window (`presented_fps_for_
window`'s `MIN_FPS_INTERVAL_MS` floor fixes this — always floor a computed rate below ~1s); (b)
rVFC-LESS browsers' `approximate_frame_from_current_time` proxy counts ≤1 "frame"/tick — a
stall-detection signal, NEVER a real rate — so it always computes ≈1fps regardless of real
smoothness; gate the classified-signal setter itself on `rvfc_supported` (see
`health_setter_for_ticker_beacon` in `ndi_health_ticker.rs`) rather than trying to special-case the
classifier's thresholds for that path.

**Verifying a new per-TV signal on PRODUCTION with a genuinely live stream:** the deterministic
`__presenterStageSet*` test hooks are LOCAL signal overrides with no protection against being
overwritten by the real beacon — on an ACTUALLY STREAMING production source, the live telemetry
keeps firing every ~15s and will race with (and usually win against) a manual test-hook call within
a couple of round-trips. Don't fight this or treat it as a bug. Instead, read the REAL computed value
as the verification evidence (e.g. `server→displej · 22 ms · 🟢 plynulé · 30 fps` sourced from the
actual live NDI stream) — that's actually STRONGER proof than a synthetic override, since it exercises
the entire real pipeline end-to-end. Use `/stage?preview=1` (the operator-preview mirror mode) to open
a read-only tab that never grabs a wake lock or changes the broadcast layout for real displays.

## Encoder selection — the two lies (#443, #541)

`hw_h264_encoder()` picks the H264 encoder for the shared-encoder pipeline. Two hard-won rules:

1. **Registry presence lies** (#443): an element can be advertised and fail to instantiate.
2. **Instantiation lies too** (#541): NVIDIA driver 595.71.05 (dev2, 2026-07-03) dropped the LEGACY
   nvenc preset API — `nvh264enc` still registers AND still constructs, then dies at caps negotiation
   with *"Selected preset not supported"*, surfacing only as an opaque
   `Could not configure supporting library` at pipeline start.

So the probe pushes ONE real frame through `videotestsrc ! <encoder> ! fakesink` and requires EOS.
Verdicts are cached per element per process (a driver rejection cannot heal without a driver change,
and a live probe would open a second NVENC session mid-stream — consumer GeForce caps those).

Candidate order: `vah264enc` (prod N100) → `nvcudah264enc` → `nvautogpuh264enc` → `nvh264enc` (legacy,
older drivers only) → `x264enc`. The modern nvcodec elements ignore the legacy `zerolatency` boolean —
they need `tune=ultra-low-latency` + `zero-reorder-delay=true` + `rate-control=cbr`, or you silently
ship a B-frame (reordering) encoder to the stage TVs.

**Triage recipe when e2e-ndi starts failing with an encoder error and the #445 GPU preflight PASSES:**

```bash
# 1. Is it the GPU, or the encoder element?
nvidia-smi --query-gpu=utilization.gpu,memory.used --format=csv,noheader   # wedge = 100% + no process
# 2. Can each encoder actually encode RIGHT NOW?
for e in vah264enc nvh264enc nvcudah264enc nvautogpuh264enc x264enc; do
  gst-inspect-1.0 --exists $e && gst-launch-1.0 -q videotestsrc num-buffers=5 \
    ! video/x-raw,width=640,height=360 ! videoconvert ! $e ! h264parse ! fakesink 2>&1 | head -2
done
# 3. Did a driver land after the last green run?  (this is what #541 turned out to be)
grep -h nvidia /var/log/dpkg.log* | grep -E ' install | upgrade ' | tail -5
gh run list --branch dev --workflow=pipeline.yml --limit 12 --json createdAt,conclusion
```

## GPU access on a deployed host (#540)

The service runs as an ordinary user; `/dev/dri/renderD128` is `root:render 0660`. Without
`SupplementaryGroups=render` in the unit the process cannot open it, the GStreamer `va` plugin
registers **zero** elements, and NDI dies silently with every driver package installed. Do NOT rely on
the systemd-logind console ACL (`getfacl /dev/dri/renderD128` showing `user:<u>:rw-`) — it exists only
while somebody is logged in at the physical console, and it is what made prod *look* healthy while PP
was dead. Check with `gst-inspect-1.0 va | grep -c '^  va'` (0 = no access).

## "No video on the stage" — triage in the order the layers actually fail (#544, #546)

Four independent layers must ALL hold, and each fails silently. Walk them in THIS order — the PP
outage cost hours because the investigation kept re-checking layer 3 while layer 1 was the cause,
and then declared victory at layer 3 while layer 4 was the real remaining blocker.

```bash
H=companion-pp.lan   # or presenter.lan / 10.77.8.134:8080

# 1. Can the service reach the GPU?  (#540 — zero features = no render-group access)
ssh $H "sudo -u newlevel GST_REGISTRY=/opt/presenter/gstreamer-registry.bin gst-inspect-1.0 va | grep -c '^  va'"

# 2. Did an encoder actually get SELECTED at startup?  (#541 / #544)
ssh $H "sudo journalctl -u presenter -b --no-pager | grep -E 'encoder-gate|WebRTC encoder|no hardware'"
#    "encoder-gate: vah264enc registered" + "NDI WebRTC encoder: vah264enc" = layers 1+2 green.

# 3. Is the NDI source the operator mapped ACTUALLY BROADCASTING?  ← the one that is easy to miss
curl -s http://$H/ndi/sources                      # what is on the network RIGHT NOW
curl -s http://$H/integrations/video-sources       # what is mapped
curl -s http://$H/healthz | jq .ndi_pipelines      # [] = nothing producing; [{state:"streaming"}] = live
#    Mapped name absent from /ndi/sources → the SENDING machine is off/renamed. Not our bug, and no
#    amount of server-side debugging will fix it. The log says so:
#    "NDI source activated but not yet producing — broadcaster silent (#448)".
#    The UI does not surface this yet — that gap is #546.

# 4. Does the BROWSER decode it?  (Chromium ≠ Chrome — see below)
```

**A "pipeline built" log line does NOT mean video is flowing.** The pipeline is (re)built every ~30 s
by the auto-reconnect loop even when the NDI source is silent. Proof of flow is
`healthz.ndi_pipelines[].state == "streaming"`, nothing less.

### `gst-inspect --exists` answers from the registry CACHE — it is not a plugin scan (#547)

This burned a whole class of "fix" that could never have worked, so keep it in mind for anything
that polls GStreamer:

- `gst-inspect-1.0 --exists <element>` is a **lookup in the cached registry file** (`$GST_REGISTRY`),
  not a plugin load. Measured on dev2: **14 ms warm vs 916 ms cold** (only the cold run emits
  `vaInitialize`). So a poll loop built on it performs ONE real scan and then re-reads that same
  verdict — it can **never observe a late-registering encoder**, which is the entire reason a
  boot-time gate exists.
- `GST_REGISTRY_UPDATE=yes` does **not** save you: it only rescans plugins whose **file** changed.
  A permission change (#540) or a driver change leaves the plugin file untouched, so a cached
  "va has 0 features" verdict survives it forever (that IS #544).
- **The only way to force a real rescan is to DELETE the registry file** — which is what the
  encoder gate now does before every poll, and what every deploy does before starting the service.
- A name lookup also answers a different question than the server asks. Since #541 the server only
  trusts an encoder after a **real one-frame encode** (`videotestsrc num-buffers=1 ! … ! <enc> !
  fakesink`). Probe the same way, or the gate green-lights an encoder the server then rejects.
- Bound every gst call with `timeout`. A wedged driver (#445) makes one hang forever; inside an
  `ExecStartPre` that hangs the unit past `TimeoutStartSec` and **fails the whole service** — the
  `-` prefix only ignores a non-zero exit, not a start timeout.

### Probing WHEP yourself: use `channel: "chrome"`, never bundled Chromium

Playwright's bundled Chromium has **no H264** — the server correctly rejects its offer with
`WHEP offer carries no H264 rtpmap — rejecting consumer`, which reads like a server fault and is not.
Launch real Chrome (same as `playwright.config.ts` does) and read the decoded-frame count:

```js
const browser = await chromium.launch({ channel: 'chrome', args: ['--autoplay-policy=no-user-gesture-required'] });
// … page.goto('http://<host>/stage'); wait ~15 s
const v = document.querySelector('video');          // in page.evaluate
v.videoWidth /* 1280 */, v.getVideoPlaybackQuality().totalVideoFrames /* > 0 */
```

To verify the chain WITHOUT a live broadcaster (e.g. to prove a build before an event), publish the
synthetic source and map it — this is exactly what the `e2e-ndi` lane does:

```bash
NDI_RUNTIME_DIR_V6=/usr/lib/ndi PRESENTER_NDI_TEST_NAME=PRESENTER-TEST \
  cargo run -p presenter-ndi --features test-helpers --bin ndi_test_sender &
# it appears as "DEV2 (PRESENTER-TEST)"; POST it to /integrations/video-sources, then
# POST /integrations/video-sources/<id>/activate   ← activation is a POST to /activate, NOT a PATCH
```

Clean up afterwards (`/deactivate`, `DELETE` the source, pid-targeted `kill` of the sender — never
`pkill -f`, it matches your own shell).

---

## "No pipeline in the snapshot map" is NOT "the source is silent" (#546)

`NdiManager::pipeline_snapshots()` gives up on the `active` mutex after **200 ms** and returns an
**empty vec** — while `start_pipeline` HOLDS that same mutex across its **8 s caps-wait**. So during
EVERY normal activation the map reads as empty. Any consumer that treats "no entry" as a fact about
the SOURCE (rather than about our ability to LOOK) will report a perfectly healthy activation as
"broadcaster silent" for those 8 seconds.

Use **`pipeline_snapshots_checked() -> Option<Vec<..>>`** when the answer is shown to a human:
`None` = the lock timed out (the manager is busy, almost always starting a pipeline) → say
*Connecting*; `Some(vec![])` = we looked and there really is nothing → the silent-broadcaster case
(#448). `pipeline_snapshots()` (the `unwrap_or_default()` wrapper) is fine for `/healthz`, which only
wants a best-effort list and must never stall.

Same shape one level up: a **discovery failure** (`discover_sources` errors, or the finder thread
never came up because `NDIlib_find_create_v2` returned null) is NOT an empty network. Degrading it to
"nothing is on the air" makes a broken server tell the operator that every sending machine at the
site is off. Model the two apart (`Discovery::Blind` vs `Discovery::Names(..)`) and say *NDI
unavailable*, never *not found on the network*, when blind. The classifier that encodes all of this
is `presenter-server/src/state/video_source_status.rs` — pure, unit-tested, and the rule ORDER is the
part that lies to the operator when it is wrong.

**E2E consequence:** never hard-assert `ndiAvailable === false` in a default-lane spec. The GitHub
runners have no libndi, **dev2 does** — and the same suite is run on dev2 before a merge. A spec that
is green only on one host is not a guard. Branch on what the server reports about itself
(`tests/e2e/video-source-status.spec.ts`), and assert something real in both branches.

**`discover_sources()` CANNOT fail — do not model blindness as an `Err`.** It is
`Ok(self.source_list.read())` (lifecycle.rs). A "discovery failed" branch keyed on `Err` is
therefore dead code in production and green in tests only because the fake fabricates an
impossible error — that exact mistake shipped once. The blindness the server really has is
**an empty list from a finder that never looked**: `NDIlib_find_create_v2` returned null (the
finder thread exits and the list stays empty forever) or it simply has not completed its first
~5 s scan yet (every restart). Ask `NdiManager::discovery_snapshot() -> Option<Vec<..>>`, which
is `None` until the finder has published a scan.

## Testing the `<video>` play/pause lifecycle without a live NDI source (#568)

To E2E-test playback-recovery logic (a pause/ended/suspend listener, an autoplay retry) with NO
NDI SDK/GPU needed: activate a not-producing bogus source (mounts `<video data-role="ndi-video">`
with no real stream, per the "Deterministic stage-NDI E2E" pattern above), then bypass WHEP
entirely by assigning a synthetic `canvas.captureStream(fps)` MediaStream directly onto that
`<video>` element's `srcObject` from `page.evaluate` — repaint the canvas on an interval so the
captured track has real frame changes. This produces a genuinely playing/pausable element your
guard code reacts to exactly like a real stream, on any runner (`stage-ndi-playback-guard.spec.ts`).

**A fast-reacting guard makes "confirm it paused" itself flaky — assert the EVENT, not the
boolean.** If your fix makes the element recover from `pause()` near-instantly, a
`expect.poll(() => video.paused).toBe(true)` sanity check can miss the brief paused window
entirely (the poll only ever samples `false`). Attach a one-shot `pause` event listener that
flips a flag BEFORE calling `.pause()`, and poll that flag instead — a discrete fact, not a
racing sample of a fast-changing state.

**Verify a `-webkit-media-controls*` CSS suppression rule via CSSOM, not `getComputedStyle`.**
`getComputedStyle(el, '::-webkit-media-controls-overlay-play-button')` resolves inconsistently
across headless Chromium builds for internal UA pseudo-elements. Instead, walk
`document.styleSheets` → `CSSStyleRule`s whose `selectorText` contains `media-controls`, strip the
`::-webkit-media-controls*` suffix from each comma-separated selector, and check
`element.matches(strippedSelector)` + `rule.style.getPropertyValue('display') === 'none'`. This
deterministically proves the rule's SELECTOR reaches the element — which is what #568 actually
broke (the rule existed, it just didn't target two of the three stage layouts' video elements).

## The grey play-arrow is CSS-UNREACHABLE on Chrome ≥150 — hide the `<video>`, not its pseudos (#732)

**Never rely on `::-webkit-media-controls*` rules to suppress the grey play-arrow on the real stage
TV WebViews.** #448/#478/#568 all tried this and all failed on the field (4 recurrences). Proven
live on the real stage TV **SD1** (Tesla/Skyworth **LEAP-S1**, Android 12, **Chrome/150 WebView** —
NOT an old vendor Chromium) via adb screencap + CDP:

- The visible arrow is `-internal-media-controls-overlay-play-button-internal` — a Chrome ≥150
  **UA-INTERNAL** pseudo-element. **Author CSS cannot select any `-internal-*` pseudo**, so the
  #478/#568 `[data-role="ndi-video"]::-webkit-media-controls-overlay-play-button{display:none}` rule
  (which DOES compute `display:none` on the field engine — verified) only hides the `-webkit-`-named
  wrapper and never reaches the `-internal-…-internal` glyph that actually paints. Injecting
  `::-webkit-media-controls-overlay-enclosure{display:none}` left the `-internal` button
  `display:block` and the arrow unmoved — decisive.
- It paints INSIDE an **empty/frameless** `<video data-role="ndi-video">` (no `srcObject`,
  `readyState 0`, `paused`). Reproduced states: the coverless **timer / api** layouts (broadcast off
  → arrow for hours), every layout's **cold-open WHEP window** (t+2..t+5s during negotiation —
  manufactured on every watchdog cold-reload, #734), and stalls. ndi-fullscreen hid it only because
  its "Connecting…" cover happened to sit over the frameless video — which is exactly why every
  emulator/desktop probe (and #478's fullscreen-only CSS) missed it.

**THE INVARIANT: the NDI `<video>` element must be INVISIBLE whenever it is not delivering frames.**
`NdiVideo` (`ndi_video.rs`) toggles a `stage-ndi-video--dormant` class → `opacity:0` (stage.css)
whenever frames are not presenting, revealed only when real frames flow — proven live on SD1 that
`opacity:0` on the ELEMENT removes the arrow. It is gated on the existing **`ndi_frames_live`**
signal (the #500 rVFC-frame-observer signal — set true per presented frame, flipped false after
`FRAMES_LIVE_STALENESS_MS`=1.5s of no frames / on cleanup / on (de)activate), so there is no new
signal and no new timer, and a 1-frame hiccup never flickers the element. Keep the element MOUNTED
(`opacity`, never `display:none`/unmount) so WHEP negotiation + autoplay are unaffected. The
`::-webkit-media-controls*` block stays as a harmless belt but is NOT the fix.

**E2E it (no NDI/GPU needed):** activate a not-producing bogus source (mounts the coverless
`<video>`), then drive the frames-live flag with the `__presenterStageSetNdiFramesLive` test hook
(the SAME signal the rVFC observer writes) — dormant with no frames → visible on `true` → dormant on
`false`. Assert via the **class attribute + `getComputedStyle(el).opacity`**, never Playwright
`toBeVisible()` (an `opacity:0` element still reports visible — see `.claude/skills/ui/SKILL.md`).
`stage-ndi-hidden-until-frames.spec.ts` covers all three layouts.
