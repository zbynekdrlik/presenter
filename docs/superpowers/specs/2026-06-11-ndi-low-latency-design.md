# NDI → Stage Display Low-Latency Design

**Date:** 2026-06-11
**Status:** Approved (approach A — full low-latency package, single PR)
**Goal:** Glass-to-glass NDI video latency on stage displays comparable to or better
than VDO.Ninja (lip-sync with live room audio), stable over hours — no growth, no
periodic jumps, no choppiness on the low-cost Android TVs.

## Background — measured state (2026-06-11)

All numbers measured directly on the running systems (clock-strip pixels baked into
a synthetic NDI source, decoded from screenshots/canvas; frame-matching between
clients; WebRTC getStats):

| Measurement | Result |
|---|---|
| sd1 TV absolute glass-to-glass (prod path: cross-VLAN NDI → vah264enc N100 → WHEP → WebView) | **38–94 ms** (clock content) |
| Laptop client on prod stream, browser-side hold | 10–60 ms, 30 fps, 0 loss |
| TV vs laptop frame-match | in sync (Δ ≈ 0) |
| Dev g2g with high-motion content (NVENC) | median ~206 ms, p95 ~234 ms |
| A/B producer `sync=false` vs `sync=true` | ~−40 ms |

Conclusion: the pipeline is already capable of sub-300 ms. The user-visible failure
("choppy + delayed, worse over time") is **degradation under time/load**, driven by
five identified mechanisms (below). The fix hardens the pipeline against all five,
adds observability so future latency complaints are diagnosable from data, and adds
a CI latency guard.

## Root-cause mechanisms addressed

1. **Sender-clock coupling.** `ndisrc` default `timestamp-mode=auto` derives PTS
   from the NDI sender's (Resolume's) timecode with windowed drift correction;
   accumulated drift is corrected via DISCONT — visible as "lag builds, then
   jumps". → Use pure server receive time.
2. **Keyframe pulses.** `gop-size 30` (1 s) emits a large IDR burst every second;
   weak TV decoders take longer on the burst, the adaptive jitter buffer inflates —
   "choppy". → Long GOP + explicit force-keyunit when a consumer joins.
3. **Producer sync hold.** `StreamProducer::from()` defaults to `sync=true`: the
   appsink holds every encoded frame until its clock deadline before fanout
   (~40 ms measured). A relay should forward immediately. → `sync=false`.
4. **Unbounded client jitter buffer.** The stage UI never hints the receiver;
   Chrome/WebView grows the jitter buffer adaptively and is slow to shrink it on
   low-end TVs. → `jitterBufferTarget = 0` (+ legacy `playoutDelayHint = 0`).
5. **Payloader aggregation.** `rtph264pay` default `aggregate-mode=none`;
   `webrtcsink` (reference implementation) sets `zero-latency`. → Parity.
6. **H264 High profile breaks strict TV decoders (found live 2026-06-11).**
   The encoder emits High profile (SPS profile_idc=100 read from the live prod
   stream). Desktop Chrome and sd1's lenient decoder play it, but the
   sd2/sd3/sd4 Vestel TVs (1 GB RAM, WebView 148, HW-only H264 for WebRTC)
   reject it — logcat shows `NullVideoDecoder doesn't support decoding`, the
   stage shows black/one frame while the server pushes RTP fine
   (`buffers_pushed` 300–4000 per churned session), and the client watchdog
   reconnects every 10–30 s forever. WebRTC's mandatory baseline is
   Constrained Baseline; strict HW decoders accept only what they offered.
   → Pin the encoder output to `constrained-baseline` via a capsfilter
   between encoder and h264parse (all three encoder paths). ~10-15 % bitrate
   efficiency loss at 2.5 Mbps 720p30 — visually irrelevant, compatibility
   mandatory.

## Changes

### 1. Server pipeline — `crates/presenter-ndi/src/pipeline/build.rs`

- `ndisrc`: add `.property_from_str("timestamp-mode", "receive-time")`.
  Pure server arrival time; no sender-clock observations, no drift DISCONTs.
  (Arrival jitter passes into PTS; the browser jitter buffer absorbs it — measured
  10–60 ms.)
- `StreamProducer::with(&appsink, ProducerSettings { sync: false })` instead of
  `StreamProducer::from(&appsink)` (path: `gstreamer_utils::streamproducer::ProducerSettings`).
- appsink `max_buffers(30)` → `max_buffers(5)` (still `drop(true)`): bounds the
  transient backlog a momentary fanout stall can replay late.
- Encoder GOP 30 → 240 frames (8 s) on all three encoders:
  `vah264enc key-int-max=240`, `nvh264enc gop-size=240`, `x264enc key-int-max=240`.
  Join latency is covered by force-keyunit (below); loss recovery stays PLI-driven
  (browser PLI → webrtcbin → StreamProducer forwards force-keyunit upstream —
  existing behavior).
- `vah264enc`: add `target-usage=6` (faster encode on the prod N100; default 4).
  `nvh264enc` already has `zerolatency=true`; `x264enc` already `tune=zerolatency`.
