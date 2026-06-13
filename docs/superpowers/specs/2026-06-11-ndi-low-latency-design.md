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

- **Server:** the existing `GET /ndi/snapshot/{source_id}` endpoint is
  extended (no new route): per session it now also reports
  `buffersPushed`/`buffersDropped` (from `ConsumptionLink` counters) and RTCP
  receiver-report stats from `webrtcbin` `get-stats` (round-trip-time,
  jitter, packets-lost) — the stage display's own view of the link, readable
  server-side.
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

## Addendum 2 (2026-06-11 late evening): VP8 fallback for broken TV H264 OMX

**Measured root cause on the Vestel displays (live logcat, sd2, valid adb):** the
MStar HW decoder (`OMX.MS.AVC.Decoder`) decodes the first frame of each GOP,
then the OMX port reconfiguration (640×480 default → 1280×720) fails —
`setParameter(ParamPortDefinition) ERROR: BadParameter`, ACodec cannot set
`nBufferCountActual` (7→6→5 all rejected), `MS_OMX_OutputBufferProcess` errors,
codec torn down and recreated every ~8 s (= our GOP). Result: ~1 displayed
frame per GOP — "a frame every few tens of seconds". This is a vendor OMX
firmware bug; H264 WebRTC cannot work on this platform. VDO.Ninja worked on
the same TVs because it negotiates VP8 by default — WebView decodes VP8 in
software (libvpx), bypassing the broken vendor OMX entirely.

**Fix: dual-codec fanout + client-side codec fallback.**

- **Server:** `tee` after the scale capsfilter; branch A unchanged (H264);
  branch B `queue → vp8enc (deadline=1, cpu-used=8, keyframe-max-dist=240,
  target-bitrate=2_000_000) → appsink(enc_appsink_vp8, sync=false, 5 buffers)`
  wrapped in a second `StreamProducer`. Verified: prod N100 runs this VP8
  realtime config (gst-launch exit 0); vp8enc/rtpvp8pay present on both hosts.
- **Codec selection rule (deterministic, zero change for healthy clients):**
  the server prefers H264 whenever the offer contains it (today's behavior —
  Chrome's default offer lists VP8 first but always includes H264, verified
  live). VP8 is served ONLY when the offer carries NO H264 — which is exactly
  what the fallback client produces via `setCodecPreferences` (VP8+rtx only).
  The existing "offer carries no H264" warn-path becomes the VP8 path.
- **Consumer pipeline (VP8):** `appsrc(video/x-vp8) → rtpvp8pay → webrtcbin`
  on the encoder clock/base-time; same three load-bearing invariants; payload
  type aligned from the offer's VP8 rtpmap; force-keyunit goes upstream from
  the producer the consumer is attached to.
- **Client fallback (stage UI):** default behavior unchanged (offer includes
  H264 → server picks H264). A decode watchdog arms after connect: if
  `framesDecoded < 30` after 12 s connected, set `ndiCodecMode=vp8` in
  localStorage and reconnect with `setCodecPreferences` limited to VP8(+rtx).
  If a VP8 session also fails the same check, clear the flag (alternate back)
  — no permanent lock-in on either codec.
- **Beacon identity:** beacons gain `displayId` (persistent random id in
  localStorage), `codec` (from getStats codec mimeType) and screen size, so
  per-TV health is attributable server-side (tonight's diagnosis was slowed
  by anonymous beacons).
- **CI guard:** e2e (e2e-ndi lane): a consumer with VP8-only codec
  preferences must decode >0 frames from the synthetic source (locks the VP8
  path); existing H264 latency guard unchanged.

**Acceptance:** Vestel TVs (sd2-4) report sustained `fps≈30` in their beacons
with `codec=video/VP8` on the live stage, while sd1/laptops keep
`codec=video/H264`.

## Addendum 3 (2026-06-12): pivot — compat profile = 640×480 H264, VP8 removed

**Measured on prod with the Addendum-2 VP8-480p branch live:** the weak
Vestel TVs decoded VP8 in software at only ~26 fps WITH 37 freezes, and the
software `vp8enc` drove the prod N100 to load ~10 (hiccups for every
consumer). VP8 avoided the broken OMX decoder but traded it for two CPU
walls.

**Refined root cause (logcat, sd2):** the MStar OMX H264 decoder
(`OMX.MS.AVC.Decoder`) dies ONLY on output-port reconfiguration — it
default-inits its port at 640×480, and the 1280×720 stream forces a
reconfig that fails (`setParameter(ParamPortDefinition) BadParameter`,
codec torn down every GOP). Hypothesis now shipped: a stream that IS
EXACTLY 640×480 H264 needs no reconfig → HW decode on the TV (zero TV CPU)
+ GPU encode on the server (`vah264enc` at 900 kbps; near-zero N100 CPU vs
`vp8enc`).

