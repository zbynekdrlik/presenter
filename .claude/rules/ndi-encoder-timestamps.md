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

Prod's GStreamer 1.24.2 `vah264enc` stamps its OUTPUT with a frame counter
(`start_pts + frame_duration × n`), discarding ndisrc's receive-time PTS. A source faster than its
declared rate makes that counter lead the clock (PP: +173 ms/h); webrtcbin's per-stream
`clocksync sync=TRUE` holds future buffers and every consumer's 500 ms queue overflows once per
GOP. The guard (`pipeline/encoder_timing.rs`, encoder `src` pad probe) is the fix; the drift
self-heal (`manager/drift_guard.rs`) is its backstop. Full story: the ndi skill's #854 chapter.

## Invariants — keep them

- **No frame leaves the shared encoder stamped later than the pipeline clock** (`out <= now`, `out`
  strictly increasing). The correction only GROWS on a pulled-back frame, so kept frames keep the
  encoder's own cadence exactly (what the TVs always received).
- **Identity on PTS-preserving encoders** (x264enc, NVENC, vah264enc >= 1.26). Never "optimise" it
  away or make it encoder-conditional: it costs nothing there and it is the fix on prod.
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
counter into a prod-shaped consumer (`configure_consumer` appsrc → `clocksync sync=true`). Keep it
asserting that the emulated raw lead really passed 500 ms (`rawLeadMaxMs`) so the harness itself
cannot silently go vacuous. x264enc missing = hard failure, never a skip. The pure policy is
`TimestampGuard::apply` (integer unit tests; simulate numeric bounds in Python before asserting
them — Tier-0 has no local `cargo test`).

## Self-heal: never trigger rebuilds on `dropRatio30s`

The backstop reads ONLY the pipeline-level lead (30 s window MINIMUM at the producer appsink, >=
300 ms on two consecutive supervisor ticks). Consumer drop ratios include join drops and reconnect
churn (PP SD1 rejoined every ~45 s) and would loop site-wide blackouts — the #263 lockstep lesson.
Rebuilds go through `request_self_heal_rebuild` (Streaming → Errored only) and the normal
supervisor path; >= 5 min apart (`SupervisorState::COOL_OFF_WINDOW`), <= 3 per hour; the memory
lives in `NdiManager.drift_guards` so it survives supervisor replacement. `driftRebuilds > 0` on
`/healthz` means the guard failed — investigate, don't tune the thresholds.

## Untouchable (refused or failed before — see #854 history)

The system-clock pin, `ntp-time-source=clock-time`, the 500 ms consumer queue, GOP 60, the
constrained-baseline profile, one shared encoder, bitrate, any per-TV/per-consumer adaptation.

## Upgrade note

After a GStreamer >= 1.26 upgrade, vah264enc keeps the input PTS: the guard becomes an identity and
prod RTP cadence turns into raw receive time. Re-measure the TVs' `ndi_client_stats` beacons on
SNV and PP at that upgrade.
