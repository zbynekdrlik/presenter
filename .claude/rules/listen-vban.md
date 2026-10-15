---
paths:
  - "crates/presenter-core/src/audio/**"
  - "crates/presenter-core/src/listen/**"
  - "crates/presenter-server/src/listen/**"
  - "crates/presenter-server/src/state/listen.rs"
  - "crates/presenter-server/src/router/listen.rs"
  - "scripts/dev/listen/**"
---

# Listening ingest — VBAN microphones (#852 C1a, spec §5)

- **Reuse map (songplayer @ dd551b7d, never vbanvst / audiomatrix):** header, constants, SR indices,
  `f32_to_int16`, `clean_f32` ← `sp-server/src/playback/vban_packet.rs` (core `audio/vban.rs`); the
  encoder (`stream_name_bytes`, `f32_to_int24`, `int24_le`, `sr_index_of`, `write_header_as`, the
  counter) ← same file, TEST-ONLY in core `audio/vban_encode.rs` (`cfg(any(test, feature =
  "test-helpers"))`, presenter-server's dev-dependency enables it; excluded from cargo-mutants); the
  converter pattern ← `vban_rate.rs` (server `listen/resample.rs`); `pcm_dbfs` / `VOICED_DBFS` ←
  `scripts/dub_live_session.py` (core `audio/level.rs`). Decode, routing, counter rule, chain, config,
  status are Presenter's own.
- **Decode** (`decode_packet`) follows spec §5.2 in its order: magic + length, AUDIO, PCM, reserved bit,
  SR index (< 21, else `Corrupt`), INT16/24/32/FLOAT32, size. Every reject is a typed `VbanReject`; no
  slice access without a length check. Decode scale = songplayer's symmetric encoder scale, clamped.
- **The mic list ships EMPTY** — no socket anywhere until mics are saved (`PUT /listen/mics`); dev
  copies prod's DB, so prod stays empty until the routing day. One socket, bound by
  `vban_rx::supervise` only while the list is non-empty; `receive` returns at once if the list was
  emptied before the bind; a taken port → `retrying` + Slovak error, retried 1, 2, 4 … 60 s, WARN on
  power-of-two attempts. Started from `spawn_background_tasks` → validate mode never binds;
  `start_listen` reads `listen.mics` under the save lock (a save during startup is never overwritten).
  Bind address `PRESENTER_LISTEN_VBAN_ADDR` (default `0.0.0.0:6980`); E2E test servers `127.0.0.1:0`.
- **The receive loop never waits** (spec rev. 13 p. 11): decode → route → lock / sequence →
  `BlockQueue::push`, all synchronous; a full queue (64 blocks) drops the OLDEST block
  (`droppedBlocks`). Never put an `.await` on a downstream consumer into `handle_datagram`.
- **Routing:** exact stream name + the sender IP parsed once per saved list (`MicInput::matches`); the
  peer IP is canonicalised (`::ffff:a.b.c.d` → `a.b.c.d`) like the stored one, for routing and for the
  refused-sender keys. No match → `unmatched` (per (name, sender), 16 most recent) and an INFO through
  `unmatched_lines` (a name entering the bounded map logs on power-of-two counts of new names, a repeat
  when its own and the session's counts are both powers of two, one summary at the first eviction). A
  mic locks to its first sender (`ip:port`); another sender of the same name counts
  `otherSenderPackets` and is ignored until the locked one is silent ≥ 3 s, then takes over with a
  fresh counter (no gap, no resync). Non-audio VBAN (VB-Matrix's VBAN-TEXT / SERVICE) is counted in
  `rejected`, DEBUG only, and left out of the WARN gate: every other reject WARNs on the power-of-two
  totals of that sender's non-audio-free rejects (`RejectCounts::count`), so a VB-Matrix host's
  constant VBAN-TEXT never swallows the slots of its rare corrupt / unsupported datagrams. The
  per-sender map keeps only 16 senders, so an evicted sender's count restarts at 1: the WARN gate
  (`reject_lines`, `unmatched_lines`' shape plus a per-sender regime below the 17th sender) also reads
  two session counters from `note_reject_seen` — `reject_senders` (a sender's first WARN-class reject
  counted in the map, again after an eviction) and `reject_total` (every WARN-class reject). Below the
  17th counted sender (`FIRST_EVICTION`) nothing can have been evicted, so a sender logs on its OWN
  power-of-two counts exactly as before (≤ 16 senders — applying the session gate from the first
  sender silenced a second persistent sender after its 1st line, review of 1c8ab852). From the 17th on
  a first reject logs on power-of-two `reject_senders`, a repeat when its own count AND `reject_total`
  are both powers of two (at most 2·(log2 N + 1) further lines), one summary at the 17th counted
  sender; both counters go with the socket (`end_session`).
- **A save** (`MicRouter::update`): a mic whose stream name, sender and channel stay keeps its state,
  counters and chain (a rename or a language change resets nothing); another mic starts fresh with a
  new chain and the update's generation; an unlisted mic's input is dropped → its queue closes → its
  chain drains and ends (`Drop for MicInput`). `record_chain` ignores a generation that is not the
  mic's current one; `ListenHealth::generation` is never reset, so a chain still draining from an
  earlier socket session can never write a later mic.
- **Counter rule** (core `audio/sequence.rs`, `StreamSequencer`): gap ≤ 1 s → silence of
  `missing × frames` before the block; behind ≤ 1 s → drop (`latePackets`, never reaches the chain);
  more than 1 s either way → resync (used, `resyncs`, nothing inserted). "1 s" = `rate / frames` of
  the CURRENT packet (187 at 48 kHz / 256); everything wrapping (u32 counter).
- **Chain** (`mic_chain.rs`): channel `left` = ch 1, `right` = ch 2 (ch 1 if mono), `mix` = mean;
  rubato `=5.0.1` `Fft` + `FixedSync::Both` with **20 ms** chunks (delay 160 samples = 10 ms; a
  whole-100-ms FFT would add 50 ms, over spec §8.5); 16 kHz passes through, 11 025 Hz is refused
  (`error`, mic state `error`); a format change starts a fresh converter. Frames = 1600 s16 samples,
  counted (`framesOut`) with the last one's level (`levelDbfs`); **no frame channel until its first
  consumer** (C1b recorder / C3 ASR) adds it.
- **rubato's lock footprint:** 13 packages, byte-identical to songplayer's lock — incl.
  `audio-codec-algorithms` (a dependency of `audioadapter-sample`, 0BSD OR Apache-2.0). Resolve a
  dependency change with `cargo metadata` (non-compiling) and diff `Cargo.lock`: only `+` lines, no
  existing package's version moves.
- **Toolchain floor = Rust 1.87** (since #852 C1a-3): rubato, audioadapter* and
  audio-codec-algorithms declare `rust-version = 1.87`, so the workspace no longer builds on 1.85/1.86.
  CI builds on stable. `clippy.toml`'s `msrv = "1.85.0"` stays ONLY for lint selection — do NOT bump
  it: 1.87 would fire `manual_is_multiple_of` across the whole repo. Our own code still avoids std APIs
  newer than 1.85 (`incompatible_msrv`).
- **The chain task yields:** `BlockQueue::pop` touches no tokio resource, so `run_chain` calls
  `tokio::task::coop::consume_budget().await` once per block (the workspace tokio floor is 1.44 for
  `task::coop`; `tokio::task::consume_budget` is the deprecated path). A format change logs through
  the #484 gate on the mic's format count (`ListenHealth::record_format`, cleared with the session).
- **Health / status:** one `std::sync::Mutex<ListenHealth>`, never across an `.await`; ages at read time
  (no-audio exactly 3 s after the last packet); the WARN gates read the health counters
  (`PacketTally`, `record_other_sender`, `note_reject_seen`) — no second set of counters. Every session
  counter (mics, unmatched, rejected and their gates) goes when the socket closes (`end_session`).
  `packetsPerSec` is measured on the 1 s tick and shown only while packets arrive (`receiving` /
  `error`); level and `voiced` only while `receiving`. `GET /listen/status` is a quiet GET in
  `request_log.rs`.
- **Config:** whole-list `GET|PUT /listen/mics`, validated in core `validate_mics` (≤ 4, id
  `[a-z0-9-]{1,20}` unique, name 1–40, stream 1–16 printable ASCII without edge spaces, IP canonical,
  no two mics one packet could match) → 422 Slovak text; a corrupt stored value → 500, receiver idle.
  Every save writes one best-effort `settings_audit` row (`app_settings` / `listen.mics`).
- **Logs** (`presenter::listen::mic`): INFO socket bound / closed, `listen mics saved`, `mic receives
  VBAN` (first packet or after ≥ 3 s silence), `mic lost audio`, format change, unassigned stream (gated);
  WARN through the #484 power-of-two gate (lost / late / resync / dropped / second sender per mic, rejects
  per sender), converter refused a format. Dev's `gst-debug.conf` hides this target (observability rule)
  — read it on SNV.
- **Tests:** packets from the ported encoder; routing, late, resync, drop-oldest and sender-lock cases go
  through `MicRouter::handle_datagram` with RFC 5737 senders and explicit `Instant`s (on the
  current-thread runtime the chain cannot drain before the test awaits — `MicRouter::queued` shows what
  was handed over); one real loopback UDP test per path covers the socket; the golden packet is shared
  with `scripts/dev/listen/test_vban_send.py` and written as ≤ 16-char hex pieces (the staging secret
  scanner refuses a 32+ char hex blob). A poll for `framesOut` must ALSO wait for the packet count: a
  20 ms chunk boundary can complete the last frame before the last packet (95 counters, one skipped →
  the 5th frame lands with the 93rd of 94 packets). Resampler / level expectations (−9.03 dBFS for a
  1 kHz half-scale tone, pulse peak at 4160, 899 crossings) come from a Python mirror of rubato's
  `FftResampler`; if CI measures outside a window, compare `output_delay()` with the measurement before
  touching the test — never widen it.
- **Routing day (owner action, spec §20, after C1b):** the owner routes `GUEST_EN` / `INTERP_SK`
  (48 kHz, 16-bit, after the channel mute) to 10.77.9.205:6980; then the two mics are saved (the C1b
  card, or `PUT /listen/mics` with the spec §5.6 list) and the box acceptance runs (spec §5.8): status
  before / after, a 10-min `tcpdump`, `scripts/dev/listen/vban_capture_check.py` (added at that step).
- **FOH routing** (VB-Matrix on fohabl, songplayer `vban-out.md`): only with the owner's go; `Mute` only
  (VB-Matrix ignores `dBGain=-inf`); record the prior values; read back every point after each write.
