# 0008 — NDI stage-display TRUE server→display latency

**Status:** Accepted (2026-07-01)

## Context

The stage display showed a "video · N ms" latency (#479) computed as the rVFC
`expectedDisplayTime − receiveTime` residual. That value is a **browser-local
residual** (jitter-buffer + decode + present) measured *after* the frame's last
packet already arrived. It contains **no network transit and no encode time**,
which produced physically nonsensical readings the user reported:

- Tailscale-from-home read the **lowest** latency (23 ms) — the WAN transit is
  spent *before* `receiveTime`, so it was invisible; only local paint delay remained.
- A weak Hyundai TV read **lower** than the powerful SD1 — the value tracks each
  WebView's frame scheduler, not real lag.

User decision: **the number must be truthful — a false number is worse than none.**

The originally-designed fix (docs/superpowers/specs/2026-06-30-ndi-true-latency-design.md,
ticket T4) used `latency = (report.timestamp + offset) − estimatedPlayoutTimestamp`.
The #509 (T0) probe, run against the **real prod stage TVs**, proved this dead on
our hardware: every real display — AND a control desktop HeadlessChrome on the
same stream — reports `estimatedPlayoutTimestamp` **absent** (`playout_class="absent"`)
after thousands of frames. Our pipeline pins a monotonic `SystemClock` with
`rtpbin ntp-time-source=clock-time`, and Chrome does not surface
`estimatedPlayoutTimestamp` from the resulting RTCP SRs. Making it appear is a deep,
uncertain GStreamer webrtcbin/SR-NTP change and was not worth blocking the metric on.

## Decision

Compute the metric from signals **every real device reliably reports**:

```
latency_ms = render_residual + network_one_way
```

- **render_residual** = rVFC `expectedDisplayTime − receiveTime` (jitter-buffer +
  decode + present), clamped ≥ 0. Falls back to decode duration when the pair is
  absent. This is what the old #479 number measured — kept, because it is real.
- **network_one_way** ≈ **RTT/2** from the `/ndi/time` pipeline-clock offset
  handshake (#510). This is the server→browser transit the old number missed — the
  reason Tailscale read lowest.

No double-count: the residual already covers everything *after* packet arrival; the
network term covers everything *before* it.

**Honesty rules:**
- `network_one_way` unavailable (no fresh `/ndi/time` round-trip, or it aged out per
  the estimator's staleness/RTT trust predicate) → the readout shows **`n/a`**, never
  the residual alone (that is the old misleading number).
- The readout is visible only while NDI video is **live**; non-video layouts hide it.
- Labelled **"server→displej"** — it measures server→display (network + buffer +
  decode + present), NOT camera→NDI→server (only the CI clock-strip measures full
  glass-to-glass), and it is a **video** diagnostic, not a lip-sync guarantee.
- If `estimatedPlayoutTimestamp` ever becomes available (desktop / a future pipeline
  change), the SR-anchored value can be preferred with automatic fallback to this sum.

## Consequences

- The metric now rises with real network + buffer + decode delay and satisfies the
  regression invariants (unit-tested in `ndi_frame_stats.rs`): for the same stream a
  higher RTT (Tailscale/WAN) yields a **higher — never lower** — number than LAN, and
  the value is always **non-negative**.
- It is **honest**: `n/a` when unmeasurable, labelled server→display, presented as a
  video diagnostic. The old residual is no longer shown as a headline number.
- It carries a small extra per-frame read of the offset estimator (cheap, in-memory).
- It does **not** need absolute time sync (dantesync) or `estimatedPlayoutTimestamp`;
  the `/ndi/time` offset keeps both terms addable in the browser clock domain.
