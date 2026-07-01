# NDI Stage-Display TRUE Latency — Analysis & Design

> **Status:** Reviewed (5-lens adversarial review applied) — ready to file as tickets
> **Date:** 2026-06-30
> **Context:** The stage display shows a "video · N ms" latency that does not reflect reality
> (Tailscale-from-home read the *lowest* 23 ms; a weak Hyundai TV read lower than the powerful
> SD1). This doc explains why, and designs a trustworthy replacement.

---

## 1. The bug — what the current number actually measures

`crates/presenter-ui/src/components/stage/ndi_frame_stats.rs:121` (feature #479) computes:

```
latency = expectedDisplayTime − receiveTime        (WebRTC requestVideoFrameCallback metadata)
```

- `receiveTime` = when the frame's **last RTP packet already arrived** in the browser.
- `expectedDisplayTime` = when the browser plans to paint it.

So it is a **browser-local residual** (jitter-buffer + decode + present) measured *after* the packet
lands. It contains **no network transit and no sender/encode time.** Consequences that match the
user's observations exactly:

- **Tailscale-from-home = 23 ms (lowest):** WAN transit is spent *before* `receiveTime`, so it is
  invisible. 23 ms is just local paint delay.
- **Weak Hyundai < powerful SD1:** the value is set by each WebView's frame scheduler + buffer
  depth, not real lag. A cheap TV can report a smaller *residual* while being laggier.
- **"Lags/jumps after a while":** real — but this metric cannot see it, **and (see §3) it may live
  upstream of where this metric measures at all.**

User decision (this session): **make the number truthful** — a false number is worse than none.

---

## 2. Infrastructure — the network's time sync (dantesync)

- **dantesync** (`github.com/zbynekdrlik/dantesync`, v1.8.17) = custom Rust daemon: PTPv1 (Dante
  grandmaster) for *frequency* + NTP (strih.lan) for *UTC phase*. Target precision **< 50 µs**.
- **strih.lan (10.77.9.202)** = the network NTP master, locked to the Dante PTP grandmaster. All
  clients point NTP at it.
- **Install** (Linux): `curl -sSL …/install.sh | sudo bash` — disables chrony/timesyncd, installs a
  FIFO systemd service. **Targets: Linux + Windows x86_64 only.**
- Presenter prod (10.77.9.205) + dev (10.77.8.134) are **not** in dantesync `TARGETS.md` / not
  installed.

### Android TV reality (SD1, Hyundai) — cannot be grandmaster-synced

- No Android/ARM build; Android forbids user-space NTP override + `settimeofday`; PTP unavailable.
- The TV **has** a clock (Android auto-time, ±tens-of-ms…seconds, drifts) — just not disciplinable.
- **The design must not trust the TV's absolute clock** (handled in §3 via the offset handshake).

### Mikrotik NTP redirect — the only Android-friendly path to point the TVs at our grandmaster

dantesync cannot run on the TV, but the **router can transparently redirect the TV's own NTP
traffic to strih.lan** so the TV syncs against our grandmaster-disciplined master instead of Google:

- **DST-NAT UDP/123 → 10.77.9.202** on the Mikrotik, scoped to the stage-TV IPs
  (`chain=dstnat protocol=udp dst-port=123 src-address-list=stage_tvs action=dst-nat
  to-addresses=10.77.9.202`). Hostname/DNS-agnostic — works no matter which NTP server the TV is
  configured for. **Preferred** over a DNS override of `time.android.com` (Android can hardcode an IP
  or use its own resolver, so DNS override is fragile unless you also DNAT :53).
- **What it buys:** the TV gets *roughly* correct UTC from our grandmaster (good for logs, TLS,
  scheduling, and a smaller app-layer offset). **What it does NOT buy:** grandmaster precision —
  Android's SNTP is a coarse one-shot client (syncs ~daily, then free-runs on the crystal → ±tens of
  ms right after a sync, drifting toward ±seconds before the next; it *steps*, not slews). That is the
  **same order of magnitude as the latency we measure**, so it **cannot replace the §3 app-layer
  offset handshake** (which is continuous + precise). It is a worthwhile **complementary** infra
  improvement and the right tool for the user's broader "all devices on the grandmaster" goal — filed
  as **T8** — but the latency metric (T3/T4) does **not** depend on it.

---

## 3. The measurement — robust to an unsynced TV

### Formula (corrected)

```
latency_ms = (report.timestamp + offset_browser→serverPipelineClock) − estimatedPlayoutTimestamp
```

- **`report.timestamp`** = the inbound-rtp stat's **own** timestamp, read from the **same
  `getStats()` snapshot** as `estimatedPlayoutTimestamp`. **Not** a separate post-`await`
  `Date.now()` — that injects the getStats-resolution latency (10–20 ms on a frozen TV WebView) as a
  systematic always-positive bias.