- `h264parse config-interval=-1` stays (SPS/PPS before every IDR).

### 2. Consumer join — `crates/presenter-ndi/src/pipeline/consumers.rs`

- `rtph264pay`: add `.property_from_str("aggregate-mode", "zero-latency")`.
- In `add_consumer` (after `producer.add_consumer(&appsrc)`): push a
  `gst_video::UpstreamForceKeyUnitEvent` (all-headers=true) into the encoder
  pipeline (upstream from the producer appsink's sink pad) so the new consumer
  gets an IDR immediately instead of waiting for the GOP boundary. With GOP=240
  this is REQUIRED for join; it also makes joins faster than today (≤1 s → ~0 s).

### 3. Stage UI client — `crates/presenter-ui/src/components/stage/ndi_video.rs`

- In the `ontrack` handler: on the video receiver set
  `jitterBufferTarget = 0` and `playoutDelayHint = 0` (via `js_sys::Reflect` —
  `web_sys` has no binding). Keeps the WebView's jitter buffer at its minimum and
  forces it to shrink back after transient spikes.

### 4. Observability

- **Server:** new `GET /ndi/whep-stats` endpoint: per pipeline (source_id), per
  session: `session_id`, `connection_state`, `pushed`/`dropped` (from
  `ConsumptionLink` counters), and RTCP receiver-report stats from
  `webrtcbin` `get-stats` (round-trip-time, jitter, packets-lost) — the stage
  display's own view of the link, readable server-side.
- **Client beacon:** stage UI samples `getStats` every 15 s and POSTs
  `{sourceId, framesDecoded, fps, jitterBufferMs, freezeCount, framesDropped}` to
  `POST /ndi/client-stats`; server logs it via `tracing::info!` (journald persists;
  no DB table — MVP, ratchet later if needed).

### 5. CI latency guard (TDD)

- **Unit tests (RED first, GREEN after):** assert the constructed pipeline's
  element properties — producer sync=false, ndisrc timestamp-mode=receive-time,
  encoder GOP=240, appsink max-buffers=5, payloader aggregate-mode=zero-latency.
  These are deterministic RED→GREEN regression locks for every knob.
- **Clock strip in the synthetic sender:** `ndi_test_sender` paints a 26-block
  strip into every frame (block 0 white + block 1 black for threshold calibration,
  blocks 2–25 = 24-bit big-endian `unix_millis % 2^24`; 48 px blocks at
  (48,48) @ 2560×1440 — survives the 720p downscale and 2.5 Mbps encode; verified
  today). Existing e2e tests are unaffected (strip is just pixels).
- **New Playwright e2e (`tests/e2e/ndi-latency.spec.ts`):** connects WHEP to the
  synthetic source, draws the decoded video to a canvas per
  `requestVideoFrameCallback`, decodes the strip, computes per-frame
  `g2g = (Date.now() − embedded) mod 2^24` (sender and browser on the same CI
  machine → same clock). Over ≥300 frames asserts:
  - median g2g ≤ 350 ms, p95 ≤ 600 ms (tolerant of the shared self-hosted runner;
    quiet-machine reality after the fix is expected ~120–160 ms median),
  - `totalFreezesDuration` < 1 s,
  - zero console errors/warnings (per browser-console-zero-errors rule).

### 6. Out of scope (tracked separately)

- Prod host NTP skew (+26.5 s): filed as **#377: Prod server clock is ~26.5s ahead —
  NTP/chrony sync broken on 10.77.9.205**.
- Settings-audit spam / supervisor rebuild loop: already tracked as **#375**.

## Acceptance criteria (definition of done)

1. CI green including the new latency e2e and unit property tests (RED commits
   precede GREEN commits in history).
2. Quiet-machine dev measurement: median g2g ≤ 160 ms with the clock-strip source
   (reported in the PR with actual numbers).
3. 30–60 min soak on dev (clock-strip content): latency trend FLAT — no monotonic
   growth, no DISCONT jumps, jitterBufferTarget honored (browser jb stays low).
4. After prod deploy (separate user approval): 2-minute clock-source swap on prod +
   sd1 screencap series → absolute TV g2g ≤ 250 ms; then prod source restored.
5. Existing e2e guards (burst #372, straggler #373) stay green — the three
   load-bearing invariants of the per-consumer architecture are untouched.

## Risks & mitigations

- **Long GOP + force-keyunit join path regression** → covered by existing burst +
  straggler e2e (they join mid-stream; with GOP=240 they only pass if
  force-keyunit works — the e2e lane itself becomes the guard).
- **receive-time PTS jitter** → browser jitter buffer absorbs (measured headroom);
  soak test verifies no freeze regression.
- **`jitterBufferTarget` unsupported on older WebViews** → property set via
  Reflect is a no-op where unsupported; sd1–sd4 run WebView 148 (supported).
- **Latency e2e flakiness on loaded runner** → generous bounds (350/600 ms vs
  ~150 ms real), median/p95 statistics over ≥300 frames, no single-frame asserts.