**Changes (replace Addendum 2's mechanism, keep its goal):**

- Server: the VP8 branch is replaced by a second H264 branch —
  `raw_tee → q_compat(leaky) → videoscale → caps(NV12 640×480 PAR 1/1,
  "compat_scale_caps") → encoder_compat (same factory as primary, 900 kbps,
  GOP 240) → constrained-baseline caps → h264parse_compat →
  enc_appsink_compat (StreamProducer sync=false)`. No videoconvert (tee is
  already NV12). Exactly TWO encoders per source by design — one per
  PROFILE, never per consumer (#336 invariant updated).
- Selection: offer-based codec sniffing is gone (`ConsumerCodec`,
  `select_codec`, `rtpvp8pay` deleted). The WHEP POST query
  `?profile=compat` (parsed into `StreamProfile` at the HTTP layer) selects
  the producer; `request_keyframe` targets the selected branch.
- Client: the no-decode fallback no longer strips H264 via
  `setCodecPreferences` — it reconnects with `?profile=compat`. localStorage
  key `ndiCodecMode` is kept, values are now `default`/`compat` (legacy
  `vp8` parses as default and self-heals); same once-per-pageload switch and
  ≥10 fps proven-mode persistence. Beacons gain a `profile` field (codec now
  reads video/H264 everywhere).
- CI guard: the synthetic-lane VP8 e2e is replaced by "compat profile
  consumers get the 640x480 H264 stream" (frameWidth === 640, mimeType ===
  "video/H264").

**Acceptance:** Vestel TVs (sd2-4) report sustained `fps≈30` with
`profile=compat`, `codec=video/H264` in their beacons on the live stage —
hardware decode, no freezes — while sd1/laptops keep the default 720p
profile.

## Addendum 4 (2026-06-13): #387 dynamic adaptive compat controller

**Quality policy (binding, user directive):** priority = (1) near-zero latency,
(2) no stutter, (3) MAXIMUM quality/resolution/fps. Defaults start HIGH; the
controller lowers quality ONLY on measured degradation and raises it back when
headroom returns — both directions, VDO.Ninja-style. Floor values are never the
shipped steady state.

**Why a homegrown controller:** the upstream `rtpgccbwe` GCC element is NOT
installed on dev2 or prod (gst-plugins-rs `net/webrtc`), and installing it on
the prod box is a heavier infra lift than the fix warrants. webrtcsink ships a
"Homegrown" congestion controller (AIMD on RTCP stats) that needs no extra
element — we replicate its algorithm in our Rust over the per-session RTCP
remote-inbound stats we ALREADY read for `/ndi/snapshot`.

**Architecture change (compat profile only):**
- Today the compat branch is ONE shared `vp8enc` → `producer_compat` fanning
  encoded frames to all compat consumers — it can only ever serve the worst
  TV's bitrate to all of them. To adapt per-TV, the compat path moves to a
  RAW-video producer: `tee → q_compat → videoconvert → videoscale → caps
  (854×480 I420) → appsink(raw) → producer_compat_raw`. Each compat consumer
  pipeline becomes `appsrc(raw) → vp8enc(per-consumer) → rtpvp8pay →
  webrtcbin`, so every weak TV has its OWN encoder whose `target-bitrate` is
  driven independently.
- The DEFAULT (H264 720p) profile stays a SHARED encoder (the #336 invariant —
  per-consumer H264 720p encoders melt the N100). Per-consumer encoders are
  affordable ONLY for the compat tier: ≤3 weak TVs at VP8 480p.
- CPU budget: vp8enc 480p realtime ≈ small; 3 instances on the N100 alongside
  one vah264enc 720p is within budget (measured headroom: load dropped to ~2.5
  after removing the shared-720p-VP8 misstep).

**The controller (per compat consumer, runs in the consumer's bus-watch task or
a dedicated 1 s tokio interval):**
- Read webrtcbin `get-stats` → remote-inbound-rtp: `packets-lost` delta,
  `round-trip-time`, `jitter`. Compute EWMA loss `pl = 0.35·prev + 0.65·cur`
  (VDO.Ninja constant).
- AIMD on `vp8enc target-bitrate` (bits/s), bounds [200_000, 1_500_000],
  start at 900_000 (HIGH per policy):
  - loss > 1% (0.01): multiplicative decrease `bitrate *= 0.85`.
  - loss < 0.5% for ≥10 s AND rtt stable: additive increase
    `bitrate += 50_000` (probe up — the "raise when headroom returns" half).
  - clamp; apply via `encoder.set_property("target-bitrate", bps)` (live, no
    caps change → NO decoder reconfig).
- **RESOLUTION STAYS FIXED.** Changing encode resolution live renegotiates caps
  and triggers the decoder port-reconfig that KILLS these Vestels (addendum 2).
  So unlike VDO.Ninja we adapt BITRATE ONLY; resolution is a fixed tier chosen
  at connect (854×480 compat). A future ladder (480p↔360p) would be a full
  reconnect, not a live change.
- PLI/IDR rate-limit: ≤1 forced keyframe/s per encoder (a struggling TV
  PLI-spams; every IDR worsens its load — the collapse spiral). 

**Observability:** beacon already carries fps/freeze/jb; ADD server-side the
current per-consumer `target-bitrate` to `/ndi/snapshot` so the AIMD loop is
visible in the ledgers.

**Acceptance (ledger method, #379):** a Vestel that previously oscillated holds
a STABLE fps for ≥10 min with the controller settling its bitrate to the link's
real capacity (no resets, freezes trending to ~0), and visibly recovers bitrate
when the link improves. Default-profile sd1 stays 30 fps untouched.

**Depends on #388** (reaper v2 — the prod reaper is inert because gst webrtcbin
never flips connection-state for vanished peers; the same RTCP-liveness read
this controller adds is what #388 needs, so build them together).
