---
paths:
  - "crates/presenter-ndi/src/pipeline/build.rs"
  - "crates/presenter-ndi/src/pipeline/encoder_timing.rs"
  - "crates/presenter-ndi/src/pipeline/time_window.rs"
  - "crates/presenter-ndi/src/pipeline/link_probe.rs"
  - "crates/presenter-ndi/src/pipeline/tests/encoder_timestamps.rs"
  - "crates/presenter-ndi/src/manager/supervisor.rs"
  - "crates/presenter-ndi/src/manager/drift_guard.rs"
---

# Shared-encoder timestamp guard + drift self-heal (#854)

Both prod sites (SNV on an Intel N100 and PP) run GStreamer 1.24.2 `vah264enc`, which stamps its
OUTPUT with a frame counter (`start_pts + frame_duration × n`) and discards ndisrc's receive-time
PTS. A source faster than its declared rate makes that counter lead the clock (PP: +173 ms/h);
webrtcbin's per-stream `clocksync sync=TRUE` holds future buffers and every consumer's 500 ms
queue overflows once per GOP. The guard (`pipeline/encoder_timing.rs`, encoder `src` pad probe) is
the fix; the drift self-heal (`manager/drift_guard.rs`) is its backstop. Full story: the ndi
skill's #854 chapter.

## Invariants — keep them

- **No frame leaves the shared encoder stamped more than D = 50 ms later than the pipeline clock**
  (`TIMESTAMP_DEADBAND_MS`; `out <= now + D`, `out` strictly increasing). Only the part of a lead
  above D is pulled back. The correction only GROWS on a pulled-back frame, so kept frames keep
  the encoder's own stamps exactly.
- **The deadband is what keeps SNV untouched (owner: "nerozbabre snv sd1").** SNV's counter is
  anchored at the first frame's arrival, so frames that arrive earlier lead the clock by the NDI
  arrival jitter (<= ~40 ms minus the encode latency). That is below D, so on SNV the guard is an
  identity and SNV keeps exactly the pacing it had before the guard. With D = 0 the guard pulled
  those frames back and changed SNV's pacing (review, 2026-10-13). Do not lower D below SNV's
  jitter; do not raise it toward the 100 ms drift clear level or the −100 ms `futureTimestamps`
  threshold (both must stay above D, and tests pin that).
- **PP's lead is capped at D**: an identity until the drift brings the lead to D (~18 min of
  pipeline age at +48 ppm), then a clocksync hold of <= D, i.e. ≈ +D of latency, never a queue
  overflow.
- **The correction can be negative.** A framerate DROP rescales the counter into a forward jump
  (pulled back, correction grows); a framerate RISE rescales it backwards (re-anchored one frame
  after the previous output, correction < 0). `encoderTimestampCorrectionMs < 0` is not a bug.
- **Identity on PTS-preserving encoders** (x264enc, NVENC, vah264enc >= 1.26). Never "optimise" it
  away or make it encoder-conditional: it costs nothing there and it is the fix on prod.
- **Decide, then commit only what was applied.** `TimestampGuard::decide` is pure;
  `GuardState::process` commits (`TimestampGuard::commit`) only after the decided running time
  converted back to a PTS. A failed conversion passes the buffer untouched AND leaves the guard
  state unchanged.
- **Convert through RUNNING TIME and the encoder's OUTPUT segment, never copy a raw PTS.** The output
  segment carries GstVideoEncoder's `time_adjustment` (vah264enc: 1000 h `min_pts`); a raw copy
  lands hours off — the #768 constant −3600 s class of mistake. Same for any new lateness/lead
  reading: `segment.to_running_time(pts)` vs `element.current_running_time()`.
- Probes are installed while the pipeline is still NULL (they must see the sticky SEGMENT); flush
  events need `PadProbeType::EVENT_FLUSH` explicitly (not part of `EVENT_DOWNSTREAM`).