- **`estimatedPlayoutTimestamp`** (inbound-rtp) = playout point of the currently-shown frame, derived
  by Chrome from RTCP Sender Reports. Stock `webrtcbin` sends SRs by default. **No custom GStreamer
  code; getStats path since Chrome M70.** SR cadence ~0.5–5 s → the term *steps* at each SR (must be
  smoothed — see below).
- **`offset_browser→serverPipelineClock`** = a periodic NTP-style round-trip to a new **`/ndi/time`**
  endpoint that returns the server's **GStreamer pipeline-clock** time — *the exact clock the RTCP SR
  encodes* — **not** `SystemTime::now()`. This is the decisive unifier: both terms of the subtraction
  then live in **one clock domain** (the server pipeline clock), so the monotonic-vs-wall and
  NTP-1900-vs-Unix-1970 epoch differences are **absorbed into the offset constant**.

### Why this drops dantesync + wall-clock anchoring off the critical path

Because `/ndi/time` serves the **same** clock domain the SR uses, the latency is a difference *within
one domain* — the server clock's absolute value cancels. So the working metric needs **no** absolute
sync: **the MVP (T3 + T4) ships on today's clock config.** dantesync (T1) and the disciplined-clock
swap (T2) become *enhancements*, not preconditions.

### The TV-sync cancellation — and its limit

The TV's absolute clock error cancels because the offset is measured browser↔server. **But the
cancellation is only as good as RTT *symmetry*:** the offset is biased by `RTT-asymmetry / 2`.
Median filtering removes *jitter*, not a *stable* one-leg-longer bias. Therefore:

- Prefer **min-RTT** sample selection (least-queued round trip ≈ most symmetric), with a median over
  the low-RTT samples.
- **Reject** high-RTT samples; **age out** a stale offset → `n/a`.
- Drive the handshake from the **rVFC callback, not `setInterval`** — TV WebViews throttle
  background/idle timers (the codebase already abandoned `setInterval` for this reason).
- **Tailscale caveat:** subnet-route traffic to 10.77.x can hairpin via a DERP relay with
  loss/asymmetry (project memory `project_cloudflare_turn.md`) → offset bias of *tens* of ms. Over
  WAN the number is **low-confidence**, not exact — flag it, never present it as precise.

### Trust predicate (concrete) — never a plausible-but-wrong number

Show a number **only** when ALL hold; otherwise **`n/a`**:

1. `estimatedPlayoutTimestamp` present + finite + **non-zero** + in a sane range + **strictly
   advancing** across N samples (it is 0/absent until the first SR; `Reflect::get(...).as_f64()`
   returns `Some(0.0)` for a literal 0 — a naive check surfaces a bogus huge latency in the first
   seconds);
2. offset **fresh** (age + RTT under bounds) from ≥1 successful `/ndi/time` round-trip;
3. ≥1 RTCP SR observed;
4. offset **dispersion** under a bound (degraded WAN → `n/a` / low-confidence).

Negative handling (replace the current blanket clamp-to-0 in `derive_video_latency_ms`): small
negative within budget → `0` / "<N ms"; large or persistent negative → **`n/a` + log** (a sign/domain
regression must surface, not be hidden).

### Smoothing

Smooth the **final** latency (EMA/median over a multi-second window) — not only the offset — so the
SR-step in `estimatedPlayoutTimestamp` doesn't sawtooth the readout (the reused `smooth_latency` EMA
helper). Hold the last good value across the first SR-less window; show `n/a` only before the first SR.

### Server clock anchoring (T2 — enhancement, NOT `ntp-time-source=ntp`)

Today `build.rs:49-62` pins the **monotonic** `SystemClock` and `consumers.rs:~720` sets `rtpbin
ntp-time-source=clock-time` — **deliberately**, to stop a ~400 ms render pause every ~20 s on all TVs
at once. **Do NOT set `ntp-time-source=ntp`** (it makes RTP(monotonic) and SR-NTP(realtime) drift; a
dantesync-disciplined `CLOCK_REALTIME` makes that drift *worse*). Instead, *if* a human-readable
wall-clock value is wanted, **replace the pipeline clock** with a disciplined `gstreamer_net::NtpClock`
(→strih.lan) or `PtpClock`, force-pinned via `use_clock()` (so `ndisrc` slaving stays off), **keeping
`ntp-time-source=clock-time`** so RTP and SR-NTP stay on ONE clock. Ship with a regression guard
asserting the ~400 ms/~20 s "naraz" hitch does not return. `gstreamer-net 0.25.0` is already in
Cargo.lock.

### GStreamer abs-capture-time (T6 — stretch)

GStreamer ships **no** `abs-capture-time` extension (only RFC-6051 `rtphdrextntp64`, which Chrome
**ignores** for `captureTimestamp`). The high-fidelity `getSynchronizationSources().captureTimestamp`
path needs a **full custom `GstRTPHeaderExtension` subclass** (NTP-64 wire read/write +
`set_attributes`/`set_caps_from_attributes`) **plus SDP `extmap` negotiation** via webrtcbin — not a
single `add-extension` call. It depends on T2 (the value must be wall-clock NTP from the disciplined
clock), is feature-detected, and is **strictly additive — must never override** the
`estimatedPlayoutTimestamp`/`n/a` fallback (almost certainly absent on frozen Android-11 WebViews).

### Scope honesty — what this is, and is NOT

- It measures **server → display** (network + jitter buffer + decode + present). The
  **camera → NDI → server** hop is **not** included; only the synthetic CI clock-strip measures full
  glass-to-glass. UI label: **"server→displej"**, never "glass-to-glass".
- It is a **video** latency diagnostic, **NOT a lip-sync meter** — it carries no audio-path
  information, so it cannot by itself tell the user whether lips match sound. §5 must not over-promise.
- The recurring "lags/jumps after a while" may live in the **unmeasured NDI ingest** hop — T0 must
  locate it before T4 trusts a server→display number to explain it.

---

## 4. Ticket decomposition (corrected backlog)

| # | Title | Repo | Depends | Bundle-safe |
|---|---|---|---|---|
| **T0** | Device-capability + lag-location probe (gates T4) | presenter | — | no |
| **T1** | Deploy dantesync to presenter prod + dev (off critical path) | dantesync | — | no |
| **T8** | Mikrotik: DST-NAT TV NTP → strih.lan (Android-friendly grandmaster sync) | dantesync/infra | — | no |
| **T2** | Replace pipeline clock with a disciplined network clock (enhancement) | presenter | T1 | no |
| **T3** | `/ndi/time` (pipeline-clock) endpoint + browser offset estimator | presenter | — | no |
| **T4** | True server→display metric + honest display + docs/ADR | presenter | T0, T3 | no |
| **T6** | (Stretch) `abs-capture-time` `GstRTPHeaderExtension` + `captureTimestamp` | presenter | T2, T4 | no |
| **T7** | Extend `/ndi/client-stats` beacon with latency + offset + RTT | presenter | T3, T4 | yes (fold into T4 unless over ceiling) |

- **T0 is the mandatory first work item.** Prove on the real SD1 + Hyundai WebViews that
  `estimatedPlayoutTimestamp` is present/finite/advancing/in-epoch (vs the report's own Unix-epoch
  `.timestamp`), via the existing `/ndi/client-stats` beacon; AND add server-side NDI receive-time
  logging to locate whether the drift is server→display or upstream. The metric is defined **only**
  on fields the real devices prove correct.
- **No standalone tests ticket.** Tests ship with the code they cover (TDD / regression-test-first /
  single-feature-single-PR): clock-strip-agreement + n/a-never-fake + Tailscale-≥-LAN + non-negative
  E2E → **T4**; estimator min-RTT/median/age-out unit tests → **T3**; the ~400 ms/~20 s hitch
  regression guard → **T2**.
- **Docs are a T4 deliverable** (the metric's user-visible semantics change + a new route/env):
  `docs/architecture.md` metric section + `docs/adr/NNNN-ndi-true-latency.md` + `CLAUDE.md` notes.

### Quality-gate landmines (re-measured)

- `consumers.rs` = **842** prod lines (cap 1000) **and** `build_consumer_pipeline_blocking` =
  **115/120** lines (fn cap, not exempt) → T2/T6 clock edits must go in a **helper / new
  `pipeline/timestamp.rs`**, never grow that function.
- `ndi_frame_stats.rs` = **570** (not 431), server `ndi.rs` = **251** (not 169), `status_bar.rs` =
  156 — all under the file cap; T4's new logic lands in a **new presenter-ui module** to keep the diff
  clean and files under cap.

---

## 5. Acceptance — does it answer the complaint?

- The number rises with real network/buffer/decode delay and **changes correctly** LAN vs Tailscale
  and across device decode speed — locked by a **regression invariant**: for the same stream the
  Tailscale reading must **never** be lower than the LAN reading, and the LAN clock-strip path must
  assert it is **non-negative**.
- It is **honest**: labelled server→display, shows `n/a` (not a lie) when the trust predicate fails,
  and is presented as a **video diagnostic, not a lip-sync guarantee**. The old residual is
  demoted to a debug-only overlay (accurately labelled), never a second headline number.
- It is **validated** against the clock-strip ground truth in CI.
- T0 first answers the open question — whether the metric even works on the real TVs, and where the
  "lags after a while" actually lives — before T4 builds the headline number on it.