- Not "copy the input PTS" (upstream 1.26 behaviour): that hands every TV the NDI arrival jitter as
  RTP timestamps — a risk to the weak SNV TVs. The owner's constraint is "never degrade SNV".

## Testing — CI cannot see the bug on its own

CI encodes with x264enc and dev2 with NVENC (both keep the input PTS); the synthetic NDI sender
runs on the server clock (0 ppm). A test that passes without the guard on those hosts is vacuous.
`pipeline/tests/encoder_timestamps.rs` emulates vah264enc 1.24 with a probe on x264enc's src pad
(added BEFORE the guard — probes run in insertion order) and a 40 fps source against a 30 fps
counter into a prod-shaped consumer (`configure_consumer` appsrc → `clocksync sync=true`):

- the regression asserts 0 drops after the join settles and an output lead <= D + 5 ms at the
  producer appsink;
- keep it asserting that the emulated raw lead really passed 500 ms (`rawLeadMaxMs`);
- the negative control (`Guard::Absent`, same harness) must DROP frames — it proves the harness
  turns a lead into drops, so the regression's "0 dropped" cannot silently go vacuous.

x264enc missing = hard failure, never a skip. The pure policy is `TimestampGuard::decide` (integer
unit tests through `apply` = decide + commit). The SNV identity test (0 ppm, one-sided 0–40 ms
jitter, late first frame) must stay exact: 0 pulled back, correction 0. Simulate numeric bounds in
Python with an exact port of the guard and the test xorshift before asserting them — Tier-0 has no
local `cargo test`.

## Self-heal: never trigger rebuilds on `dropRatio30s`

The backstop reads ONLY the pipeline-level lead (30 s window MINIMUM at the producer appsink, >=
300 ms on two consecutive supervisor ticks). Consumer drop ratios include join drops and reconnect
churn (PP SD1 rejoined every ~45 s) and would loop site-wide blackouts — the #263 lockstep lesson.

- The over-window count belongs to ONE pipeline generation (`EncoderTiming::generation`, unique per
  build): a replaced pipeline starts from zero, so windows of two pipelines never add up.
- A tick on a pipeline younger than the 30 s lead window is neutral. A new supervisor's first tick
  fires at once (tokio interval) and a tick missed during a rebuild fires as soon as the loop is
  back; both would otherwise act on a window only seconds long.
- `on_tick` only reports a rebuild as due. It is recorded (`record_rebuild`: spacing, budget,
  `driftRebuilds`) only when `request_self_heal_rebuild` (Streaming → Errored only) returned true.
- Rebuilds then run the normal supervisor path; >= 5 min apart (`SupervisorState::COOL_OFF_WINDOW`),
  <= 3 per hour; the memory lives in `NdiManager.drift_guards` (per source) so the spacing and the
  budget survive supervisor and pipeline replacement.

`driftRebuilds > 0` on `/healthz` means the guard failed — investigate, don't tune the thresholds.

## Verifying on prod (read-only)

- SNV: `encoderTiming.framesPulledBack` 0 and `encoderTimestampCorrectionMs` 0 (the arrival-jitter
  lead stays inside D), unless SNV's source also drifts fast; per-display presented fps from the
  `ndi_client_stats` beacons at its baseline.
- PP after the lead reached D: `timestampLeadMs30s` <= 50, `encoderTimestampCorrectionMs ≈`
  ppm × pipeline age − 50 ms, consumers' `dropRatio30s ≈ 0`, `driftRebuilds` 0.

## Untouchable (refused or failed before — see #854 history)

The system-clock pin, `ntp-time-source=clock-time`, the 500 ms consumer queue, GOP 60, the
constrained-baseline profile, one shared encoder, bitrate, any per-TV/per-consumer adaptation.

## Upgrade note

After a GStreamer >= 1.26 upgrade, vah264enc keeps the input PTS: the guard becomes an identity and
prod RTP cadence turns into raw receive time. Re-measure the TVs' `ndi_client_stats` beacons on
SNV and PP at that upgrade.
