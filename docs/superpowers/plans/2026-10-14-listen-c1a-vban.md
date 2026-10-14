# Listening C1a — VBAN Ingest Core (#852) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship unit **C1a** of #852 in three lanes, each its own PR unit: Presenter receives the VBAN microphones (`GUEST_EN`, `INTERP_SK`) on one UDP socket, decodes, routes and counts every packet, applies the frame-counter rule, turns each configured microphone into 100 ms frames of 16 kHz mono s16 with a level, and serves the per-mic health the box acceptance needs (spec §5.8: packets, lost / late packets, resyncs, last-packet age, level, format) as `GET /listen/status`. The microphone list is the `app_settings` row `listen.mics`, read and replaced through `GET|PUT /listen/mics`. **It ships EMPTY everywhere**: no socket is bound until mics are saved (PP, C-S8; and SNV until the owner's routing day, see "Routing day" at the end). The Settings card and the recorder are C1b, the ASR is C3; the per-mic frame channel arrives with its first consumer (C1b / C3) — in C1a, `framesOut` and `levelDbfs` in `/listen/status` prove the chain.

**Architecture:** The pure parts live in `presenter-core`, shared with C1b's WASM UI: the VBAN codec (`audio/vban.rs`, songplayer's production encoder ported with the decode direction added, every reject typed), the level meter (`audio/level.rs`, songplayer's `pcm_dbfs`), the config + status DTOs (`listen/mics.rs`, `listen/status.rs`) and, in lane 3, the frame-counter rule (`audio/sequence.rs`). The songplayer encoder itself is ported test-only (`audio/vban_encode.rs`, `cfg(any(test, feature = "test-helpers"))`, excluded from cargo-mutants) and generates the packets of every test. The server owns the I/O in a new top-level `listen` module (the `playback` precedent): one supervisor task binds the socket only while `listen.mics` is non-empty (retrying a taken port with the #484 backoff); the receive loop decodes, routes by stream name (+ the sender IP, parsed once per saved list) and counts, never waiting on anything downstream. Lane 3 adds per mic the sender lock, the counter rule and a bounded drop-oldest queue to a chain task that picks the channel, converts with rubato's `Fft` + `FixedSync::Both` (songplayer's converter pattern, 20 ms chunks) and counts 100 ms frames with their level. One `std::sync::Mutex<ListenHealth>` holds the counters; `GET /listen/status` snapshots it with ages measured at read time, so a stopped sender reads "no audio" exactly 3 s after its last packet. `AppState` holds a `ListenService`; `spawn_background_tasks` starts the socket task, so validate mode never binds.

**Tech Stack:** Rust 2021 (clippy msrv 1.85), tokio 1.52 (`net`, `sync`, `time`), axum 0.8, rubato `=5.0.1` (lane 3; MIT OR Apache-2.0, the same pin as songplayer), serde / thiserror / tracing; Python 3 stdlib for the box sender (`scripts/dev/listen/`).

**Spec:** `docs/superpowers/specs/2026-10-14-sermon-listening-interpreter-assist-design.md` (commit 754d1a4f; APPROVED by the owner 2026-10-14, #852) — §1 C-S1 / C-S8, §2 (two VBAN mics, reuse songplayer), §3 (reuse table), §4, §5.1–§5.8 (ingest, decode, selection / ordering / gaps, per-mic chain, config, sender side, box acceptance), §8.5 (VBAN ≤ 30 ms, framing + resampler ≤ 120 ms), §11 (VBAN rows), §14 (observability), §15 "Audio and listening", §16 (unit C1a), §18 defaults 7 and 9, §20 (owner action).

**Review:** round 1 (2026-10-14) — three lanes; no frame channel without a consumer; the capture checker moved to the acceptance step; an empty mic list by default; the boot race, the empty-before-bind race, keep-the-mic-on-rename, no duplicated event counters, the sender IP parsed once, `audioadapter-sample`, the unassigned-stream gate, the test split and the mutants exclusion are folded in below.

## What is reused from songplayer, and what is new

Songplayer has a production VBAN **sender** only (no receiver, spec §3). Everything below is read from the local clone of `zbynekdrlik/songplayer` at **`dd551b7d`** (`dev`, v0.81.0 — the spec's songplayer baseline). `vbanvst` and `audiomatrix` are dead experiments and are not a source.

| Songplayer @ dd551b7d | Ported into (lane) | What exactly |
|---|---|---|
| `crates/sp-server/src/playback/vban_packet.rs` | `presenter-core/src/audio/vban.rs` (production, C1a-1) | `VBAN_MAGIC`, `VBAN_HEADER_LEN`, `VBAN_STREAM_NAME_LEN`, `VBAN_DATA_MAX`, `INT16_FULL_SCALE`, `INT24_FULL_SCALE`, the header table (doc), the SR indices (48k = 3, 96k = 4, 192k = 5, 44.1k = 16, 88.2k = 17) inside the full rev. 13 list, the format bits (INT16 0x01, INT24 0x02, FLOAT32 0x04), `f32_to_int16`, `clean_f32` (verbatim) |
| same file | `presenter-core/src/audio/vban_encode.rs` (test-only + `test-helpers`, C1a-1) | `stream_name_bytes`, `f32_to_int24`, `int24_le` (verbatim), `sr_index_of` (widened from 5 rates to the spec's 21), `write_header_as` (widened from the fixed stereo 30 fps geometry to any 1–256 frames × 1–256 channels → `header`), the `VbanEncoder` counter (`next_counter`, `starting_at`, +1 per packet, wrapping at `u32::MAX`). NOT ported: `VbanFormat`'s grid geometry, the packet schedule, the send latency — sender pacing (spec §3 "Not reused") |
| `crates/sp-server/src/playback/vban_packet_tests.rs` | `presenter-core/src/audio/vban/tests.rs` (C1a-1) | `parse_packet`'s INT24 sign extension → the PRODUCTION decode, without its `assert!`s; tests ported: `the_header_is_the_vban_spec_layout_little_endian` (same expected bytes), `the_stream_name_is_16_ascii_bytes_zero_padded` and `int24_conversion_is_bit_exact_and_clamps` (verbatim), `the_frame_counter_wraps_at_u32_max` (adapted), `ramp_block` (→ `ramp(samples)`) |
| `crates/sp-server/src/playback/vban_packet_tests_format.rs` | same tests file (C1a-1) | the 15-row `TABLE` (rate × format → frames, packets, packet length) as round trips through the new decoder; `int16_and_float32_samples_are_exact` (verbatim); `the_sr_index_and_format_bit_follow_the_spec` (adapted to the full list + INT32) |
| `scripts/dub_live_session.py` (= `eval/dubbing/live_translate_continuous_probe.py`) | `presenter-core/src/audio/level.rs` (C1a-1) | `pcm_dbfs` (RMS of s16 / 32768, floor `SILENCE_DBFS` = −120, rounded to 0.01 dB), `VOICED_DBFS` = −50 (`db > VOICED_DBFS`) |
| `crates/sp-server/src/playback/vban_out_tests.rs` (`one_block_over_loopback_udp_decodes…`) | `listen/tests.rs`, `router/listen.rs` tests (C1a-2 / -3) | the real-socket test pattern: `UdpSocket::bind("127.0.0.1:0")`, encoder → socket → decode, bounded waits |
| `crates/sp-server/src/playback/vban_rate.rs` | `presenter-server/src/listen/resample.rs` (C1a-3) | the converter pattern: rubato `=5.0.1` `Fft` with `FixedSync::Both`, built in its own fn (cargo-mutants never mutates `new`), checked so `input_frames_next()` / `output_frames_next()` are the wanted sizes or refused with a reason, a refused chunk goes out as silence, `InterleavedSlice` adapters, `process_into_buffer(…, None)`. Changed: mono, 16 kHz out, 20 ms chunks (decided default 1) |
| `crates/sp-server/src/playback/vban_rate_tests.rs` | `resample.rs` tests (C1a-3) | `a_1khz_tone_stays_1khz_at_96k` → `…_48k_to_16k`, `every_other_rate_gives_exactly…` → exact 100 ms frames for every VBAN rate, `silence_in_is_silence_out`, `a_converter_rubato_refuses…` → 11 025 Hz |
| `.claude/rules/vban-out.md` | `.claude/rules/listen-vban.md` (C1a-3) | port 6980, "the RECEIVE thread never waits" (spec rev. 13 p. 11), drop the OLDEST block and count it, receivers keep ≥ 6 × 256 samples and senders burst 2–4 packets, box acceptance = tcpdump (0 counter gaps, interval p99) with the status read before / after, FOH routing on fohabl = VB-Matrix over VBAN-TEXT, `Mute` only, read back every point, owner's go |

**New (no songplayer counterpart):** the decode direction and its reject taxonomy (§5.2), the receive socket + bind supervisor, routing with a parsed sender IP, the per-mic sender lock, the counter rule (gap → silence, late → drop, > 1 s → resync, §5.3), the per-mic chain (drop-oldest queue, channel pick, resampler, 100 ms frames, level), the `listen.mics` config (DTOs, validation, `GET|PUT /listen/mics`, settings audit row), `GET /listen/status` and its counters, the Python VBAN sender; and, at the acceptance step, the capture checker.

## Global Constraints

- **Three lanes, three PR units:** C1a-1 core (Tasks 1–4), C1a-2 receiver + API (Tasks 5–9), C1a-3 chain (Tasks 10–14). Each lane compiles and is tested at its own head (CI on its push), has no `pub(crate)` item without a production caller (test-only accessors are `#[cfg(test)]`) and stays ≤ ~700 raw production lines (measured per lane in its last task). A lane starts from `origin/dev` with the previous lane merged.
- **Version:** dev is unreleased (0.4.321 on 2026-10-14, main below it) → no bump, unless main caught up with dev before a lane starts; then that lane's FIRST commit bumps `[workspace.package].version` in `Cargo.toml` + `Cargo.lock` (`version-bumping`; recipe: the šepkár plan, Task 1).
- Tier-0: NO local compiling cargo (`build` / `check` / `clippy` / `test` / `run`). Allowed locally: `cargo fmt --all` (+ `-- --check`), `cargo metadata` (Task 11 needs it online once to resolve rubato into `Cargo.lock`), the Python / bash gates, `python3 -m unittest`. Every Rust test is verified by CI after the lane's push. Never put personal data (e-mail, names) in any network request; crate sources are in `~/.cargo/registry`, songplayer is a local clone.
- File cap ≤ 1000 production lines (warn > 800) via `bash scripts/dev/count_prod_lines.sh <file>`; function cap ≤ 120 (warn > 80) via `QC_TARGETS="<newline-separated files>" python3 scripts/dev/fn_length_check.py .` — on EVERY touched `.rs` file. Near-cap files: `crates/presenter-server/src/state/mod.rs` (947 prod lines → ≤ 951, lane 2), `crates/presenter-server/src/router.rs` (768 → 770, lane 2). Only `tests.rs` / `tests/*.rs` and an inline `#[cfg(test)] mod tests { … }` at a file's end are size-exempt. `fn_length_check.py` does not see generic fns (`with_health<R>`, `Recent::touch`): they are short by construction.
- No `unwrap()` / `expect()` / `panic!` in production code (`python3 scripts/dev/no_panic_check.py .`). `audio/vban_encode.rs` (`cfg(any(test, feature = "test-helpers"))`) counts as PRODUCTION for that gate and is written panic-free. A `std::sync::Mutex` is locked with `.unwrap_or_else(PoisonError::into_inner)` and never held across an `.await`. Network data never panics: every slice access in the decoder is behind a length check.
- CI clippy = `cargo clippy --workspace --all-targets --all-features -- -D warnings -W clippy::all`, msrv 1.85 (`clippy.toml`): no std API newer than 1.85 (`x % n != 0`, never `is_multiple_of`; `std::iter::repeat_n` and `Option::is_none_or` are 1.82 and fine); `.ok_or(..)` for a cheap value; no `&vec![..]` where a slice works (`useless_vec`, tests included); wrapped `///` list items indented. From lane 3 on, the effective toolchain floor is **Rust 1.87** (rubato, audioadapter* and audio-codec-algorithms declare `rust-version = 1.87`; CI builds on stable); `clippy.toml`'s msrv stays 1.85 for lint selection only — never bump it (1.87 would fire `manual_is_multiple_of` across the repo).
- The staging secret scanner refuses a 32+ char hex blob: the golden test packet is written as ≤ 16-char hex pieces (`concat!` / adjacent strings).
- Startup is read-only against settings: the receiver READS `listen.mics` (under the save lock, Task 7); only `PUT /listen/mics` writes it (plus one best-effort `settings_audit` row, the `ai-settings` pattern).
- Public repo: no e-mails, keys or IPs beyond the spec's; tests use RFC 5737 senders (`192.0.2.10`, `198.51.100.7`, `203.0.113.9`) and loopback. The dev instance's address is not written here: `$DEV` / `$DEV_HOST` come from CLAUDE.md "Deployed Instances".
- The placeholder gate fails on "coming soon" / "not implemented" and non-comment `TODO|FIXME|HACK`.
- Operator-facing text (the 422 rules, the status `error` strings) is Slovak; log messages are English, as in the rest of the server.
- Two-branch flow: commit on the lane's worktree branch the supervisor integrates into `dev` (or on `dev`); PR dev→main with a merge commit (never squash / rebase / `--admin`); merge only when every check is green and the PR is `mergeable: true` + `mergeable_state: clean`; PP gets a release via `gh release create vX.Y.Z --target main`. Opening a release PR is the supervisor's call when dev is quiet (`.claude/rules/quality-gates.md` "Shared dev under a multi-worker fleet").
- Features, not bug fixes: no RED/GREEN commits, but every task ships its tests in the same lane. Commit subjects `type(scope): … (#852 C1a-N)`; `[no-test: <reason>]` on a commit without tests; every message ends with a blank line + the session's `Co-Authored-By: Claude Opus 5.5 (1M context) …` attribution line (as every commit of this repo). Never rewrite history; stage explicit paths only.
- Before a push load `ci-push-discipline`; after it monitor the run to a terminal state (`ci-monitoring`, bounded foreground polls, never `gh run watch`); after a deploy `post-deploy-verification`; after each lane `playbook-review`.

Shell variables used below:

```bash
REPO=/home/newlevel/devel/presenter/presenter-dev2   # or your worktree root
SCRATCH=<your session scratchpad directory>           # the system prompt lists it
DEV=<dev instance URL, CLAUDE.md "Deployed Instances">   # http://<host>:8080
DEV_HOST=<the dev instance's host from the same table>
SNV=http://10.77.9.205
PP=http://companion-pp.lan
```

Commit recipe (every "Commit" step): write the message to `$SCRATCH/msg.txt` with the Write tool (subject, blank line, body, blank line, the `Co-Authored-By:` line), run `cargo fmt --all`, then `git -C "$REPO" add <the listed paths> && git -C "$REPO" commit -F "$SCRATCH/msg.txt"`.

**How the tests are split (review 1g):** the late / resync / drop-oldest / sender-lock / routing / reject cases are tested by calling `MicRouter::handle_datagram` directly with RFC 5737 senders and explicit `Instant`s (deterministic, no sleep, and on the current-thread test runtime the chain task provably cannot drain the queue); ONE real loopback UDP test per lane covers the socket path end to end (`real_udp_packets_reach_the_mic_through_the_socket` in lane 2, `real_udp_packets_come_out_as_16_khz_frames_at_the_tone_level` in lane 3), plus the HTTP integration test `real_vban_packets_over_udp_show_in_get_listen_status` and the socket life-cycle tests (bind, idle, taken port), all on `127.0.0.1:0` with bounded 5 s polls.

## Review Focus

The spec says what the ingest must do; these five conditions it implies but does not spell out are the most likely to bite during a service. Each line names the tests that pin it.

1. **A sender restart, a reordered burst and the 32-bit wrap must not read as lost audio or freeze a stream.** The 1 s edge is counted in packets of the CURRENT length, both directions; a counter 1 s + 1 packet away is a resync (used), not 187 packets of silence. Tests: `exactly_one_second_ahead_is_still_a_gap_and_one_more_is_a_resync`, `a_second_behind_is_late_and_more_is_a_resync`, `the_counter_wraps_at_u32_max` (Task 10); `a_jump_of_more_than_a_second_resyncs_and_counting_goes_on` (Task 13).
2. **The receive loop never waits.** A chain that falls behind must cost the oldest audio, counted and logged on a power-of-two gate — never a blocked socket. Tests: `a_full_queue_drops_the_oldest_block` (Task 12), `a_full_queue_drops_the_oldest_block_and_counts_it` (Task 13).
3. **Two senders of one stream name, and VB-Matrix's other VBAN traffic on 6980.** VB-Matrix and Voicemeeter both sending `GUEST_EN` would interleave two counters; VBAN-TEXT / SERVICE and an unassigned stream are normal and must not flood the journal. Tests: `an_unassigned_stream_is_logged_on_power_of_two_packets_per_name` (Task 6); `rejects_are_counted_per_sender_and_kind`, `non_audio_vban_is_counted_but_logged_at_debug_only`, `a_second_sender_of_the_stream_is_ignored_until_the_first_is_silent_for_3_s` (Task 13).
4. **The socket's life cycle and the config's.** Empty list → no socket (the default everywhere), first mic → bind, taken port → retry and say so, list emptied before or after the bind → port released, a save during startup is never overwritten, a rename keeps the mic's state, validate mode never binds, an E2E test server never takes 6980. Tests: `no_mic_binds_no_socket_and_a_saved_mic_binds_one`, `a_taken_port_is_retried_and_reported_until_it_is_free`, `receive_ends_at_once_when_the_list_was_emptied_before_it_started`, `a_rename_keeps_the_mic_and_a_new_stream_name_starts_it_fresh` (Task 6); `the_startup_read_waits_for_a_save_in_progress`, `validate_startup_mode_never_starts_the_vban_receiver`, the `support.ts` default (Task 7); `a_rename_keeps_the_chain_and_a_new_channel_restarts_it` (Task 13).
5. **The 16 kHz output is exactly what the ASR will hear.** Exact 1600-sample frames for every VBAN rate, a constant 160-sample delay, the configured channel, inserted silence where packets were lost, the level measured on the s16 frame. Tests: `every_supported_rate_gives_exact_100_ms_frames`, `the_delay_is_a_constant_160_samples` (Task 11), `the_chain_picks_the_configured_channel`, `inserted_silence_is_part_of_the_output` (Task 12), `real_udp_packets_come_out_as_16_khz_frames_at_the_tone_level` and the extended `real_vban_packets_over_udp_show_in_get_listen_status` (Task 13).

---

## File map

| Lane | File | Responsibility |
|---|---|---|
| 1 | `crates/presenter-core/Cargo.toml`, `.cargo/mutants.toml` | `test-helpers` feature; the test-only encoder excluded from mutants |
| 1 | `crates/presenter-core/src/lib.rs` | `pub mod audio;`, `pub mod listen;` |
| 1 | `crates/presenter-core/src/audio/{mod,vban,vban_encode,level}.rs`, `audio/vban/tests.rs` | codec, test-only encoder, level meter + tests |
| 1 | `crates/presenter-core/src/listen/{mod,mics,status}.rs` | `MicConfig` + validation, `/listen/status` DTOs (socket, packet / format / no-audio fields, unmatched streams) |
| 2 | `crates/presenter-server/Cargo.toml`, `src/lib.rs` | tokio `net`, the core `test-helpers` dev-dependency; `mod listen;` |
| 2 | `crates/presenter-server/src/listen/{mod,health,vban_rx}.rs`, `listen/tests.rs` | `ListenService`, health + snapshot, socket supervisor + receive loop + routing |
| 2 | `crates/presenter-server/src/state/listen.rs` (+ `mod.rs`, `background_tasks.rs`, `tests.rs`) | glue, field, start, validate-mode test |
| 2 | `crates/presenter-server/src/router/listen.rs` (+ `router.rs`, `router/request_log.rs`) | 3 endpoints + tests, merge, quiet `/listen/status` |
| 2 | `tests/e2e/support.ts` | `PRESENTER_LISTEN_VBAN_ADDR=127.0.0.1:0` for test servers |
| 2 | `scripts/dev/listen/{vban_send,test_vban_send}.py`, `.github/workflows/pipeline.yml` | box sender + CI step |
| 2 | `docs/configuration.md`, `CLAUDE.md` (env row) | docs |
| 3 | `crates/presenter-core/src/audio/sequence.rs`, `audio/mod.rs`, `listen/status.rs` | counter rule; `packetsPerSec`, the counter-rule / chain fields, `rejected` |
| 3 | `crates/presenter-server/Cargo.toml`, `Cargo.lock` | rubato `=5.0.1` |
| 3 | `crates/presenter-server/src/listen/{resample,mic_chain}.rs` | `Resampler16k`, `BlockQueue`, `MicChain`, `run_chain` |
| 3 | `crates/presenter-server/src/listen/{mod,health,vban_rx}.rs`, `listen/tests.rs` | chain counters, packets per second, counted rejects (moved here from lane 2 for its size budget), sender lock + counter rule + queue in `MicInput` |
| 3 | `.claude/rules/listen-vban.md`, `CLAUDE.md` (router row), `docs/configuration.md` | playbook, docs |
| after C1b | `scripts/dev/listen/{vban_capture_check,test_vban_capture_check}.py` | box acceptance (spec §5.8) |

---

## PR unit C1a-1 — core

Starts from `origin/dev`. Version: dev is unreleased; no bump unless main caught up.

### Task 1: The core VBAN codec — decode (production) + the ported encoder (test-only)

**Files:**
- Modify: `crates/presenter-core/Cargo.toml` (new `[features]` table after `[dependencies]`)
- Modify: `crates/presenter-core/src/lib.rs:15` (insert `pub mod audio;` after `pub mod android_stage_display;`)
- Create: `crates/presenter-core/src/audio/mod.rs`, `crates/presenter-core/src/audio/vban.rs`, `crates/presenter-core/src/audio/vban_encode.rs`
- Modify: `.cargo/mutants.toml` (`exclude_globs`: the test-only encoder)
- Test: `crates/presenter-core/src/audio/vban/tests.rs`

**Interfaces:**
- Consumes: serde, thiserror (core deps).
- Produces (`presenter_core::audio::vban::…`, no root re-export): consts `VBAN_MAGIC: [u8; 4]`, `VBAN_HEADER_LEN = 28`, `VBAN_STREAM_NAME_LEN = 16`, `VBAN_DATA_MAX = 1436`, `VBAN_DEFAULT_PORT: u16 = 6980`, `INT16_FULL_SCALE: i16 = 32_767`, `INT24_FULL_SCALE: i32 = 8_388_607`, `VBAN_SAMPLE_RATES: [u32; 21]`; `enum VbanSampleType { Int16, Int24, Int32, Float32 }` (serde lowercase; `from_data_type(u8) -> Option<Self>`, `data_type() -> u8`, `bytes() -> usize`, `as_str() -> &'static str`); `struct VbanAudioFormat { rate_hz: u32, channels: u16, sample_type: VbanSampleType }` (Copy, Hash, serde camelCase); `struct VbanPacket { stream_name: String, counter: u32, format: VbanAudioFormat, frames: u16, samples: Vec<f32> }`; `enum VbanReject { NotVban, NonAudio(u8), UnsupportedCodec(u8), UnsupportedType(u8), Corrupt }` (Copy, `Error`); `fn decode_packet(&[u8]) -> Result<VbanPacket, VbanReject>`; `fn stream_name_of(&[u8]) -> String`; `fn f32_to_int16(f32) -> i16`; `fn clean_f32(f32) -> f32`.
- Produces (`presenter_core::audio::vban_encode::…`, only under `cfg(any(test, feature = "test-helpers"))`): `stream_name_bytes`, `f32_to_int24`, `int24_le`, `f32_to_int32`, `sr_index_of`, `header(rate_hz, frames, channels, sample_type, name, counter) -> Option<[u8; 28]>`, `write_sample`, `struct VbanEncoder` (`new(name, rate_hz, channels, sample_type)`, `starting_at(self, u32) -> Self`, `next_counter`, `skip(u32)`, `packet(&[f32]) -> Option<Vec<u8>>`, `packets(&[f32], frames_per_packet) -> Vec<Vec<u8>>`), `ramp(samples) -> Vec<f32>`. (C1a-1 review 1: `tone(rate_hz, hz, amplitude, start_frame, frames, channels) -> Vec<f32>` (Step 5 below) is NOT in lane 1; lane 2 adds it to `vban_encode.rs` with its first caller, Task 6.)

- [ ] **Step 1: The feature and the module skeleton.** In `crates/presenter-core/Cargo.toml`, after the `[dependencies]` table:

```toml
[features]
# #852 C1a: the songplayer VBAN encoder, ported test-only (`audio::vban_encode`) —
# the packet generator for this crate's tests and, through presenter-server's
# dev-dependency, for the real-UDP receiver tests. Never in a production binary.
test-helpers = []
```

`crates/presenter-core/src/audio/mod.rs`:

```rust
//! Audio ingest building blocks (#852 C1a, spec §5): the VBAN packet codec,
//! the per-stream frame-counter rule and the level meter. Pure, no I/O — the
//! server's `listen` module owns the socket and the resampler.

pub mod vban;
#[cfg(any(test, feature = "test-helpers"))]
pub mod vban_encode;
```

In `crates/presenter-core/src/lib.rs`, after `pub mod android_stage_display;`:

```rust
pub mod audio;
```

- [ ] **Step 2: Write the failing tests** — `crates/presenter-core/src/audio/vban/tests.rs` (declared by `#[cfg(test)] mod tests;` at the end of `vban.rs`, Step 4):

```rust
//! #852 C1a: the VBAN decoder against songplayer's pinned encoder bytes
//! (`vban_packet_tests.rs` / `vban_packet_tests_format.rs` @ dd551b7d) and
//! the spec's receive checks (rev. 13 p. 10–11).

use super::*;
use crate::audio::vban_encode::{
    f32_to_int24, f32_to_int32, header, int24_le, ramp, sr_index_of, stream_name_bytes,
    VbanEncoder,
};
use VbanSampleType::{Float32, Int16, Int24, Int32};

/// INT16 mono 48 kHz, 4 frames [0.0, 0.5, -0.25, 1.5], stream GUEST_EN, counter
/// 0x01020304 — the same packet `scripts/dev/listen/test_vban_send.py` pins,
/// so the Python box sender and this decoder cannot drift apart.
const GOLDEN: &str = concat!(
    "5642414e",         // VBAN
    "03030001",         // SR index 3 (48 kHz), 4 frames, 1 channel, INT16 PCM
    "47554553545f454e", // "GUEST_EN"
    "0000000000000000", // name padding
    "04030201",         // counter 0x01020304
    "0000004000e0ff7f", // 0, 16384, -8192, 32767
);

fn hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

fn golden_with(index: usize, value: u8) -> Vec<u8> {
    let mut bytes = hex(GOLDEN);
    bytes[index] = value;
    bytes
}

#[test]
fn golden_packet_decodes_and_the_encoder_rebuilds_it() {
    let packet = decode_packet(&hex(GOLDEN)).unwrap();
    assert_eq!(packet.stream_name, "GUEST_EN");
    assert_eq!(packet.counter, 0x0102_0304);
    assert_eq!(
        packet.format,
        VbanAudioFormat { rate_hz: 48_000, channels: 1, sample_type: Int16 }
    );
    assert_eq!(packet.frames, 4);
    let ints: Vec<i16> = packet.samples.iter().map(|&x| f32_to_int16(x)).collect();
    assert_eq!(ints, vec![0, 16_384, -8_192, 32_767]);
    let mut encoder = VbanEncoder::new("GUEST_EN", 48_000, 1, Int16).starting_at(0x0102_0304);
    assert_eq!(encoder.packet(&[0.0, 0.5, -0.25, 1.5]).unwrap(), hex(GOLDEN));
    assert_eq!(encoder.next_counter(), 0x0102_0305);
}

#[test]
fn the_header_is_the_vban_spec_layout_little_endian() {
    // songplayer vban_packet_tests.rs, the same expected bytes.
    let h = header(48_000, 200, 2, Int24, "sp-program", 0x0102_0304).unwrap();
    let expected: [u8; 28] = [
        b'V', b'B', b'A', b'N', //
        0x03, // SR index 3 = 48 kHz, sub protocol AUDIO (0x00)
        199,  // 200 samples − 1
        1,    // 2 channels − 1
        0x02, // INT24, reserved bit 0, codec PCM (0x00)
        b's', b'p', b'-', b'p', b'r', b'o', b'g', b'r', b'a', b'm', 0, 0, 0, 0, 0, 0, //
        0x04, 0x03, 0x02, 0x01, // nuFrame, little-endian
    ];
    assert_eq!(h, expected);
    assert_eq!(header(48_000, 0, 1, Int16, "x", 0), None, "0 frames");
    assert_eq!(header(48_000, 257, 1, Int16, "x", 0), None, "257 frames");
    assert_eq!(header(48_000, 1, 257, Int16, "x", 0), None, "257 channels");
    assert_eq!(header(44_000, 1, 1, Int16, "x", 0), None, "not in the SR list");
}

#[test]
fn the_stream_name_is_16_ascii_bytes_zero_padded() {
    // songplayer, verbatim:
    assert_eq!(&stream_name_bytes("sp-program"), b"sp-program\0\0\0\0\0\0");
    assert_eq!(
        &stream_name_bytes("a-very-long-stream-name"),
        b"a-very-long-stre",
        "truncated to 16"
    );
    assert_eq!(
        &stream_name_bytes("é\tx"),
        b"__x\0\0\0\0\0\0\0\0\0\0\0\0\0",
        "non-ASCII and control characters become _"
    );
    assert_eq!(stream_name_bytes(""), [0u8; 16]);
    // the decode side: up to the first zero byte, the same substitution.
    assert_eq!(stream_name_of(b"GUEST_EN\0\0\0\0\0\0\0\0"), "GUEST_EN");
    assert_eq!(stream_name_of(b"a-very-long-stre"), "a-very-long-stre");
    assert_eq!(stream_name_of(b"ab\0cd\0\0\0\0\0\0\0\0\0\0\0"), "ab");
    assert_eq!(
        stream_name_of(&[0xC3, 0xA9, b'x', 0x7F, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
        "__x_"
    );
}

#[test]
fn int24_conversion_is_bit_exact_and_clamps() {
    // songplayer, verbatim.
    assert_eq!(f32_to_int24(0.0), 0);
    assert_eq!(f32_to_int24(1.0), 8_388_607, "+1.0 = +full scale");
    assert_eq!(f32_to_int24(-1.0), -8_388_607, "-1.0 = -full scale");
    assert_eq!(f32_to_int24(1.5), 8_388_607, "> 1.0 clamps");
    assert_eq!(f32_to_int24(-2.0), -8_388_607, "< -1.0 clamps");
    assert_eq!(f32_to_int24(f32::INFINITY), 8_388_607);
    assert_eq!(f32_to_int24(f32::NEG_INFINITY), -8_388_607);
    assert_eq!(f32_to_int24(f32::NAN), 0, "NaN is silence");
    assert_eq!(f32_to_int24(0.5), 4_194_304, "4194303.5 rounds away from 0");
    assert_eq!(f32_to_int24(-0.5), -4_194_304);
    assert_eq!(f32_to_int24(0.25), 2_097_152);
    assert_eq!(f32_to_int24(-0.25), -2_097_152);
    assert_eq!(f32_to_int24(1.0 / 8_388_607.0), 1, "one LSB");
    assert_eq!(int24_le(8_388_607), [0xFF, 0xFF, 0x7F]);
    assert_eq!(int24_le(-8_388_607), [0x01, 0x00, 0x80]);
    assert_eq!(int24_le(-1), [0xFF, 0xFF, 0xFF]);
    assert_eq!(int24_le(0x12_3456), [0x56, 0x34, 0x12]);
}

#[test]
fn int16_and_float32_samples_are_exact() {
    // songplayer vban_packet_tests_format.rs, verbatim.
    assert_eq!(f32_to_int16(1.0), 32_767);
    assert_eq!(f32_to_int16(-1.0), -32_767);
    assert_eq!(f32_to_int16(0.5), 16_384, "16383.5 rounds away from zero");
    assert_eq!(f32_to_int16(-0.25), -8_192);
    assert_eq!(f32_to_int16(2.0), 32_767);
    assert_eq!(f32_to_int16(-2.0), -32_767);
    assert_eq!(f32_to_int16(f32::NAN), 0);
    assert_eq!(clean_f32(1.5), 1.0);
    assert_eq!(clean_f32(-1.5), -1.0);
    assert_eq!(clean_f32(-0.25), -0.25);
    assert_eq!(clean_f32(f32::NAN), 0.0);
    assert_eq!(clean_f32(f32::INFINITY), 0.0);
    assert_eq!(clean_f32(f32::NEG_INFINITY), 0.0);
}

#[test]
fn every_index_of_the_sr_list_decodes_to_its_rate() {
    // songplayer's pinned indices, now from the full rev. 13 list:
    assert_eq!(sr_index_of(48_000), Some(3));
    assert_eq!(sr_index_of(96_000), Some(4));
    assert_eq!(sr_index_of(192_000), Some(5));
    assert_eq!(sr_index_of(44_100), Some(16));
    assert_eq!(sr_index_of(88_200), Some(17));
    assert_eq!(sr_index_of(16_000), Some(8));
    assert_eq!(sr_index_of(11_025), Some(14));
    assert_eq!(sr_index_of(44_000), None);
    for (index, &rate) in VBAN_SAMPLE_RATES.iter().enumerate() {
        let bytes = golden_with(4, index as u8);
        assert_eq!(decode_packet(&bytes).unwrap().format.rate_hz, rate, "index {index}");
    }
    // songplayer's format bits (INT16 0x01, INT24 0x02, FLOAT32 0x04) + INT32:
    let types = [Int16, Int24, Int32, Float32];
    assert_eq!(types.map(VbanSampleType::data_type), [1, 2, 3, 4]);
    assert_eq!(types.map(VbanSampleType::bytes), [2, 3, 4, 4]);
    assert_eq!(VbanSampleType::from_data_type(3), Some(Int32));
    assert_eq!(VbanSampleType::from_data_type(0), None, "BYTE8");
}

struct Geometry {
    rate: u32,
    sample: VbanSampleType,
    frames: usize,
    packets: usize,
    len: usize,
}

const fn g(rate: u32, sample: VbanSampleType, frames: usize, packets: usize, len: usize) -> Geometry {
    Geometry { rate, sample, frames, packets, len }
}

/// songplayer `vban_packet_tests_format.rs` `TABLE`: the packet geometry of one
/// 1/30 s stereo block per rate × format (frames per packet, packets, bytes).
const TABLE: [Geometry; 15] = [
    g(44_100, Int16, 245, 6, 1008),
    g(44_100, Int24, 210, 7, 1288),
    g(44_100, Float32, 147, 10, 1204),
    g(48_000, Int16, 200, 8, 828),
    g(48_000, Int24, 200, 8, 1228),
    g(48_000, Float32, 160, 10, 1308),
    g(88_200, Int16, 245, 12, 1008),
    g(88_200, Int24, 210, 14, 1288),
    g(88_200, Float32, 147, 20, 1204),
    g(96_000, Int16, 200, 16, 828),
    g(96_000, Int24, 200, 16, 1228),
    g(96_000, Float32, 160, 20, 1308),
    g(192_000, Int16, 256, 25, 1052),
    g(192_000, Int24, 200, 32, 1228),
    g(192_000, Float32, 160, 40, 1308),
];

#[test]
fn songplayers_15_geometries_round_trip_through_the_decoder() {
    for t in TABLE {
        let block = ramp(t.rate as usize / 30 * 2);
        let packets = VbanEncoder::new("sp-program", t.rate, 2, t.sample).packets(&block, t.frames);
        assert_eq!(packets.len(), t.packets, "{} {:?}", t.rate, t.sample);
        let mut decoded = Vec::new();
        for (k, bytes) in packets.iter().enumerate() {
            assert_eq!(bytes.len(), t.len, "{} {:?}", t.rate, t.sample);
            let p = decode_packet(bytes).unwrap();
            assert_eq!(
                p.format,
                VbanAudioFormat { rate_hz: t.rate, channels: 2, sample_type: t.sample }
            );
            assert_eq!(usize::from(p.frames), t.frames);
            assert_eq!(p.counter, k as u32);
            decoded.extend(p.samples);
        }
        assert_eq!(decoded.len(), block.len());
        for (i, (&back, &x)) in decoded.iter().zip(&block).enumerate() {
            let same = match t.sample {
                Int16 => f32_to_int16(back) == f32_to_int16(x),
                Int24 => f32_to_int24(back) == f32_to_int24(x),
                Int32 | Float32 => back == clean_f32(x),
            };
            assert!(same, "{} {:?} sample {i}: {back} vs {x}", t.rate, t.sample);
        }
    }
}

#[test]
fn mono_and_stereo_int16_and_int32_round_trip() {
    for channels in [1usize, 2] {
        for sample in [Int16, Int32] {
            let block = ramp(256 * channels);
            let bytes = VbanEncoder::new("GUEST_EN", 48_000, channels, sample).packet(&block).unwrap();
            assert_eq!(bytes.len(), VBAN_HEADER_LEN + 256 * channels * sample.bytes());
            let p = decode_packet(&bytes).unwrap();
            assert_eq!((usize::from(p.format.channels), p.frames), (channels, 256));
            for (&back, &x) in p.samples.iter().zip(&block) {
                match sample {
                    Int16 => assert_eq!(f32_to_int16(back), f32_to_int16(x)),
                    _ => assert!((back - x).abs() <= 1.0e-6, "{back} vs {x}"),
                }
            }
        }
    }
    assert_eq!(f32_to_int32(1.0), i32::MAX);
    assert_eq!(f32_to_int32(-1.0), -i32::MAX);
}

#[test]
fn int24_samples_sign_extend_like_songplayers_oracle() {
    let mut bytes = header(48_000, 5, 1, Int24, "x", 0).unwrap().to_vec();
    for v in [8_388_607, -8_388_607, -1, 0x12_3456] {
        bytes.extend_from_slice(&int24_le(v));
    }
    bytes.extend_from_slice(&[0x00, 0x00, 0x80]); // -8388608, one past full scale
    let p = decode_packet(&bytes).unwrap();
    let back: Vec<i32> = p.samples.iter().map(|&x| f32_to_int24(x)).collect();
    assert_eq!(back, vec![8_388_607, -8_388_607, -1, 0x12_3456, -8_388_607]);
    assert_eq!(p.samples[0], 1.0);
    assert_eq!(p.samples[4], -1.0, "clamped");
}

#[test]
fn every_reject_has_its_type() {
    let golden = hex(GOLDEN);
    let cases = [
        (Vec::new(), VbanReject::NotVban),
        (golden[..27].to_vec(), VbanReject::NotVban),
        (golden_with(3, b'X'), VbanReject::NotVban),
        (golden_with(4, 0x20 | 3), VbanReject::NonAudio(0x20)), // SERIAL
        (golden_with(4, 0x40 | 3), VbanReject::NonAudio(0x40)), // TXT (VB-Matrix)
        (golden_with(4, 0x60 | 3), VbanReject::NonAudio(0x60)), // SERVICE
        (golden_with(4, 0xE0 | 3), VbanReject::NonAudio(0xE0)), // USER
        (golden_with(7, 0x10 | 1), VbanReject::UnsupportedCodec(0x10)), // VBCA
        (golden_with(7, 0x20 | 1), VbanReject::UnsupportedCodec(0x20)), // VBCV
        (golden_with(7, 0xF0 | 1), VbanReject::UnsupportedCodec(0xF0)), // USER
        (golden_with(7, 0x08 | 1), VbanReject::Corrupt),                 // reserved bit
        (golden_with(4, 21), VbanReject::Corrupt),                       // SR index 21
        (golden_with(4, 31), VbanReject::Corrupt),
        (golden_with(7, 0x00), VbanReject::UnsupportedType(0)), // BYTE8
        (golden_with(7, 0x05), VbanReject::UnsupportedType(5)), // FLOAT64
        (golden_with(7, 0x06), VbanReject::UnsupportedType(6)), // 12 bits
        (golden_with(7, 0x07), VbanReject::UnsupportedType(7)), // 10 bits
        (golden[..golden.len() - 1].to_vec(), VbanReject::Corrupt),
        ([golden.as_slice(), &[0u8][..]].concat(), VbanReject::Corrupt),
        (golden_with(5, 4), VbanReject::Corrupt), // 5 frames announced, 4 sent
        (golden_with(6, 1), VbanReject::Corrupt), // 2 channels announced, 1 sent
    ];
    for (bytes, expected) in cases {
        assert_eq!(decode_packet(&bytes), Err(expected), "{bytes:02x?}");
    }
}

#[test]
fn the_checks_run_in_the_spec_order() {
    let mut bytes = hex(GOLDEN);
    bytes[4] = 0x40 | 31; // non-audio AND an SR index past the list
    bytes[7] = 0x18; // VBCA AND the reserved bit
    bytes.push(0); // AND one byte too many
    assert_eq!(decode_packet(&bytes), Err(VbanReject::NonAudio(0x40)));
    bytes[4] = 31;
    assert_eq!(decode_packet(&bytes), Err(VbanReject::UnsupportedCodec(0x10)));
    bytes[7] = 0x09;
    assert_eq!(decode_packet(&bytes), Err(VbanReject::Corrupt), "reserved bit");
    bytes[7] = 0x01;
    assert_eq!(decode_packet(&bytes), Err(VbanReject::Corrupt), "SR index 31");
    bytes[4] = 3;
    bytes[7] = 0x05;
    assert_eq!(decode_packet(&bytes), Err(VbanReject::UnsupportedType(5)));
    bytes[7] = 0x01;
    assert_eq!(decode_packet(&bytes), Err(VbanReject::Corrupt), "size last");
}

#[test]
fn the_frame_counter_wraps_at_u32_max() {
    let mut encoder = VbanEncoder::new("x", 48_000, 1, Int16).starting_at(u32::MAX - 1);
    let counters: Vec<u32> = (0..4)
        .map(|_| decode_packet(&encoder.packet(&[0.0; 8]).unwrap()).unwrap().counter)
        .collect();
    assert_eq!(counters, vec![u32::MAX - 1, u32::MAX, 0, 1]);
    assert_eq!(encoder.next_counter(), 2);
    encoder.skip(3);
    assert_eq!(encoder.next_counter(), 5);
    let mut stereo = VbanEncoder::new("x", 48_000, 2, Int16);
    assert_eq!(stereo.packet(&[0.0; 3]), None, "not whole frames");
    assert_eq!(stereo.next_counter(), 0, "a refused packet does not count");
}

#[test]
fn the_format_serializes_for_the_status_endpoint() {
    let format = VbanAudioFormat { rate_hz: 48_000, channels: 2, sample_type: Int24 };
    assert_eq!(
        serde_json::to_value(format).unwrap(),
        serde_json::json!({"rateHz": 48000, "channels": 2, "sampleType": "int24"})
    );
    assert_eq!(Float32.as_str(), "float32");
    assert_eq!(serde_json::to_value(Int32).unwrap(), serde_json::json!("int32"));
}
```

- [ ] **Step 3: How it is verified.** CI `Test` job (`cargo test --workspace --all-features`). Without Steps 4–5 the module does not compile.

- [ ] **Step 4: Implement the decoder** — `crates/presenter-core/src/audio/vban.rs`:

```rust
//! VBAN audio packets (#852 C1a, spec §5.2): the 28-byte header and the PCM
//! payload. Pure, no I/O.
//!
//! Ported from songplayer `crates/sp-server/src/playback/vban_packet.rs`
//! @ dd551b7d (the production sender to FOH; VB-Audio "VBAN Protocol
//! Specifications" rev. 13): the header layout, the constants, the SR indices
//! and format bits, [`f32_to_int16`] and [`clean_f32`]. Songplayer only SENDS;
//! the decode direction is new, from the spec's reference receive algorithm
//! (p. 10–11), and the sample decode is the sign extension of songplayer's
//! test oracle `parse_packet` without its `assert!`s: a datagram from the
//! network is untrusted, every reject is a typed [`VbanReject`] and nothing
//! here can panic.
//!
//! | bytes | field |
//! |---|---|
//! | 0..4 | `'V' 'B' 'A' 'N'` |
//! | 4 | `format_SR`: bits 0–4 SR index, 5–7 sub-protocol (AUDIO = 0x00) |
//! | 5 | `format_nbs` = frames − 1 |
//! | 6 | `format_nbc` = channels − 1 |
//! | 7 | `format_bit`: bits 0–2 data type, bit 3 reserved (0), 4–7 codec (PCM = 0x00) |
//! | 8..24 | `streamname[16]`, ASCII, zero padded |
//! | 24..28 | `nuFrame`, u32 LE, +1 per packet |
//!
//! The decode scale is songplayer's symmetric encoder scale (INT16 ±32767,
//! INT24 ±8388607, INT32 ±`i32::MAX`), clamped to ±1.0, so a packet of the
//! ported encoder round-trips bit-exactly in the integer domain.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// The four magic bytes every VBAN packet starts with.
pub const VBAN_MAGIC: [u8; 4] = *b"VBAN";
/// The VBAN header length.
pub const VBAN_HEADER_LEN: usize = 28;
/// The stream-name field length.
pub const VBAN_STREAM_NAME_LEN: usize = 16;
/// The spec's data maximum per packet (a conformant sender stays within it).
pub const VBAN_DATA_MAX: usize = 1436;
/// The VBAN default UDP port (spec §5.1).
pub const VBAN_DEFAULT_PORT: u16 = 6980;
/// Full scale of INT16: `±1.0 ↔ ±32767` (songplayer, symmetric).
pub const INT16_FULL_SCALE: i16 = 32_767;
/// Full scale of INT24: `±1.0 ↔ ±8388607` (songplayer, symmetric).
pub const INT24_FULL_SCALE: i32 = 8_388_607;

/// The spec's sample rates (rev. 13 p. 8), indexed by `format_SR & 0x1F`.
pub const VBAN_SAMPLE_RATES: [u32; 21] = [
    6_000, 12_000, 24_000, 48_000, 96_000, 192_000, 384_000, 8_000, 16_000, 32_000, 64_000,
    128_000, 256_000, 512_000, 11_025, 22_050, 44_100, 88_200, 176_400, 352_800, 705_600,
];

const PROTOCOL_MASK: u8 = 0xE0;
const PROTOCOL_AUDIO: u8 = 0x00;
const SR_INDEX_MASK: u8 = 0x1F;
const DATA_TYPE_MASK: u8 = 0x07;
const RESERVED_BIT: u8 = 0x08;
const CODEC_MASK: u8 = 0xF0;
const CODEC_PCM: u8 = 0x00;

/// The PCM sample types Presenter accepts (spec §5.2 step 6); BYTE8, FLOAT64
/// and the 12/10-bit types are refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VbanSampleType {
    Int16,
    Int24,
    Int32,
    Float32,
}

impl VbanSampleType {
    /// The type of a `format_bit & 0x07` data-type index.
    pub fn from_data_type(index: u8) -> Option<Self> {
        match index {
            1 => Some(Self::Int16),
            2 => Some(Self::Int24),
            3 => Some(Self::Int32),
            4 => Some(Self::Float32),
            _ => None,
        }
    }

    /// The `format_bit` data-type index (codec PCM).
    pub fn data_type(self) -> u8 {
        match self {
            Self::Int16 => 1,
            Self::Int24 => 2,
            Self::Int32 => 3,
            Self::Float32 => 4,
        }
    }

    /// Bytes per sample.
    pub fn bytes(self) -> usize {
        match self {
            Self::Int16 => 2,
            Self::Int24 => 3,
            Self::Int32 | Self::Float32 => 4,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Int16 => "int16",
            Self::Int24 => "int24",
            Self::Int32 => "int32",
            Self::Float32 => "float32",
        }
    }

    /// One little-endian sample as ±1.0; `bytes` is exactly [`Self::bytes`]
    /// long (the caller's `chunks_exact`), anything else reads as silence.
    fn decode(self, bytes: &[u8]) -> f32 {
        let scaled = |value: f64, full_scale: f64| (value / full_scale).clamp(-1.0, 1.0) as f32;
        match (self, bytes) {
            (Self::Int16, [a, b]) => scaled(
                f64::from(i16::from_le_bytes([*a, *b])),
                f64::from(INT16_FULL_SCALE),
            ),
            (Self::Int24, [a, b, c]) => {
                let sign = if c & 0x80 != 0 { 0xFF } else { 0x00 };
                scaled(
                    f64::from(i32::from_le_bytes([*a, *b, *c, sign])),
                    f64::from(INT24_FULL_SCALE),
                )
            }
            (Self::Int32, [a, b, c, d]) => scaled(
                f64::from(i32::from_le_bytes([*a, *b, *c, *d])),
                f64::from(i32::MAX),
            ),
            (Self::Float32, [a, b, c, d]) => clean_f32(f32::from_le_bytes([*a, *b, *c, *d])),
            _ => 0.0,
        }
    }
}

/// What a converter must know about a stream. A change of any field resets
/// the mic's converter (spec §5.3); the frames per packet may vary freely.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VbanAudioFormat {
    pub rate_hz: u32,
    pub channels: u16,
    pub sample_type: VbanSampleType,
}

/// One decoded VBAN audio packet (spec §5.2 step 8).
#[derive(Debug, Clone, PartialEq)]
pub struct VbanPacket {
    pub stream_name: String,
    pub counter: u32,
    pub format: VbanAudioFormat,
    /// Frames (one sample per channel): `format_nbs + 1`, 1–256.
    pub frames: u16,
    /// Interleaved samples (`frames × channels`), ±1.0.
    pub samples: Vec<f32>,
}

/// Why a datagram is not usable VBAN audio. Counted per sender in
/// `/listen/status`; the log text is English like every server log line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Error)]
pub enum VbanReject {
    #[error("not a VBAN packet (shorter than the header or no VBAN magic)")]
    NotVban,
    #[error("not VBAN audio (sub-protocol {0:#04x})")]
    NonAudio(u8),
    #[error("unsupported VBAN codec {0:#04x}")]
    UnsupportedCodec(u8),
    #[error("unsupported VBAN sample type {0}")]
    UnsupportedType(u8),
    #[error("corrupt VBAN packet (reserved bit, SR index or size)")]
    Corrupt,
}

/// Decode one datagram with the spec's checks, in its order: magic + length,
/// AUDIO sub-protocol, PCM codec, reserved bit, SR index, sample type, size.
pub fn decode_packet(datagram: &[u8]) -> Result<VbanPacket, VbanReject> {
    let Some((header, payload)) = datagram.split_at_checked(VBAN_HEADER_LEN) else {
        return Err(VbanReject::NotVban);
    };
    if header[0..4] != VBAN_MAGIC {
        return Err(VbanReject::NotVban);
    }
    let (format_sr, nbs, nbc, format_bit) = (header[4], header[5], header[6], header[7]);
    let protocol = format_sr & PROTOCOL_MASK;
    if protocol != PROTOCOL_AUDIO {
        return Err(VbanReject::NonAudio(protocol));
    }
    let codec = format_bit & CODEC_MASK;
    if codec != CODEC_PCM {
        return Err(VbanReject::UnsupportedCodec(codec));
    }
    if format_bit & RESERVED_BIT != 0 {
        return Err(VbanReject::Corrupt);
    }
    let rate_hz = VBAN_SAMPLE_RATES
        .get(usize::from(format_sr & SR_INDEX_MASK))
        .copied()
        .ok_or(VbanReject::Corrupt)?;
    let data_type = format_bit & DATA_TYPE_MASK;
    let sample_type =
        VbanSampleType::from_data_type(data_type).ok_or(VbanReject::UnsupportedType(data_type))?;
    let frames = u16::from(nbs) + 1;
    let channels = u16::from(nbc) + 1;
    if payload.len() != usize::from(frames) * usize::from(channels) * sample_type.bytes() {
        return Err(VbanReject::Corrupt);
    }
    Ok(VbanPacket {
        stream_name: stream_name_of(&header[8..24]),
        counter: u32::from_le_bytes([header[24], header[25], header[26], header[27]]),
        format: VbanAudioFormat { rate_hz, channels, sample_type },
        frames,
        samples: payload
            .chunks_exact(sample_type.bytes())
            .map(|bytes| sample_type.decode(bytes))
            .collect(),
    })
}

/// A header's stream name: up to the first zero byte; a byte that is not
/// printable ASCII reads as `_` (the substitution songplayer's
/// `stream_name_bytes` writes).
pub fn stream_name_of(field: &[u8]) -> String {
    field
        .iter()
        .take_while(|&&byte| byte != 0)
        .map(|&byte| if (0x20..0x7F).contains(&byte) { char::from(byte) } else { '_' })
        .collect()
}

/// One f32 sample as INT16 (songplayer, verbatim): clamped, scaled by
/// [`INT16_FULL_SCALE`], rounded half away from zero; NaN is silence.
pub fn f32_to_int16(x: f32) -> i16 {
    (f64::from(x).clamp(-1.0, 1.0) * f64::from(INT16_FULL_SCALE)).round() as i16
}

/// One f32 sample as VBAN FLOAT32 carries it (songplayer, verbatim): clamped
/// to ±1, a non-finite value as silence.
pub fn clean_f32(x: f32) -> f32 {
    if x.is_finite() {
        x.clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests;
```

- [ ] **Step 5: Port the encoder** — `crates/presenter-core/src/audio/vban_encode.rs`:

```rust
//! The VBAN packet ENCODER, test-only (#852 C1a): the packet generator for the
//! decoder's round trips and for presenter-server's real-UDP receiver tests
//! (its dev-dependency enables `test-helpers`). Never in a production binary.
//!
//! Ported from songplayer `crates/sp-server/src/playback/vban_packet.rs`
//! @ dd551b7d: `stream_name_bytes`, `f32_to_int24`, `int24_le` (verbatim),
//! `sr_index_of` (widened from songplayer's 5 rates to the spec's list),
//! `write_header_as` (widened to any 1–256 frames × 1–256 channels: [`header`])
//! and `VbanEncoder`'s counter (+1 per packet, wrapping at `u32::MAX`).
//! Songplayer's 30 fps grid geometry and packet schedule are sender pacing and
//! are not ported. Written panic-free: the no-panic gate counts this file.

use super::vban::{
    clean_f32, f32_to_int16, VbanSampleType, INT24_FULL_SCALE, VBAN_HEADER_LEN, VBAN_MAGIC,
    VBAN_SAMPLE_RATES, VBAN_STREAM_NAME_LEN,
};

/// The stream-name field for `name` (songplayer, verbatim): its first 16
/// characters as ASCII (any non-ASCII or control character becomes `_`),
/// zero padded.
pub fn stream_name_bytes(name: &str) -> [u8; VBAN_STREAM_NAME_LEN] {
    let mut out = [0u8; VBAN_STREAM_NAME_LEN];
    for (dst, c) in out.iter_mut().zip(name.chars()) {
        *dst = if c.is_ascii() && !c.is_ascii_control() { c as u8 } else { b'_' };
    }
    out
}

/// One f32 sample as a 24-bit integer (songplayer, verbatim).
pub fn f32_to_int24(x: f32) -> i32 {
    let clamped = f64::from(x).clamp(-1.0, 1.0);
    (clamped * f64::from(INT24_FULL_SCALE)).round() as i32
}

/// The 3 little-endian bytes of a 24-bit two's-complement sample (songplayer).
pub fn int24_le(v: i32) -> [u8; 3] {
    let b = v.to_le_bytes();
    [b[0], b[1], b[2]]
}

/// One f32 sample as INT32: clamped, scaled by `i32::MAX`, rounded.
pub fn f32_to_int32(x: f32) -> i32 {
    (f64::from(x).clamp(-1.0, 1.0) * f64::from(i32::MAX)).round() as i32
}

/// The SR index of a rate (songplayer `sr_index_of`, now the full list).
pub fn sr_index_of(rate_hz: u32) -> Option<u8> {
    VBAN_SAMPLE_RATES
        .iter()
        .position(|&rate| rate == rate_hz)
        .and_then(|index| u8::try_from(index).ok())
}

/// The 28-byte header (songplayer `write_header_as`, any geometry); `None`
/// when it does not fit a header (0 or more than 256 frames or channels, a
/// rate outside the SR list).
pub fn header(
    rate_hz: u32,
    frames: usize,
    channels: usize,
    sample_type: VbanSampleType,
    name: &str,
    counter: u32,
) -> Option<[u8; VBAN_HEADER_LEN]> {
    let sr_index = sr_index_of(rate_hz)?;
    let nbs = u8::try_from(frames.checked_sub(1)?).ok()?;
    let nbc = u8::try_from(channels.checked_sub(1)?).ok()?;
    let mut out = [0u8; VBAN_HEADER_LEN];
    out[0..4].copy_from_slice(&VBAN_MAGIC);
    out[4] = sr_index;
    out[5] = nbs;
    out[6] = nbc;
    out[7] = sample_type.data_type();
    out[8..24].copy_from_slice(&stream_name_bytes(name));
    out[24..28].copy_from_slice(&counter.to_le_bytes());
    Some(out)
}

/// Append one sample in `sample_type` (songplayer `write_sample` + INT32).
pub fn write_sample(sample_type: VbanSampleType, x: f32, out: &mut Vec<u8>) {
    match sample_type {
        VbanSampleType::Int16 => out.extend_from_slice(&f32_to_int16(x).to_le_bytes()),
        VbanSampleType::Int24 => out.extend_from_slice(&int24_le(f32_to_int24(x))),
        VbanSampleType::Int32 => out.extend_from_slice(&f32_to_int32(x).to_le_bytes()),
        VbanSampleType::Float32 => out.extend_from_slice(&clean_f32(x).to_le_bytes()),
    }
}

/// The encoder of ONE stream (songplayer `VbanEncoder`): it owns the frame
/// counter, which grows by exactly 1 per packet and wraps at `u32::MAX`.
#[derive(Debug, Clone)]
pub struct VbanEncoder {
    name: String,
    rate_hz: u32,
    channels: usize,
    sample_type: VbanSampleType,
    counter: u32,
}

impl VbanEncoder {
    pub fn new(name: &str, rate_hz: u32, channels: usize, sample_type: VbanSampleType) -> Self {
        Self { name: name.to_string(), rate_hz, channels, sample_type, counter: 0 }
    }

    /// Start the counter at `counter` (the wrap and second-sender tests).
    pub fn starting_at(mut self, counter: u32) -> Self {
        self.counter = counter;
        self
    }

    /// The `nuFrame` the next packet carries.
    pub fn next_counter(&self) -> u32 {
        self.counter
    }

    /// Leave `n` packets out (a lost-packet test): the counter moves, nothing is sent.
    pub fn skip(&mut self, n: u32) {
        self.counter = self.counter.wrapping_add(n);
    }

    /// One packet of interleaved `samples` (1–256 whole frames); `None`, and no
    /// counter step, otherwise.
    pub fn packet(&mut self, samples: &[f32]) -> Option<Vec<u8>> {
        if self.channels == 0 || samples.len() % self.channels != 0 {
            return None;
        }
        let frames = samples.len() / self.channels;
        let head = header(self.rate_hz, frames, self.channels, self.sample_type, &self.name, self.counter)?;
        let mut out = Vec::with_capacity(VBAN_HEADER_LEN + samples.len() * self.sample_type.bytes());
        out.extend_from_slice(&head);
        for &x in samples {
            write_sample(self.sample_type, x, &mut out);
        }
        self.counter = self.counter.wrapping_add(1);
        Some(out)
    }

    /// A block split into packets of `frames_per_packet` frames (the last may be shorter).
    pub fn packets(&mut self, samples: &[f32], frames_per_packet: usize) -> Vec<Vec<u8>> {
        let step = frames_per_packet.max(1) * self.channels.max(1);
        samples.chunks(step).filter_map(|chunk| self.packet(chunk)).collect()
    }
}

/// `frames` interleaved frames of a sine from `start_frame` on, the same value
/// in every channel.
pub fn tone(
    rate_hz: u32,
    hz: f64,
    amplitude: f64,
    start_frame: usize,
    frames: usize,
    channels: usize,
) -> Vec<f32> {
    (start_frame..start_frame + frames)
        .flat_map(|n| {
            let phase = std::f64::consts::TAU * hz * n as f64 / f64::from(rate_hz);
            std::iter::repeat_n((amplitude * phase.sin()) as f32, channels)
        })
        .collect()
}

/// songplayer's `ramp_block`, any length: distinct-ish values over `[-1, 1)`.
pub fn ramp(samples: usize) -> Vec<f32> {
    (0..samples).map(|i| (i % 2000) as f32 / 1000.0 - 1.0).collect()
}
```

- [ ] **Step 6: Keep the test-only encoder out of mutants (review 1h).** In `.cargo/mutants.toml`, `exclude_globs` gains, after `"crates/presenter-server/src/companion/protocol.rs",`:

```toml
    # #852 C1a: the ported songplayer encoder is a test-only packet generator
    # (cfg(any(test, feature = "test-helpers"))), never production behaviour.
    "crates/presenter-core/src/audio/vban_encode.rs",
```

- [ ] **Step 7: Local gates**

```bash
cd "$REPO" && cargo fmt --all && cargo fmt --all -- --check
for f in crates/presenter-core/src/audio/vban.rs crates/presenter-core/src/audio/vban_encode.rs; do bash scripts/dev/count_prod_lines.sh "$f"; done   # ≈ 200, ≈ 115
QC_TARGETS="$(printf '%s\n' crates/presenter-core/src/audio/vban.rs crates/presenter-core/src/audio/vban_encode.rs)" python3 scripts/dev/fn_length_check.py .
python3 scripts/dev/no_panic_check.py .
```

Expected: no fmt diff, `"violations": []`, the no-panic gate clean (the encoder is counted and has no `unwrap` / `expect` / `panic!`).

- [ ] **Step 8: Commit** — paths `crates/presenter-core/Cargo.toml crates/presenter-core/src/lib.rs crates/presenter-core/src/audio/ .cargo/mutants.toml`, message `feat(core): VBAN decode with typed rejects + the ported songplayer encoder for tests (#852 C1a-1)` + a body line naming songplayer `vban_packet.rs` @ dd551b7d + Co-Authored-By.

### Task 2: The level meter

**Files:**
- Modify: `crates/presenter-core/src/audio/mod.rs` (add `pub mod level;`)
- Create: `crates/presenter-core/src/audio/level.rs` (tests inline)

**Interfaces:**
- Produces: `presenter_core::audio::level::{SILENCE_DBFS = -120.0, VOICED_DBFS = -50.0, rms_dbfs(&[i16]) -> f32, is_voiced(f32) -> bool}`. Lane 2 does not call it (no audio is converted yet); it is `pub` core API, used by lane 3's chain.

- [ ] **Step 1: The Python mirror** of songplayer's `pcm_dbfs` (pins the values the tests use; run once, keep the output in the PR description):

```bash
python3 - <<'EOF'
import math
def dbfs(s):
    rms = math.sqrt(sum((x / 32768) ** 2 for x in s) / len(s)) if s else 0
    return round(max(20 * math.log10(rms), -120.0), 2) if rms > 0 else -120.0
def i16(x):
    v = max(-1.0, min(1.0, x)) * 32767
    return int(math.floor(v + 0.5)) if v >= 0 else -int(math.floor(-v + 0.5))
print(dbfs([3000, -3000] * 50), dbfs([30, -30] * 50), dbfs([32767, -32767]), dbfs([0] * 8))
print("1 kHz, 0.5, 16 kHz:", dbfs([i16(0.5 * math.sin(2 * math.pi * 1000 * n / 16000)) for n in range(1600)]))
EOF
```

Expected: `-20.77 -60.77 -0.0 -120.0`, `-9.03`.

- [ ] **Step 2: Implement + test the level meter** — `crates/presenter-core/src/audio/level.rs`:

```rust
//! A frame's level (#852 C1a, spec §5.4): the RMS of 16 kHz s16 PCM — what the
//! ASR gets — in dBFS. songplayer's `pcm_dbfs` (`scripts/dub_live_session.py`
//! @ dd551b7d, also `eval/dubbing/live_translate_continuous_probe.py`):
//! samples / 32768, floored at −120 dBFS (JSON has no −inf), rounded to
//! 0.01 dB; "voiced" is above −50 dBFS (songplayer's `VOICED_DBFS`, the
//! silence gate of spec §6.4).

/// The level of digital silence (songplayer `SILENCE_DBFS`).
pub const SILENCE_DBFS: f32 = -120.0;
/// Above this a frame is speech (songplayer `VOICED_DBFS`).
pub const VOICED_DBFS: f32 = -50.0;

/// RMS level of mono s16 samples in dBFS.
pub fn rms_dbfs(samples: &[i16]) -> f32 {
    if samples.is_empty() {
        return SILENCE_DBFS;
    }
    let energy: f64 = samples
        .iter()
        .map(|&s| {
            let w = f64::from(s) / 32_768.0;
            w * w
        })
        .sum();
    let rms = (energy / samples.len() as f64).sqrt();
    if rms <= 0.0 {
        return SILENCE_DBFS;
    }
    let db = (20.0 * rms.log10()).max(f64::from(SILENCE_DBFS));
    ((db * 100.0).round() / 100.0) as f32
}

/// songplayer: `db > VOICED_DBFS`.
pub fn is_voiced(dbfs: f32) -> bool {
    dbfs > VOICED_DBFS
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::vban::f32_to_int16;

    #[test]
    fn the_levels_match_songplayers_pcm_dbfs() {
        // the Python mirror (Task 2 Step 1) of songplayer's pcm_dbfs:
        assert_eq!(rms_dbfs(&[3000i16, -3000].repeat(50)), -20.77);
        assert_eq!(rms_dbfs(&[30i16, -30].repeat(50)), -60.77);
        assert_eq!(rms_dbfs(&[32_767, -32_767]), 0.0);
        assert_eq!(rms_dbfs(&[0; 8]), SILENCE_DBFS);
        assert_eq!(rms_dbfs(&[]), SILENCE_DBFS);
    }

    #[test]
    fn a_1khz_tone_at_half_scale_is_minus_9_dbfs() {
        let tone: Vec<i16> = (0..1600)
            .map(|n| {
                let x = 0.5 * (std::f64::consts::TAU * 1000.0 * f64::from(n) / 16_000.0).sin();
                f32_to_int16(x as f32)
            })
            .collect();
        assert_eq!(rms_dbfs(&tone), -9.03);
    }

    #[test]
    fn voiced_is_strictly_above_minus_50() {
        assert!(is_voiced(-49.99));
        assert!(!is_voiced(-50.0));
        assert!(!is_voiced(SILENCE_DBFS));
    }
}
```

`crates/presenter-core/src/audio/mod.rs` now reads:

```rust
pub mod level;
pub mod vban;
#[cfg(any(test, feature = "test-helpers"))]
pub mod vban_encode;
```

(keep the module doc comment above the list).


- [ ] **Step 3: How it is verified.** CI `Test` job; the values come from the Step 1 mirror.

- [ ] **Step 4: Gates + commit.** `cargo fmt --all -- --check`; `QC_TARGETS` with `level.rs`; commit paths `crates/presenter-core/src/audio/`, message `feat(core): songplayer's frame level for the listening chain (#852 C1a-1)`.

### Task 3: The microphone config and the status DTOs

**Files:**
- Modify: `crates/presenter-core/src/lib.rs:19` (insert `pub mod listen;` after `pub mod library;`)
- Create: `crates/presenter-core/src/listen/mod.rs`, `crates/presenter-core/src/listen/mics.rs`, `crates/presenter-core/src/listen/status.rs` (tests inline)

**Interfaces:**
- Produces (`presenter_core::listen::mics::…`): `LISTEN_MICS_KEY = "listen.mics"`, `LISTEN_MAX_MICS = 4`, `MIC_ID_MAX_CHARS = 20`, `MIC_NAME_MAX_CHARS = 40`, `MIC_STREAM_NAME_MAX_CHARS = 16`; `enum MicChannel { Left, Right, Mix }`, `enum MicLanguage { En, Sk, Off }`, `enum MicRole { Guest, Interpreter, Preacher }` (serde lowercase); `struct MicConfig { id, name, stream_name, sender_ip: Option<String>, channel, language, role }` (camelCase; `fn sender_ip_addr(&self) -> Option<IpAddr>` — the server parses it once per saved list, never per packet); `enum MicsValidationError` (Slovak `Display`); `fn validate_mics(Vec<MicConfig>) -> Result<Vec<MicConfig>, MicsValidationError>`.
- Produces (`presenter_core::listen::status::…`): `enum SocketState { NotStarted, Idle, Bound, Retrying }`, `struct SocketStatus { state, bind_addr: Option<String>, local_addr: Option<String>, error: Option<String> }`, `enum MicState { Waiting, Receiving, NoAudio }` (serde snake_case; lane 3 adds `Error`), `struct MicStatus { #[serde(flatten)] config: MicConfig, state, sender, format: Option<VbanAudioFormat>, frames_per_packet: Option<u16>, packets, last_packet_age_ms: Option<u64> }` (lane 3 adds `packets_per_sec`, the counter-rule and chain fields — each lane ships only what it computes), `struct UnmatchedStream { stream_name, sender, format, packets, last_packet_age_ms }`, `struct ListenStatus { socket, mics, unmatched }` (all camelCase; lane 3 adds `rejected: Vec<RejectedSender>`).

- [ ] **Step 1: Write the failing tests.** At the end of `crates/presenter-core/src/listen/mics.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// The spec §5.6 example, verbatim.
    const SPEC_EXAMPLE: &str = r#"[{ "id": "guest", "name": "Hosť", "streamName": "GUEST_EN", "senderIp": null, "channel": "left",
   "language": "en", "role": "guest" },
 { "id": "interp", "name": "Tlmočník", "streamName": "INTERP_SK", "senderIp": null, "channel": "left",
   "language": "sk", "role": "interpreter" }]"#;

    fn mic(id: &str, stream: &str, ip: Option<&str>) -> MicConfig {
        MicConfig {
            id: id.to_string(),
            name: id.to_string(),
            stream_name: stream.to_string(),
            sender_ip: ip.map(str::to_string),
            channel: MicChannel::Left,
            language: MicLanguage::En,
            role: MicRole::Guest,
        }
    }

    #[test]
    fn the_spec_example_is_valid_and_round_trips() {
        let parsed: Vec<MicConfig> = serde_json::from_str(SPEC_EXAMPLE).unwrap();
        let mics = validate_mics(parsed).unwrap();
        assert_eq!(mics.len(), 2);
        assert_eq!(
            (mics[1].language, mics[1].role, mics[1].channel),
            (MicLanguage::Sk, MicRole::Interpreter, MicChannel::Left)
        );
        let back = serde_json::to_value(&mics).unwrap();
        let spec: serde_json::Value = serde_json::from_str(SPEC_EXAMPLE).unwrap();
        assert_eq!(back, spec);
        assert_eq!(validate_mics(Vec::new()), Ok(Vec::new()), "an empty list = listening idle");
    }

    #[test]
    fn names_and_sender_ips_are_normalized() {
        let mut raw = mic("guest", "GUEST_EN", Some(" 192.0.2.10 "));
        raw.name = "  Hosť ".to_string();
        let mut v6 = mic("interp", "INTERP_SK", Some("2001:db8:0:0:0:0:0:1"));
        v6.name = "Tlmočník".to_string();
        let mut any = mic("pastor", "PASTOR", Some("   "));
        any.role = MicRole::Preacher;
        let mics = validate_mics(vec![raw, v6, any]).unwrap();
        assert_eq!(mics[0].name, "Hosť");
        assert_eq!(mics[0].sender_ip.as_deref(), Some("192.0.2.10"));
        assert_eq!(mics[1].sender_ip.as_deref(), Some("2001:db8::1"));
        assert_eq!(mics[2].sender_ip, None, "blank = any sender");
    }

    #[test]
    fn each_rule_has_its_own_refusal() {
        use MicsValidationError::*;
        let long = |n: usize| "a".repeat(n);
        let named = |name: &str| MicConfig { name: name.to_string(), ..mic("guest", "GUEST_EN", None) };
        let cases = [
            ((0..5).map(|k| mic(&format!("m{k}"), &format!("S{k}"), None)).collect(), TooMany),
            (vec![mic("Guest", "GUEST_EN", None)], BadId("Guest".into())),
            (vec![mic("", "GUEST_EN", None)], BadId(String::new())),
            (vec![mic(&long(21), "GUEST_EN", None)], BadId(long(21))),
            (vec![mic("a_b", "GUEST_EN", None)], BadId("a_b".into())),
            (vec![mic("guest", "A", None), mic("guest", "B", None)], DuplicateId("guest".into())),
            (vec![named("   ")], BadName("guest".into())),
            (vec![named(&"ž".repeat(41))], BadName("guest".into())),
            (vec![named("a\u{7}b")], BadName("guest".into())),
            (vec![mic("guest", "", None)], BadStreamName("guest".into())),
            (vec![mic("guest", "ABCDEFGHIJKLMNOPQ", None)], BadStreamName("guest".into())),
            (vec![mic("guest", "HOSŤ", None)], BadStreamName("guest".into())),
            (vec![mic("guest", " GUEST", None)], BadStreamName("guest".into())),
            (vec![mic("guest", "GUEST_EN", Some("10.0.0"))], BadSenderIp("guest".into())),
            (
                vec![mic("guest", "GUEST_EN", None), mic("interp", "GUEST_EN", Some("192.0.2.10"))],
                Ambiguous("guest".into(), "interp".into()),
            ),
            (
                vec![
                    mic("guest", "GUEST_EN", Some("192.0.2.10")),
                    mic("interp", "GUEST_EN", Some("192.0.2.10")),
                ],
                Ambiguous("guest".into(), "interp".into()),
            ),
        ];
        for (mics, expected) in cases {
            assert_eq!(validate_mics(mics.clone()), Err(expected), "{mics:?}");
        }
        let two_senders = vec![
            mic("guest", "GUEST_EN", Some("192.0.2.10")),
            mic("interp", "GUEST_EN", Some("198.51.100.7")),
        ];
        assert!(validate_mics(two_senders).is_ok(), "one name from two fixed senders is fine");
        assert!(validate_mics(vec![named(&"ž".repeat(40))]).is_ok(), "40 chars, not bytes");
    }

    #[test]
    fn the_messages_are_slovak_and_name_the_mic() {
        assert_eq!(MicsValidationError::TooMany.to_string(), "Najviac 4 mikrofóny.");
        let text = MicsValidationError::BadStreamName("guest".into()).to_string();
        assert!(text.contains("„guest“") && text.contains("16"), "{text}");
        let text = MicsValidationError::Ambiguous("guest".into(), "interp".into()).to_string();
        assert!(text.contains("„guest“") && text.contains("„interp“"), "{text}");
    }

    #[test]
    fn the_sender_ip_of_a_validated_list_parses() {
        let mics = validate_mics(vec![
            mic("guest", "GUEST_EN", Some(" 192.0.2.10 ")),
            mic("interp", "INTERP_SK", None),
        ])
        .unwrap();
        assert_eq!(mics[0].sender_ip_addr(), Some("192.0.2.10".parse().unwrap()));
        assert_eq!(mics[1].sender_ip_addr(), None);
    }

    #[test]
    fn unknown_enum_values_are_refused_by_serde() {
        let json = SPEC_EXAMPLE.replacen(r#""channel": "left""#, r#""channel": "center""#, 1);
        assert!(serde_json::from_str::<Vec<MicConfig>>(&json).is_err());
    }
}
```

At the end of `crates/presenter-core/src/listen/status.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::vban::VbanSampleType;
    use crate::listen::mics::{MicChannel, MicLanguage, MicRole};
    use serde_json::json;

    #[test]
    fn the_status_wire_shape_is_exact() {
        let status = ListenStatus {
            socket: SocketStatus {
                state: SocketState::Bound,
                bind_addr: Some("0.0.0.0:6980".into()),
                local_addr: Some("0.0.0.0:6980".into()),
                error: None,
            },
            mics: vec![MicStatus {
                config: MicConfig {
                    id: "guest".into(),
                    name: "Hosť".into(),
                    stream_name: "GUEST_EN".into(),
                    sender_ip: None,
                    channel: MicChannel::Left,
                    language: MicLanguage::En,
                    role: MicRole::Guest,
                },
                state: MicState::NoAudio,
                sender: Some("192.0.2.10:6980".into()),
                format: Some(VbanAudioFormat { rate_hz: 48_000, channels: 1, sample_type: VbanSampleType::Int16 }),
                frames_per_packet: Some(256),
                packets: 936,
                last_packet_age_ms: Some(3_200),
            }],
            unmatched: vec![UnmatchedStream {
                stream_name: "Stream1".into(),
                sender: "198.51.100.7:6980".into(),
                format: VbanAudioFormat { rate_hz: 48_000, channels: 2, sample_type: VbanSampleType::Int24 },
                packets: 3,
                last_packet_age_ms: 40,
            }],
        };
        let wire = serde_json::to_value(&status).unwrap();
        assert_eq!(
            wire,
            json!({
                "socket": {"state": "bound", "bindAddr": "0.0.0.0:6980", "localAddr": "0.0.0.0:6980", "error": null},
                "mics": [{
                    "id": "guest", "name": "Hosť", "streamName": "GUEST_EN", "senderIp": null,
                    "channel": "left", "language": "en", "role": "guest",
                    "state": "no_audio", "sender": "192.0.2.10:6980",
                    "format": {"rateHz": 48000, "channels": 1, "sampleType": "int16"},
                    "framesPerPacket": 256, "packets": 936,
                    "lastPacketAgeMs": 3200
                }],
                "unmatched": [{"streamName": "Stream1", "sender": "198.51.100.7:6980",
                    "format": {"rateHz": 48000, "channels": 2, "sampleType": "int24"},
                    "packets": 3, "lastPacketAgeMs": 40}]
            })
        );
        let back: ListenStatus = serde_json::from_value(wire).unwrap();
        assert_eq!(back, status);
        assert_eq!(serde_json::to_value(SocketState::NotStarted).unwrap(), json!("not_started"));
    }
}
```

- [ ] **Step 2: How it is verified.** CI `Test` job.

- [ ] **Step 3: Implement.** `crates/presenter-core/src/listen/mod.rs`:

```rust
//! Sermon listening (#852 / #850 part C): the shared config and status types.
//! C1a: the microphones ([`mics`]) and `GET /listen/status` ([`status`]).

pub mod mics;
pub mod status;
```

`crates/presenter-core/src/listen/mics.rs` (before its test block):

```rust
//! The microphones (#852 C1a, spec §5.6): the `app_settings` row
//! `listen.mics`, a JSON list edited through `GET|PUT /listen/mics` (the C1b
//! Settings card). An empty list = listening idle, no socket (PP, C-S8).
//!
//! A mic is selected by its VBAN stream name (exact, 1–16 printable ASCII) and
//! optionally the sender IP; two mics that could both receive one packet are
//! refused. The server is the only validator.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::net::IpAddr;
use thiserror::Error;

/// The `app_settings` key of the list.
pub const LISTEN_MICS_KEY: &str = "listen.mics";
/// Two are planned (guest + interpreter); room for a spare and a preacher.
pub const LISTEN_MAX_MICS: usize = 4;
pub const MIC_ID_MAX_CHARS: usize = 20;
pub const MIC_NAME_MAX_CHARS: usize = 40;
/// = the VBAN stream-name field.
pub const MIC_STREAM_NAME_MAX_CHARS: usize = 16;

/// Which channel of the stream the mic is (spec §5.4): `left` = channel 1,
/// `right` = channel 2 (channel 1 of a mono stream), `mix` = the mean of all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MicChannel {
    Left,
    Right,
    Mix,
}

/// What is spoken into the mic (owner: per mic); `off` = not transcribed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MicLanguage {
    En,
    Sk,
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MicRole {
    Guest,
    Interpreter,
    Preacher,
}

/// One microphone of `listen.mics`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MicConfig {
    pub id: String,
    pub name: String,
    pub stream_name: String,
    /// `None` = any sender.
    #[serde(default)]
    pub sender_ip: Option<String>,
    pub channel: MicChannel,
    pub language: MicLanguage,
    pub role: MicRole,
}

impl MicConfig {
    /// The sender IP (a validated list holds only valid ones). The receiver
    /// calls it once per saved list, never per packet.
    pub fn sender_ip_addr(&self) -> Option<IpAddr> {
        self.sender_ip.as_deref().and_then(|ip| ip.parse().ok())
    }
}

/// Why a list was refused (HTTP 422, shown as it is).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MicsValidationError {
    #[error("Najviac 4 mikrofóny.")]
    TooMany,
    #[error("Identifikátor mikrofónu „{0}“ môže mať 1–20 znakov a–z, 0–9 alebo „-“.")]
    BadId(String),
    #[error("Identifikátor „{0}“ je použitý dvakrát.")]
    DuplicateId(String),
    #[error("Mikrofón „{0}“: názov môže mať 1–40 znakov.")]
    BadName(String),
    #[error("Mikrofón „{0}“: názov VBAN streamu môže mať 1–16 znakov ASCII bez medzery na kraji.")]
    BadStreamName(String),
    #[error("Mikrofón „{0}“: neplatná IP adresa odosielateľa.")]
    BadSenderIp(String),
    #[error("Mikrofóny „{0}“ a „{1}“ by dostávali ten istý VBAN stream.")]
    Ambiguous(String, String),
}

/// Validate and normalize a whole list: names trimmed, the sender IP in its
/// canonical form, a blank IP = any sender.
pub fn validate_mics(mics: Vec<MicConfig>) -> Result<Vec<MicConfig>, MicsValidationError> {
    if mics.len() > LISTEN_MAX_MICS {
        return Err(MicsValidationError::TooMany);
    }
    let mut ids = HashSet::new();
    let mut valid: Vec<MicConfig> = Vec::with_capacity(mics.len());
    for mic in mics {
        let mic = normalize_mic(mic)?;
        if !ids.insert(mic.id.clone()) {
            return Err(MicsValidationError::DuplicateId(mic.id));
        }
        if let Some(other) = valid.iter().find(|other| streams_overlap(other, &mic)) {
            return Err(MicsValidationError::Ambiguous(other.id.clone(), mic.id));
        }
        valid.push(mic);
    }
    Ok(valid)
}

fn normalize_mic(mut mic: MicConfig) -> Result<MicConfig, MicsValidationError> {
    if !is_valid_mic_id(&mic.id) {
        return Err(MicsValidationError::BadId(mic.id));
    }
    let name = mic.name.trim();
    let chars = name.chars().count();
    if chars == 0 || chars > MIC_NAME_MAX_CHARS || name.chars().any(char::is_control) {
        return Err(MicsValidationError::BadName(mic.id));
    }
    mic.name = name.to_string();
    if !is_valid_stream_name(&mic.stream_name) {
        return Err(MicsValidationError::BadStreamName(mic.id));
    }
    let sender_ip = match mic.sender_ip.as_deref().map(str::trim).filter(|ip| !ip.is_empty()) {
        None => None,
        Some(raw) => match raw.parse::<IpAddr>() {
            Ok(ip) => Some(ip.to_string()),
            Err(_) => return Err(MicsValidationError::BadSenderIp(mic.id)),
        },
    };
    mic.sender_ip = sender_ip;
    Ok(mic)
}

fn is_valid_mic_id(id: &str) -> bool {
    (1..=MIC_ID_MAX_CHARS).contains(&id.len())
        && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn is_valid_stream_name(name: &str) -> bool {
    (1..=MIC_STREAM_NAME_MAX_CHARS).contains(&name.len())
        && name.bytes().all(|b| (0x20..0x7F).contains(&b))
        && name.trim() == name
}

/// Whether one packet could match both mics.
fn streams_overlap(a: &MicConfig, b: &MicConfig) -> bool {
    a.stream_name == b.stream_name
        && match (&a.sender_ip, &b.sender_ip) {
            (Some(x), Some(y)) => x == y,
            _ => true,
        }
}
```

(Borrow note for the reviewer: `Err(_) => return Err(…BadSenderIp(mic.id))` moves `mic.id` while `raw` still borrows `mic.sender_ip` — two different fields, accepted by the borrow checker.)

`crates/presenter-core/src/listen/status.rs` (before its test block):

```rust
//! `GET /listen/status` (#852 C1a, spec §5.4 "Health", §14): the socket, each
//! configured mic's counters, the streams seen but not used (they help the
//! first setup); lane C1a-3 adds the refused senders and its counters. Read by the
//! C1b Settings card and the box acceptance (§5.8: counters read before and
//! after a capture and diffed).

use crate::audio::vban::VbanAudioFormat;
use crate::listen::mics::MicConfig;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SocketState {
    /// The receiver was never started (validate mode, tests).
    NotStarted,
    /// No mic configured: no socket (C-S8).
    Idle,
    Bound,
    /// Binding or receiving failed; retried with a growing pause.
    Retrying,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SocketStatus {
    pub state: SocketState,
    pub bind_addr: Option<String>,
    pub local_addr: Option<String>,
    /// Slovak, for the operator.
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MicState {
    /// Configured, no packet yet.
    Waiting,
    Receiving,
    /// No packet for 3 s ("žiadny zvuk", C-S1).
    NoAudio,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MicStatus {
    #[serde(flatten)]
    pub config: MicConfig,
    pub state: MicState,
    /// `ip:port` of the mic's sender (from C1a-3: the one it is locked to).
    pub sender: Option<String>,
    pub format: Option<VbanAudioFormat>,
    pub frames_per_packet: Option<u16>,
    pub packets: u64,
    pub last_packet_age_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnmatchedStream {
    pub stream_name: String,
    pub sender: String,
    pub format: VbanAudioFormat,
    pub packets: u64,
    pub last_packet_age_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListenStatus {
    pub socket: SocketStatus,
    pub mics: Vec<MicStatus>,
    pub unmatched: Vec<UnmatchedStream>,
}
```

In `crates/presenter-core/src/lib.rs`, after `pub mod library;`:

```rust
pub mod listen;
```

- [ ] **Step 4: Gates + commit.** `cargo fmt --all -- --check`; `count_prod_lines.sh` on `mics.rs` (≈ 135) and `status.rs` (≈ 85); `QC_TARGETS` both. Commit paths `crates/presenter-core/src/lib.rs crates/presenter-core/src/listen/`, message `feat(core): listen.mics config with validation and the /listen/status DTOs (#852 C1a-1)`.

### Task 4: Close lane C1a-1 — gates, push, CI

Lane 1 adds pure core code only (no caller in the server yet, all `pub` API), so there is nothing to verify on a box; it lands on `dev` and rides the next release.

- [ ] **Step 1: Measure the lane** (raw production lines, the review's budget ≤ ~700):

```bash
cd "$REPO"
for f in crates/presenter-core/src/audio/vban.rs crates/presenter-core/src/audio/level.rs crates/presenter-core/src/listen/mics.rs crates/presenter-core/src/listen/status.rs crates/presenter-core/src/audio/mod.rs crates/presenter-core/src/listen/mod.rs; do printf '%s ' "$f"; bash scripts/dev/count_prod_lines.sh "$f"; done
bash scripts/dev/count_prod_lines.sh crates/presenter-core/src/audio/vban_encode.rs   # the test-only encoder, ≈ 120 non-blank / ≈ 167 raw
```

Expected ≈ 495 raw production lines (vban ≈ 230, level ≈ 35, mics ≈ 150, status ≈ 70, the two `mod.rs` ≈ 15) + the encoder (≈ 166 raw).

- [ ] **Step 2: Local gates**

```bash
cargo fmt --all -- --check
bash scripts/dev/placeholder_check.sh . && bash scripts/dev/feature_router_check.sh .
QC_TARGETS="$(git diff --name-only origin/dev...HEAD -- '*.rs')" python3 scripts/dev/fn_length_check.py .
python3 scripts/dev/no_panic_check.py .
git status --short    # nothing unexpected
```

- [ ] **Step 3: Push and watch CI.** Load `ci-push-discipline`; `git fetch origin && git merge origin/dev`; push as the supervisor directs; monitor `pipeline.yml` to a terminal state (Format, Clippy `--all-features` — it compiles the `test-helpers` encoder too, Test, Quality, Coverage, the rest). A red job: `gh run view <id> --log-failed`, root cause, ONE fix commit.

- [ ] **Step 4: Close.** `playbook-review` (the `📔 Playbook:` line — the area rule `listen-vban.md` lands with lane 3, when the area is complete); comment on #852: lane C1a-1 merged (codec, level, config / status DTOs), next lane C1a-2.

---

## PR unit C1a-2 — receiver + API

Starts from `origin/dev` with C1a-1 merged. Version: dev is unreleased; no bump unless main caught up.

### Task 5: The listen service and its health

**Files:**
- Modify: `crates/presenter-server/Cargo.toml` (`tokio` gets `net`; `presenter-core` with `test-helpers` in `[dev-dependencies]`), `crates/presenter-server/src/lib.rs` (insert `mod listen;` after `mod drive;`)
- Create: `crates/presenter-server/src/listen/mod.rs`, `crates/presenter-server/src/listen/health.rs`
- Test: `crates/presenter-server/src/listen/tests.rs` (first part; Task 6 appends)

**Interfaces:**
- Consumes: core `audio::vban::{VbanAudioFormat, VBAN_DEFAULT_PORT}`, `listen::{mics::MicConfig, status::*}`; tokio `watch`, `time::Instant`.
- Produces (`crate::listen`): `MIC_TARGET = "presenter::listen::mic"`, `VBAN_ADDR_ENV = "PRESENTER_LISTEN_VBAN_ADDR"`, `DEFAULT_VBAN_ADDR: SocketAddr` (0.0.0.0:6980), `NO_AUDIO_AFTER = 3 s`; `#[derive(Clone)] struct ListenService { new(), mics() -> Arc<Vec<MicConfig>>, apply(Vec<MicConfig>), status() -> ListenStatus, start(SocketAddr) -> bool, save_guard() -> tokio MutexGuard<()>, with_health(FnOnce(&mut ListenHealth) -> R) -> R }` (+ private `config_receiver`); `fn vban_bind_addr() -> SocketAddr`, `fn parse_bind_addr(Option<&str>) -> SocketAddr`, `fn should_log_event(u64) -> bool`, `fn bind_retry_delay(u32) -> Duration`. **No frame channel** (review 2): it arrives with its first consumer (C1b recorder / C3 ASR).
- Produces (`crate::listen::health`): `RECENT_CAP = 16`; `struct SocketHealth { state, bind_addr, local_addr, error }`; `struct PacketSeen { from, format, frames, at }`; `struct ListenHealth { socket, … }` with `set_socket`, `set_mics(kept: &[&str], fresh: &[&str])`, `record_packet(id, PacketSeen)`, `note_unmatched(name, from, format, now) -> u64` (packets of that stream NAME), `snapshot(&[MicConfig], now) -> ListenStatus`. Packets per second and the per-sender reject counters come with lane 3's counters (Task 12), to keep this lane within its size budget.

- [ ] **Step 1: The manifest.** In `crates/presenter-server/Cargo.toml` `[dependencies]`, replace `tokio.workspace = true` with:

```toml
# #852 C1a: `net` for the VBAN UdpSocket (listen/vban_rx.rs) — explicit, not
# only through axum's feature set.
tokio = { workspace = true, features = ["net"] }
```

In `[dev-dependencies]`, after `presenter-persistence = { path = "../presenter-persistence" }`:

```toml
# #852 C1a: the ported songplayer VBAN encoder (presenter_core::audio::vban_encode)
# generates the packets of the receiver tests.
presenter-core = { path = "../presenter-core", features = ["test-helpers"] }
```

`cargo metadata --format-version 1 --offline > /dev/null` must leave `Cargo.lock` unchanged (`git diff --stat Cargo.lock` empty: tokio's `net` feature and a path dev-dependency add no package).

- [ ] **Step 2: Write the failing tests** — `crates/presenter-server/src/listen/tests.rs` (declared at the end of `listen/mod.rs`):

```rust
//! #852 C1a: the receiver's routing, health and socket life cycle. Routing
//! cases call `MicRouter::handle_datagram` directly with RFC 5737 senders and
//! explicit instants (never a sleep); the socket path has its own real
//! loopback UDP tests on `127.0.0.1:0` with bounded 5 s polls (`wait_for`).

use super::health::{ListenHealth, PacketSeen, RECENT_CAP};
use super::*;
use presenter_core::audio::vban::{VbanAudioFormat, VbanSampleType};
use presenter_core::listen::mics::{MicChannel, MicLanguage, MicRole};
use presenter_core::listen::status::{MicState, SocketState};

const GUEST: &str = "192.0.2.10:6980";
const OTHER: &str = "198.51.100.7:6980";
const MONO_48K: VbanAudioFormat =
    VbanAudioFormat { rate_hz: 48_000, channels: 1, sample_type: VbanSampleType::Int16 };

fn addr(text: &str) -> SocketAddr {
    text.parse().unwrap()
}

fn mic(id: &str, stream: &str, sender_ip: Option<&str>) -> MicConfig {
    MicConfig {
        id: id.to_string(),
        name: id.to_string(),
        stream_name: stream.to_string(),
        sender_ip: sender_ip.map(str::to_string),
        channel: MicChannel::Left,
        language: MicLanguage::En,
        role: MicRole::Guest,
    }
}

fn seen(at: Instant) -> PacketSeen {
    PacketSeen { from: addr(GUEST), format: MONO_48K, frames: 256, at }
}

/// Poll `service.status()` until `done` holds; fails with the last status after 5 s.
async fn wait_for(
    service: &ListenService,
    what: &str,
    done: impl Fn(&ListenStatus) -> bool,
) -> ListenStatus {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let status = service.status();
        if done(&status) {
            return status;
        }
        assert!(Instant::now() < deadline, "{what}: {status:#?}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[test]
fn should_log_event_is_the_power_of_two_gate() {
    let gated: Vec<bool> = (0..10).map(should_log_event).collect();
    assert_eq!(
        gated,
        vec![false, true, true, false, true, false, false, false, true, false]
    );
}

#[test]
fn bind_retries_back_off_to_a_minute() {
    let delays: Vec<u64> = (1..=9).map(|n| bind_retry_delay(n).as_secs()).collect();
    assert_eq!(delays, vec![1, 2, 4, 8, 16, 32, 60, 60, 60]);
    assert_eq!(bind_retry_delay(u32::MAX).as_secs(), 60);
}

#[test]
fn the_bind_address_is_the_env_value_or_the_vban_default() {
    assert_eq!(DEFAULT_VBAN_ADDR.to_string(), "0.0.0.0:6980");
    assert_eq!(parse_bind_addr(None), DEFAULT_VBAN_ADDR);
    assert_eq!(parse_bind_addr(Some("  ")), DEFAULT_VBAN_ADDR);
    assert_eq!(parse_bind_addr(Some(" 127.0.0.1:0 ")), addr("127.0.0.1:0"));
    assert_eq!(parse_bind_addr(Some("nonsense")), DEFAULT_VBAN_ADDR);
}

#[tokio::test]
async fn the_mic_state_follows_the_last_packet() {
    let t0 = Instant::now();
    let mics = vec![mic("guest", "GUEST_EN", None)];
    let mut health = ListenHealth::default();
    health.set_mics(&[], &["guest"]);
    assert_eq!(health.snapshot(&mics, t0).mics[0].state, MicState::Waiting);
    health.record_packet("guest", seen(t0));
    let mic = &health.snapshot(&mics, t0 + Duration::from_millis(500)).mics[0];
    assert_eq!(
        (mic.state, mic.packets, mic.last_packet_age_ms),
        (MicState::Receiving, 1, Some(500))
    );
    assert_eq!(mic.sender.as_deref(), Some(GUEST));
    assert_eq!((mic.format, mic.frames_per_packet), (Some(MONO_48K), Some(256)));
    let silent = &health.snapshot(&mics, t0 + Duration::from_secs(3)).mics[0];
    assert_eq!(silent.state, MicState::NoAudio, "3 s after the last packet: no audio (C-S1)");
}

#[tokio::test]
async fn a_saved_list_keeps_kept_counters_and_starts_fresh_ones() {
    let t0 = Instant::now();
    let mut health = ListenHealth::default();
    health.set_mics(&[], &["guest", "interp"]);
    health.record_packet("guest", seen(t0));
    health.record_packet("interp", seen(t0));
    health.set_mics(&["guest"], &["spare"]);
    let mics = vec![
        mic("guest", "GUEST_EN", None),
        mic("spare", "SPARE", None),
        mic("interp", "INTERP_SK", None),
    ];
    let packets: Vec<u64> = health.snapshot(&mics, t0).mics.iter().map(|m| m.packets).collect();
    assert_eq!(packets, vec![1, 0, 0], "guest kept, spare fresh, interp dropped");
}

#[tokio::test]
async fn unmatched_streams_keep_the_16_most_recent_and_count_per_name() {
    let t0 = Instant::now();
    let mut health = ListenHealth::default();
    for k in 0..=16u64 {
        let at = t0 + Duration::from_millis(k);
        assert_eq!(health.note_unmatched(&format!("S{k:02}"), addr(OTHER), MONO_48K, at), 1);
    }
    let again = t0 + Duration::from_millis(17);
    assert_eq!(health.note_unmatched("S16", addr(OTHER), MONO_48K, again), 2, "per NAME");
    let status = health.snapshot(&[], t0 + Duration::from_secs(1));
    let names: Vec<&str> = status.unmatched.iter().map(|s| s.stream_name.as_str()).collect();
    assert_eq!(names.len(), RECENT_CAP);
    assert!(!names.contains(&"S00"), "the oldest went");
    assert_eq!(names.first(), Some(&"S01"), "sorted by name");
    assert_eq!(status.unmatched[15].packets, 2);
}

#[tokio::test]
async fn a_service_that_was_never_started_reports_not_started() {
    let service = ListenService::new();
    service.apply(vec![mic("guest", "GUEST_EN", None)]);
    let status = service.status();
    assert_eq!(status.socket.state, SocketState::NotStarted);
    assert_eq!((status.mics.len(), status.mics[0].state), (1, MicState::Waiting));
    assert_eq!(service.mics().len(), 1);
}
```

- [ ] **Step 3: How it is verified.** CI `Test` job (`listen::tests`).

- [ ] **Step 4: Implement the module** — `crates/presenter-server/src/listen/mod.rs`; `crates/presenter-server/src/lib.rs` gets `mod listen;` after `mod drive;`:

```rust
//! Sermon listening, the audio ingest (#852 C1a, spec §5): ONE UDP socket
//! receives the VBAN microphones (`vban_rx`); `GET /listen/status` reads
//! `health`. [`ListenService`] (in `AppState`) holds the current `listen.mics`
//! (a `watch`), the health (a std mutex, never held across an `.await`), the
//! start-once flag and the save lock. Started from `spawn_background_tasks`
//! only, so validate mode never binds.

pub(crate) mod health;
mod vban_rx;
#[cfg(test)]
mod tests;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use presenter_core::audio::vban::VBAN_DEFAULT_PORT;
use presenter_core::listen::mics::MicConfig;
use presenter_core::listen::status::ListenStatus;
use tokio::sync::watch;
use tokio::time::Instant;

use self::health::ListenHealth;

/// The target of every receiver log line (spec §14 `presenter::listen::mic`).
pub(crate) const MIC_TARGET: &str = "presenter::listen::mic";
/// Overrides where the VBAN socket binds (tests / E2E: `127.0.0.1:0`).
pub(crate) const VBAN_ADDR_ENV: &str = "PRESENTER_LISTEN_VBAN_ADDR";
/// Every interface, the VBAN default port (spec §5.1).
pub(crate) const DEFAULT_VBAN_ADDR: SocketAddr =
    SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), VBAN_DEFAULT_PORT);
/// A mic whose last packet is this old has no audio ("žiadny zvuk" within 3 s, C-S1).
pub(crate) const NO_AUDIO_AFTER: Duration = Duration::from_secs(3);
const BIND_RETRY_CAP: Duration = Duration::from_secs(60);

/// The listen state every `AppState` clone shares.
#[derive(Clone)]
pub(crate) struct ListenService {
    inner: Arc<Inner>,
}

struct Inner {
    config: watch::Sender<Arc<Vec<MicConfig>>>,
    health: Mutex<ListenHealth>,
    started: AtomicBool,
    save_lock: tokio::sync::Mutex<()>,
}

impl ListenService {
    pub(crate) fn new() -> Self {
        let (config, _) = watch::channel(Arc::new(Vec::new()));
        Self {
            inner: Arc::new(Inner {
                config,
                health: Mutex::new(ListenHealth::default()),
                started: AtomicBool::new(false),
                save_lock: tokio::sync::Mutex::new(()),
            }),
        }
    }

    /// The mics the receiver uses now.
    pub(crate) fn mics(&self) -> Arc<Vec<MicConfig>> {
        Arc::clone(&self.inner.config.borrow())
    }

    /// Hand a validated list to the receiver; it updates its mics at once.
    pub(crate) fn apply(&self, mics: Vec<MicConfig>) {
        let _previous = self.inner.config.send_replace(Arc::new(mics));
    }

    /// `GET /listen/status`, ages measured now.
    pub(crate) fn status(&self) -> ListenStatus {
        let mics = self.mics();
        self.with_health(|health| health.snapshot(&mics, Instant::now()))
    }

    /// Start the socket task once; `false` when it was already started.
    pub(crate) fn start(&self, bind: SocketAddr) -> bool {
        if self.inner.started.swap(true, Ordering::SeqCst) {
            return false;
        }
        self.with_health(|health| health.socket.bind_addr = Some(bind));
        tokio::spawn(vban_rx::supervise(self.clone(), bind));
        true
    }

    /// Serializes `listen.mics` writers AND the startup read, so the stored and
    /// the applied list always agree.
    pub(crate) async fn save_guard(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.inner.save_lock.lock().await
    }

    pub(crate) fn with_health<R>(&self, f: impl FnOnce(&mut ListenHealth) -> R) -> R {
        let mut health = self.inner.health.lock().unwrap_or_else(PoisonError::into_inner);
        f(&mut health)
    }

    fn config_receiver(&self) -> watch::Receiver<Arc<Vec<MicConfig>>> {
        self.inner.config.subscribe()
    }
}

/// Where the VBAN socket binds: `PRESENTER_LISTEN_VBAN_ADDR`, else 0.0.0.0:6980.
pub(crate) fn vban_bind_addr() -> SocketAddr {
    parse_bind_addr(std::env::var(VBAN_ADDR_ENV).ok().as_deref())
}

pub(crate) fn parse_bind_addr(raw: Option<&str>) -> SocketAddr {
    let Some(value) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return DEFAULT_VBAN_ADDR;
    };
    value.parse().unwrap_or_else(|err| {
        tracing::warn!(
            target: MIC_TARGET,
            value,
            %err,
            "invalid PRESENTER_LISTEN_VBAN_ADDR — binding 0.0.0.0:6980"
        );
        DEFAULT_VBAN_ADDR
    })
}

/// The #484 log gate: the 1st event and every power-of-two one.
pub(crate) fn should_log_event(count: u64) -> bool {
    count.is_power_of_two()
}

/// The pause before bind attempt `failures + 1`: 1, 2, 4 … s, at most 60 s
/// (the #484 `backoff_interval` shape).
pub(crate) fn bind_retry_delay(failures: u32) -> Duration {
    let shift = failures.saturating_sub(1).min(32);
    Duration::from_secs(1u64.checked_shl(shift).unwrap_or(u64::MAX)).min(BIND_RETRY_CAP)
}
```

- [ ] **Step 5: Implement the health** — `crates/presenter-server/src/listen/health.rs`:

```rust
//! What `GET /listen/status` reports (#852 C1a, spec §5.4, §14): the socket,
//! the per-mic counters and the streams seen but not used. Ages are computed
//! at read time: "no audio" (C-S1) appears exactly 3 s after the last packet.

use std::collections::HashMap;
use std::hash::Hash;
use std::net::SocketAddr;

use presenter_core::audio::vban::VbanAudioFormat;
use presenter_core::listen::mics::MicConfig;
use presenter_core::listen::status::{
    ListenStatus, MicState, MicStatus, SocketState, SocketStatus, UnmatchedStream,
};
use tokio::time::Instant;

use super::NO_AUDIO_AFTER;

/// Streams seen but not used (and their names) are kept at most this many
/// (the oldest goes): a spoofed flood cannot grow the status.
pub(crate) const RECENT_CAP: usize = 16;

#[derive(Debug)]
pub(crate) struct SocketHealth {
    pub(crate) state: SocketState,
    pub(crate) bind_addr: Option<SocketAddr>,
    pub(crate) local_addr: Option<SocketAddr>,
    pub(crate) error: Option<String>,
}

impl Default for SocketHealth {
    fn default() -> Self {
        Self { state: SocketState::NotStarted, bind_addr: None, local_addr: None, error: None }
    }
}

/// One packet a mic took.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PacketSeen {
    pub(crate) from: SocketAddr,
    pub(crate) format: VbanAudioFormat,
    pub(crate) frames: u16,
    pub(crate) at: Instant,
}

#[derive(Debug, Default)]
struct MicCounters {
    sender: Option<SocketAddr>,
    format: Option<VbanAudioFormat>,
    frames_per_packet: Option<u16>,
    packets: u64,
    last_packet: Option<Instant>,
}

#[derive(Debug, Default)]
struct UnmatchedEntry {
    format: Option<VbanAudioFormat>,
    packets: u64,
}

/// A map that keeps the [`RECENT_CAP`] most recently seen keys.
#[derive(Debug)]
struct Recent<K, V> {
    entries: HashMap<K, (V, Instant)>,
}

impl<K, V> Default for Recent<K, V> {
    fn default() -> Self {
        Self { entries: HashMap::new() }
    }
}

impl<K: Eq + Hash + Clone, V: Default> Recent<K, V> {
    /// The entry of `key`, seen `now` (a new key pushes out the oldest).
    fn touch(&mut self, key: K, now: Instant) -> &mut V {
        if !self.entries.contains_key(&key) && self.entries.len() >= RECENT_CAP {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, (_, seen))| *seen)
                .map(|(key, _)| key.clone());
            if let Some(oldest) = oldest {
                self.entries.remove(&oldest);
            }
        }
        let entry = self.entries.entry(key).or_insert_with(|| (V::default(), now));
        entry.1 = now;
        &mut entry.0
    }
}

#[derive(Debug, Default)]
pub(crate) struct ListenHealth {
    pub(crate) socket: SocketHealth,
    mics: HashMap<String, MicCounters>,
    unmatched: Recent<(String, SocketAddr), UnmatchedEntry>,
    /// Packets per unassigned stream NAME: the #484 gate of its INFO line.
    unmatched_names: Recent<String, u64>,
}

impl ListenHealth {
    pub(crate) fn set_socket(
        &mut self,
        state: SocketState,
        local_addr: Option<SocketAddr>,
        error: Option<String>,
    ) {
        self.socket.state = state;
        self.socket.local_addr = local_addr;
        self.socket.error = error;
    }

    /// A saved mic list: `kept` mics keep their counters (same stream and
    /// sender — a rename or a language change resets nothing), `fresh` ones
    /// start from zero, mics no longer listed go.
    pub(crate) fn set_mics(&mut self, kept: &[&str], fresh: &[&str]) {
        self.mics.retain(|id, _| kept.contains(&id.as_str()));
        for id in fresh {
            self.mics.insert((*id).to_string(), MicCounters::default());
        }
    }

    pub(crate) fn record_packet(&mut self, mic_id: &str, seen: PacketSeen) {
        let Some(mic) = self.mics.get_mut(mic_id) else {
            return;
        };
        mic.packets += 1;
        mic.last_packet = Some(seen.at);
        mic.sender = Some(seen.from);
        mic.format = Some(seen.format);
        mic.frames_per_packet = Some(seen.frames);
    }

    /// A stream no mic takes; the packets seen so far of that stream NAME.
    pub(crate) fn note_unmatched(
        &mut self,
        stream_name: &str,
        from: SocketAddr,
        format: VbanAudioFormat,
        now: Instant,
    ) -> u64 {
        let entry = self.unmatched.touch((stream_name.to_string(), from), now);
        entry.packets += 1;
        entry.format = Some(format);
        let packets = self.unmatched_names.touch(stream_name.to_string(), now);
        *packets += 1;
        *packets
    }

    pub(crate) fn snapshot(&self, mics: &[MicConfig], now: Instant) -> ListenStatus {
        let mut unmatched: Vec<UnmatchedStream> = self
            .unmatched
            .entries
            .iter()
            .filter_map(|((stream_name, from), (entry, seen))| {
                Some(UnmatchedStream {
                    stream_name: stream_name.clone(),
                    sender: from.to_string(),
                    format: entry.format?,
                    packets: entry.packets,
                    last_packet_age_ms: age_ms(*seen, now),
                })
            })
            .collect();
        unmatched.sort_by(|a, b| (&a.stream_name, &a.sender).cmp(&(&b.stream_name, &b.sender)));
        ListenStatus {
            socket: SocketStatus {
                state: self.socket.state,
                bind_addr: self.socket.bind_addr.map(|addr| addr.to_string()),
                local_addr: self.socket.local_addr.map(|addr| addr.to_string()),
                error: self.socket.error.clone(),
            },
            mics: mics.iter().map(|config| mic_status(config, self.mics.get(&config.id), now)).collect(),
            unmatched,
        }
    }
}

/// Waiting (no packet yet) → receiving → no audio 3 s after the last packet.
fn mic_state(counters: &MicCounters, now: Instant) -> MicState {
    match counters.last_packet {
        None => MicState::Waiting,
        Some(at) if now.saturating_duration_since(at) >= NO_AUDIO_AFTER => MicState::NoAudio,
        Some(_) => MicState::Receiving,
    }
}

fn mic_status(config: &MicConfig, counters: Option<&MicCounters>, now: Instant) -> MicStatus {
    let empty = MicCounters::default();
    let c = counters.unwrap_or(&empty);
    MicStatus {
        config: config.clone(),
        state: mic_state(c, now),
        sender: c.sender.map(|addr| addr.to_string()),
        format: c.format,
        frames_per_packet: c.frames_per_packet,
        packets: c.packets,
        last_packet_age_ms: c.last_packet.map(|at| age_ms(at, now)),
    }
}

fn age_ms(at: Instant, now: Instant) -> u64 {
    u64::try_from(now.saturating_duration_since(at).as_millis()).unwrap_or(u64::MAX)
}
```

- [ ] **Step 6: Gates + commit.** `cargo fmt --all -- --check`; `count_prod_lines.sh` on `listen/mod.rs` (≈ 100) and `listen/health.rs` (≈ 165); `QC_TARGETS` both. The module compiles with Task 6's `vban_rx` (CI runs once per lane push); commit paths `crates/presenter-server/Cargo.toml crates/presenter-server/src/lib.rs crates/presenter-server/src/listen/`, message `feat(listen): listen service, health counters and the status snapshot (#852 C1a-2)`.

### Task 6: The VBAN receiver — socket supervisor, receive loop, routing

**Files:**
- Create: `crates/presenter-server/src/listen/vban_rx.rs`
- Modify: `crates/presenter-core/src/audio/vban_encode.rs` (add `tone`, the code of Task 1 Step 5: lane 1 left it out until this first caller, C1a-1 review 1)
- Test: `crates/presenter-server/src/listen/tests.rs` (append; `use` lines added at its top)

**Interfaces:**
- Consumes: core `decode_packet`, `VbanPacket`, `MicConfig::sender_ip_addr`; `ListenService::{config_receiver, with_health}`, `health::PacketSeen` (Task 5).
- Produces (`crate::listen::vban_rx`): `async fn supervise(ListenService, SocketAddr)` (spawned by `ListenService::start`); `pub(super) enum SocketEnd { NoMic, Failed }`, `pub(super) async fn receive(&ListenService, &UdpSocket, &mut ConfigRx) -> SocketEnd` (pub(super) so the race test can call it); `struct MicRouter { new(&ListenService, &[MicConfig]), update(&mut self, &[MicConfig]), handle_datagram(&mut self, &[u8], SocketAddr, Instant), on_tick(&mut self, Instant) }`.

- [ ] **Step 1: Write the failing tests.** At the top of `crates/presenter-server/src/listen/tests.rs`, after the existing `use` lines:

```rust
use super::vban_rx::{receive, MicRouter, SocketEnd};
use crate::tracing_capture::{quoted, Capture};
use presenter_core::audio::vban_encode::{tone, VbanEncoder};
use presenter_core::listen::status::MicStatus;
```

Append:

```rust
// ---- the receiver (MicRouter): explicit senders and instants ----------------

fn guest_encoder() -> VbanEncoder {
    VbanEncoder::new("GUEST_EN", 48_000, 1, VbanSampleType::Int16)
}

/// One 256-frame packet of a 1 kHz tone (48 kHz: 187 packets per second).
fn next_packet(encoder: &mut VbanEncoder) -> Vec<u8> {
    encoder.packet(&tone(48_000, 1_000.0, 0.5, 0, 256, 1)).unwrap()
}

fn router_for(configs: Vec<MicConfig>) -> (ListenService, MicRouter) {
    let service = ListenService::new();
    service.apply(configs.clone());
    let router = MicRouter::new(&service, &configs);
    (service, router)
}

fn status_of(service: &ListenService, id: &str, now: Instant) -> MicStatus {
    let mics = service.mics();
    let status = service.with_health(|health| health.snapshot(&mics, now));
    status.mics.into_iter().find(|mic| mic.config.id == id).unwrap()
}

#[tokio::test]
async fn packets_are_routed_by_stream_name_and_sender_ip() {
    let (service, mut router) = router_for(vec![
        mic("guest", "GUEST_EN", Some("192.0.2.10")),
        mic("interp", "INTERP_SK", None),
    ]);
    let t0 = Instant::now();
    let mut guest = guest_encoder();
    let mut interp = VbanEncoder::new("INTERP_SK", 48_000, 1, VbanSampleType::Int16);
    let mut stray = VbanEncoder::new("Stream1", 48_000, 2, VbanSampleType::Int24);
    router.handle_datagram(&next_packet(&mut guest), addr(GUEST), t0);
    router.handle_datagram(&next_packet(&mut guest), addr(OTHER), t0); // right name, wrong sender
    router.handle_datagram(&next_packet(&mut interp), addr(OTHER), t0);
    let stray_packet = stray.packet(&tone(48_000, 440.0, 0.25, 0, 64, 2)).unwrap();
    router.handle_datagram(&stray_packet, addr(OTHER), t0);
    assert_eq!(status_of(&service, "guest", t0).packets, 1);
    assert_eq!(status_of(&service, "interp", t0).packets, 1);
    let status = service.with_health(|health| health.snapshot(&[], t0));
    let seen: Vec<(&str, &str)> = status
        .unmatched
        .iter()
        .map(|s| (s.stream_name.as_str(), s.sender.as_str()))
        .collect();
    assert_eq!(seen, vec![("GUEST_EN", OTHER), ("Stream1", OTHER)], "seen, not used");
    assert_eq!(
        status.unmatched[1].format,
        VbanAudioFormat { rate_hz: 48_000, channels: 2, sample_type: VbanSampleType::Int24 }
    );
}

#[tokio::test]
async fn an_unassigned_stream_is_logged_on_power_of_two_packets_per_name() {
    let capture = Capture::install();
    let (_service, mut router) = router_for(vec![mic("guest", "GUEST_EN", None)]);
    let t0 = Instant::now();
    let mut stray = VbanEncoder::new("Stream1", 48_000, 1, VbanSampleType::Int16);
    for _ in 0..5 {
        router.handle_datagram(&next_packet(&mut stray), addr(OTHER), t0);
    }
    // the same name from another sender counts on the same per-name gate
    router.handle_datagram(&next_packet(&mut stray), addr(GUEST), t0);
    let lines = capture.events_with_message("VBAN stream not assigned to any mic");
    let packets: Vec<&str> = lines.iter().filter_map(|line| line.field("packets")).collect();
    assert_eq!(packets, vec!["1", "2", "4"]);
    assert_eq!(lines[0].field("stream"), Some(quoted("Stream1").as_str()));
}

#[tokio::test]
async fn a_stopped_sender_shows_no_audio_after_3_s_and_logs_it_once() {
    let capture = Capture::install();
    let (service, mut router) = router_for(vec![mic("guest", "GUEST_EN", None)]);
    let t0 = Instant::now();
    router.handle_datagram(&next_packet(&mut guest_encoder()), addr(GUEST), t0);
    let just_before = t0 + Duration::from_millis(2_999);
    assert_eq!(status_of(&service, "guest", just_before).state, MicState::Receiving);
    let at_3_s = t0 + Duration::from_millis(3_000);
    assert_eq!(status_of(&service, "guest", at_3_s).state, MicState::NoAudio);
    for ms in [2_999, 3_100, 4_100] {
        router.on_tick(t0 + Duration::from_millis(ms));
    }
    let events = capture.events(MIC_TARGET);
    let messages: Vec<&str> = events.iter().filter_map(|event| event.field("message")).collect();
    assert_eq!(messages, vec!["mic receives VBAN", "mic lost audio — no VBAN packet for 3 s"]);
    assert_eq!(events[0].field("mic"), Some(quoted("guest").as_str()));
    assert_eq!(events[0].field("stream"), Some(quoted("GUEST_EN").as_str()));
    assert_eq!(events[0].field("rate"), Some("48000"));
}

#[tokio::test]
async fn a_rename_keeps_the_mic_and_a_new_stream_name_starts_it_fresh() {
    let (service, mut router) = router_for(vec![mic("guest", "GUEST_EN", None)]);
    let t0 = Instant::now();
    router.handle_datagram(&next_packet(&mut guest_encoder()), addr(GUEST), t0);
    let mut renamed = mic("guest", "GUEST_EN", None);
    renamed.name = "Hosť z Londýna".into();
    renamed.language = MicLanguage::Off;
    service.apply(vec![renamed.clone()]);
    router.update(&[renamed]);
    assert_eq!(status_of(&service, "guest", t0).packets, 1, "a rename / language change resets nothing");
    let moved = mic("guest", "GUEST_2", None);
    service.apply(vec![moved.clone()]);
    router.update(&[moved]);
    assert_eq!(status_of(&service, "guest", t0).packets, 0, "another stream name: a fresh mic");
}

// ---- the socket (real loopback UDP) ----------------------------------------

#[tokio::test]
async fn receive_ends_at_once_when_the_list_was_emptied_before_it_started() {
    // supervise saw a mic, then the list was emptied before the bind finished
    let service = ListenService::new();
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let mut config = service.config_receiver();
    let end = tokio::time::timeout(Duration::from_secs(5), receive(&service, &socket, &mut config))
        .await
        .expect("receive returns at once");
    assert_eq!(end, SocketEnd::NoMic);
}

#[tokio::test]
async fn no_mic_binds_no_socket_and_a_saved_mic_binds_one() {
    let service = ListenService::new();
    assert!(service.start(addr("127.0.0.1:0")));
    assert!(!service.start(addr("127.0.0.1:0")), "started once");
    let idle = wait_for(&service, "idle", |s| s.socket.state == SocketState::Idle).await;
    assert_eq!((idle.socket.local_addr, idle.socket.bind_addr.as_deref()), (None, Some("127.0.0.1:0")));
    service.apply(vec![mic("guest", "GUEST_EN", None)]);
    let bound = wait_for(&service, "bound", |s| s.socket.state == SocketState::Bound).await;
    let local: SocketAddr = bound.socket.local_addr.unwrap().parse().unwrap();
    assert!(local.ip().is_loopback() && local.port() != 0);
    service.apply(Vec::new());
    wait_for(&service, "closed", |s| s.socket.state == SocketState::Idle).await;
    std::net::UdpSocket::bind(local).expect("the port is free again: the socket was closed");
}

#[tokio::test]
async fn a_taken_port_is_retried_and_reported_until_it_is_free() {
    let holder = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let taken = holder.local_addr().unwrap();
    let service = ListenService::new();
    service.apply(vec![mic("guest", "GUEST_EN", None)]);
    service.start(taken);
    let retrying = wait_for(&service, "retrying", |s| s.socket.state == SocketState::Retrying).await;
    let error = retrying.socket.error.unwrap_or_default();
    assert!(error.contains(&taken.port().to_string()), "{error}");
    drop(holder);
    wait_for(&service, "bound after a retry", |s| s.socket.state == SocketState::Bound).await;
}

#[tokio::test]
async fn real_udp_packets_reach_the_mic_through_the_socket() {
    let service = ListenService::new();
    service.apply(vec![mic("guest", "GUEST_EN", Some("127.0.0.1"))]);
    service.start(addr("127.0.0.1:0"));
    let bound = wait_for(&service, "bound", |s| s.socket.state == SocketState::Bound).await;
    let vban: SocketAddr = bound.socket.local_addr.unwrap().parse().unwrap();
    let tx = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let mut guest = guest_encoder();
    for k in 0..12 {
        tx.send_to(&next_packet(&mut guest), vban).await.unwrap();
        if k % 4 == 3 {
            tokio::time::sleep(Duration::from_millis(5)).await; // Voicemeeter-like bursts of 4
        }
    }
    let status = wait_for(&service, "12 packets", |s| s.mics[0].packets == 12).await;
    let mic = &status.mics[0];
    assert_eq!((mic.state, mic.format, mic.frames_per_packet), (MicState::Receiving, Some(MONO_48K), Some(256)));
    assert_eq!(mic.sender, Some(tx.local_addr().unwrap().to_string()));
}
```

- [ ] **Step 2: How it is verified.** CI `Test` job (`listen::tests`). The socket tests bind `127.0.0.1:0` only, so they never collide with a deployed service on the runner.

- [ ] **Step 3: Implement** — `crates/presenter-server/src/listen/vban_rx.rs`:

```rust
//! The VBAN receiver (#852 C1a, spec §5.1 + §5.3): ONE UDP socket, bound only
//! while a mic is configured (the list ships empty, C-S8). The receive loop
//! never waits on anything downstream (spec rev. 13 p. 11): decode, route and
//! count, synchronously. [`supervise`] binds (retrying 1, 2, 4 … 60 s) and
//! closes the socket when the list is emptied; [`MicRouter`] keeps a mic whose
//! stream name and sender stay across a save. Repeated log lines go through
//! the #484 power-of-two gate.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use presenter_core::audio::vban::{decode_packet, VbanPacket};
use presenter_core::listen::mics::MicConfig;
use presenter_core::listen::status::SocketState;
use tokio::net::UdpSocket;
use tokio::sync::watch;
use tokio::time::{interval, sleep, Instant, MissedTickBehavior};
use tracing::{debug, info, warn};

use super::health::PacketSeen;
use super::{bind_retry_delay, should_log_event, ListenService, MIC_TARGET, NO_AUDIO_AFTER};

/// Big enough for any datagram (a conformant VBAN packet is ≤ 1464 bytes).
const RECV_BUFFER_LEN: usize = 65_536;
/// The "lost audio" line is evaluated this often.
const TICK: Duration = Duration::from_secs(1);

pub(super) type ConfigRx = watch::Receiver<Arc<Vec<MicConfig>>>;

/// Why a bound socket was closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SocketEnd {
    /// The list was emptied (or the service is gone).
    NoMic,
    /// `recv_from` failed: rebind after a pause.
    Failed,
}

pub(super) async fn supervise(service: ListenService, bind: SocketAddr) {
    let mut config = service.config_receiver();
    let mut failures: u32 = 0;
    loop {
        let idle = config.borrow_and_update().is_empty();
        if idle {
            service.with_health(|health| health.set_socket(SocketState::Idle, None, None));
            if config.changed().await.is_err() {
                return;
            }
            continue;
        }
        let error = match UdpSocket::bind(bind).await {
            Ok(socket) => {
                failures = 0;
                let local = socket.local_addr().ok();
                info!(target: MIC_TARGET, %bind, local = ?local, "VBAN socket bound");
                service.with_health(|health| health.set_socket(SocketState::Bound, local, None));
                let end = receive(&service, &socket, &mut config).await;
                info!(target: MIC_TARGET, %bind, ?end, "VBAN socket closed");
                if end == SocketEnd::NoMic {
                    continue;
                }
                "príjem zo socketu VBAN zlyhal".to_string()
            }
            Err(err) => format!("port {} sa nedá otvoriť: {err}", bind.port()),
        };
        failures = failures.saturating_add(1);
        let delay = bind_retry_delay(failures);
        if should_log_event(u64::from(failures)) {
            warn!(
                target: MIC_TARGET,
                %bind,
                error = error.as_str(),
                attempt = failures,
                retry_in_s = delay.as_secs(),
                "the VBAN socket is not receiving — retrying"
            );
        }
        service.with_health(|health| health.set_socket(SocketState::Retrying, None, Some(error)));
        tokio::select! {
            () = sleep(delay) => {}
            changed = config.changed() => {
                if changed.is_err() {
                    return;
                }
            }
        }
    }
}

/// Receive until the list is emptied or the socket fails.
pub(super) async fn receive(service: &ListenService, socket: &UdpSocket, config: &mut ConfigRx) -> SocketEnd {
    let mics = config.borrow_and_update().clone();
    if mics.is_empty() {
        // emptied between `supervise`'s idle check and the bind (review 1b)
        return SocketEnd::NoMic;
    }
    let mut router = MicRouter::new(service, &mics);
    let mut buf = vec![0u8; RECV_BUFFER_LEN];
    let mut tick = interval(TICK);
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            received = socket.recv_from(&mut buf) => match received {
                Ok((len, from)) => router.handle_datagram(&buf[..len], from, Instant::now()),
                Err(err) => {
                    warn!(target: MIC_TARGET, %err, "receiving from the VBAN socket failed");
                    return SocketEnd::Failed;
                }
            },
            changed = config.changed() => {
                if changed.is_err() {
                    return SocketEnd::NoMic;
                }
                let mics = config.borrow_and_update().clone();
                if mics.is_empty() {
                    return SocketEnd::NoMic;
                }
                router.update(&mics);
            }
            _ = tick.tick() => router.on_tick(Instant::now()),
        }
    }
}

/// The receive side of every configured mic.
pub(super) struct MicRouter {
    service: ListenService,
    mics: Vec<MicInput>,
}

struct MicInput {
    config: MicConfig,
    /// Parsed once per saved list, never per packet.
    sender_ip: Option<IpAddr>,
    last_packet: Option<Instant>,
    /// "receives" was logged and "lost audio" not yet.
    receiving: bool,
}

impl MicRouter {
    pub(super) fn new(service: &ListenService, configs: &[MicConfig]) -> Self {
        let mut router = Self { service: service.clone(), mics: Vec::new() };
        router.update(configs);
        router
    }

    /// A saved list: a mic whose stream name and sender stay keeps its state
    /// and counters (a rename or a language change resets nothing); every
    /// other mic starts fresh; an unlisted one goes.
    pub(super) fn update(&mut self, configs: &[MicConfig]) {
        let mut old: HashMap<String, MicInput> =
            self.mics.drain(..).map(|input| (input.config.id.clone(), input)).collect();
        let kept: Vec<&str> = configs
            .iter()
            .filter(|config| old.get(&config.id).is_some_and(|input| input.receives_like(config)))
            .map(|config| config.id.as_str())
            .collect();
        let fresh: Vec<&str> = configs.iter().map(|config| config.id.as_str()).filter(|id| !kept.contains(id)).collect();
        self.service.with_health(|health| health.set_mics(&kept, &fresh));
        self.mics = configs
            .iter()
            .map(|config| match old.remove(&config.id) {
                Some(mut input) if input.receives_like(config) => {
                    input.config = config.clone();
                    input
                }
                _ => MicInput::new(config),
            })
            .collect();
    }

    pub(super) fn handle_datagram(&mut self, datagram: &[u8], from: SocketAddr, now: Instant) {
        let packet = match decode_packet(datagram) {
            Ok(packet) => packet,
            Err(reject) => {
                debug!(target: MIC_TARGET, %from, %reject, "dropped a datagram that is not usable VBAN audio");
                return;
            }
        };
        // C1a-1 review 1: `validate_mics` stores the sender IP canonical
        // (`::ffff:192.0.2.10` → `192.0.2.10`), so the peer is canonicalised too.
        let sender = from.ip().to_canonical();
        let index = self.mics.iter().position(|mic| mic.matches(&packet.stream_name, sender));
        match index {
            Some(index) => self.mics[index].accept(&self.service, &packet, from, now),
            None => self.unmatched(&packet, from, now),
        }
    }

    pub(super) fn on_tick(&mut self, now: Instant) {
        for mic in &mut self.mics {
            let stale = mic.last_packet.is_some_and(|at| now.saturating_duration_since(at) >= NO_AUDIO_AFTER);
            if mic.receiving && stale {
                mic.receiving = false;
                info!(
                    target: MIC_TARGET,
                    mic = mic.config.id.as_str(),
                    stream = mic.config.stream_name.as_str(),
                    "mic lost audio — no VBAN packet for 3 s"
                );
            }
        }
    }

    /// Counted per (name, sender) for the status; logged on power-of-two
    /// packets per stream NAME (a bounded map, review 1f).
    fn unmatched(&self, packet: &VbanPacket, from: SocketAddr, now: Instant) {
        let packets = self
            .service
            .with_health(|health| health.note_unmatched(&packet.stream_name, from, packet.format, now));
        if should_log_event(packets) {
            info!(
                target: MIC_TARGET,
                stream = packet.stream_name.as_str(),
                %from,
                rate = packet.format.rate_hz,
                channels = packet.format.channels,
                packets,
                "VBAN stream not assigned to any mic"
            );
        }
    }
}

impl MicInput {
    fn new(config: &MicConfig) -> Self {
        Self { sender_ip: config.sender_ip_addr(), config: config.clone(), last_packet: None, receiving: false }
    }

    /// The same stream name and sender: the mic keeps its state on a save.
    fn receives_like(&self, config: &MicConfig) -> bool {
        self.config.stream_name == config.stream_name && self.config.sender_ip == config.sender_ip
    }

    /// Exact stream name, and the configured sender if there is one.
    fn matches(&self, stream_name: &str, from: IpAddr) -> bool {
        self.config.stream_name == stream_name && self.sender_ip.is_none_or(|ip| ip == from)
    }

    fn accept(&mut self, service: &ListenService, packet: &VbanPacket, from: SocketAddr, now: Instant) {
        let silent = self.last_packet.is_none_or(|at| now.saturating_duration_since(at) >= NO_AUDIO_AFTER);
        if silent {
            self.log_receiving(packet, from);
        }
        self.last_packet = Some(now);
        let seen = PacketSeen { from, format: packet.format, frames: packet.frames, at: now };
        service.with_health(|health| health.record_packet(&self.config.id, seen));
    }

    fn log_receiving(&mut self, packet: &VbanPacket, from: SocketAddr) {
        self.receiving = true;
        info!(
            target: MIC_TARGET,
            mic = self.config.id.as_str(),
            stream = self.config.stream_name.as_str(),
            %from,
            rate = packet.format.rate_hz,
            channels = packet.format.channels,
            sample_type = packet.format.sample_type.as_str(),
            frames = packet.frames,
            "mic receives VBAN"
        );
    }
}
```

(Reviewer notes: `tokio::select!` drops the branch futures before a handler runs, so `&buf[..len]` after `recv_from(&mut buf)` borrows fine — tokio's documented pattern; `UdpSocket::recv_from` and `watch::Receiver::changed` are cancel-safe. The `match old.remove(..) { Some(mut input) if … }` guard drops an input whose stream / sender changed.)

- [ ] **Step 4: Gates + commit.** `cargo fmt --all -- --check`; `count_prod_lines.sh crates/presenter-server/src/listen/vban_rx.rs` (≈ 205); `QC_TARGETS` it; `no_panic_check.py`. Commit paths `crates/presenter-server/src/listen/vban_rx.rs crates/presenter-server/src/listen/tests.rs`, message `feat(listen): VBAN receiver — bind supervisor, never-waiting receive loop, routing (#852 C1a-2)`.

### Task 7: The AppState glue, `GET|PUT /listen/mics`, `GET /listen/status`

**Files:**
- Create: `crates/presenter-server/src/state/listen.rs`, `crates/presenter-server/src/router/listen.rs` (tests inline)
- Modify: `crates/presenter-server/src/state/mod.rs` (module line, field, constructor), `crates/presenter-server/src/state/background_tasks.rs` (start), `crates/presenter-server/src/router.rs` (module line + merge), `crates/presenter-server/src/router/request_log.rs` (quiet GET + its test)
- Test: `crates/presenter-server/src/state/tests.rs` (the validate-mode test), `crates/presenter-server/src/state/listen.rs` (the boot-race test, inline), `tests/e2e/support.ts` (test-server env)

**Interfaces:**
- Consumes: `ListenService` (Task 5), core `validate_mics`, `MicsValidationError`, `LISTEN_MICS_KEY`, `ListenStatus`; `Repository::{get_app_setting, set_app_setting, record_settings_audit}`; `router::integrations::extract_actor`.
- Produces: `AppState::{listen_mics() -> anyhow::Result<Vec<MicConfig>>, set_listen_mics(Vec<MicConfig>, actor: &str) -> anyhow::Result<Vec<MicConfig>>, listen_status() -> ListenStatus, start_listen(SocketAddr) -> bool}` (`pub(crate)`; `start_listen` reads `listen.mics` under `save_guard()`, review 1a), `spawn_listen_task()` (`pub(super)`), `state::listen::StoredMicsCorrupt`; routes `GET /listen/mics` → 200 JSON array, `PUT /listen/mics` (JSON array) → 200 the normalized array | 422 `{ "message": <Slovak rule> }`, `GET /listen/status` → 200 `ListenStatus`.

- [ ] **Step 1: Write the failing tests.** At the end of `crates/presenter-server/src/router/listen.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::router::build_router;
    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode};
    use presenter_core::audio::vban::VbanSampleType;
    use presenter_core::audio::vban_encode::{tone, VbanEncoder};
    use presenter_core::listen::mics::LISTEN_MICS_KEY;
    use presenter_core::listen::status::SocketState;
    use serde_json::{json, Value};
    use std::net::SocketAddr;
    use std::time::Duration;
    use tower::ServiceExt;

    /// The spec §5.6 example.
    const SPEC_MICS: &str = r#"[
        {"id": "guest", "name": "Hosť", "streamName": "GUEST_EN", "senderIp": null,
         "channel": "left", "language": "en", "role": "guest"},
        {"id": "interp", "name": "Tlmočník", "streamName": "INTERP_SK", "senderIp": null,
         "channel": "left", "language": "sk", "role": "interpreter"}]"#;

    async fn send(app: &Router, method: Method, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let builder = Request::builder()
            .method(method)
            .uri(uri)
            .header("x-forwarded-for", "203.0.113.9");
        let request = match body {
            Some(body) => builder
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
            None => builder.body(Body::empty()).unwrap(),
        };
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    async fn app() -> (AppState, Router) {
        let state = AppState::in_memory().await.unwrap();
        let app = build_router(state.clone());
        (state, app)
    }

    #[tokio::test]
    async fn a_fresh_server_has_no_mics_and_an_unstarted_receiver() {
        let (_state, app) = app().await;
        assert_eq!(send(&app, Method::GET, "/listen/mics", None).await, (StatusCode::OK, json!([])));
        let (status, body) = send(&app, Method::GET, "/listen/status", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["socket"]["state"], "not_started");
        assert_eq!((body["mics"].clone(), body["unmatched"].clone()), (json!([]), json!([])));
    }

    #[tokio::test]
    async fn put_stores_normalizes_audits_and_applies_the_list() {
        let (state, app) = app().await;
        let mut mics: Value = serde_json::from_str(SPEC_MICS).unwrap();
        mics[0]["name"] = json!("  Hosť ");
        mics[0]["senderIp"] = json!(" 192.0.2.10 ");
        let (status, saved) = send(&app, Method::PUT, "/listen/mics", Some(mics)).await;
        assert_eq!(status, StatusCode::OK, "{saved}");
        assert_eq!((saved[0]["name"].clone(), saved[0]["senderIp"].clone()), (json!("Hosť"), json!("192.0.2.10")));
        assert_eq!(send(&app, Method::GET, "/listen/mics", None).await, (StatusCode::OK, saved.clone()));
        let stored = state.repository().get_app_setting(LISTEN_MICS_KEY).await.unwrap().unwrap();
        assert_eq!(serde_json::from_str::<Value>(&stored).unwrap(), saved);
        let audit = state
            .repository()
            .list_settings_audit(Some("app_settings"), Some(LISTEN_MICS_KEY), None, 10)
            .await
            .unwrap();
        assert_eq!(audit.len(), 1);
        assert_eq!((audit[0].actor.as_str(), audit[0].before_json.clone()), ("203.0.113.9", None));
        assert_eq!(audit[0].after_json, saved);
        let (_, live) = send(&app, Method::GET, "/listen/status", None).await;
        let ids: Vec<&str> = live["mics"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap()).collect();
        assert_eq!(ids, vec!["guest", "interp"], "the receiver got the list");
        assert_eq!(live["mics"][0]["state"], "waiting");
    }

    #[tokio::test]
    async fn refused_lists_are_422_with_the_slovak_rule_and_store_nothing() {
        let (state, app) = app().await;
        let base: Value = serde_json::from_str(SPEC_MICS).unwrap();
        let mut duplicate = base.clone();
        duplicate[1]["id"] = json!("guest");
        let mut bad_stream = base.clone();
        bad_stream[0]["streamName"] = json!("HOSŤ_EN");
        let mut ambiguous = base.clone();
        ambiguous[1]["streamName"] = json!("GUEST_EN");
        for (body, needle) in [
            (duplicate, "dvakrát"),
            (bad_stream, "VBAN streamu"),
            (ambiguous, "ten istý VBAN stream"),
        ] {
            let (status, reply) = send(&app, Method::PUT, "/listen/mics", Some(body)).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{reply}");
            assert!(reply["message"].as_str().unwrap_or_default().contains(needle), "{reply}");
        }
        assert_eq!(state.repository().get_app_setting(LISTEN_MICS_KEY).await.unwrap(), None);
    }

    #[tokio::test]
    async fn a_corrupt_stored_list_is_a_500_and_the_receiver_stays_idle() {
        let (state, app) = app().await;
        state.repository().set_app_setting(LISTEN_MICS_KEY, "{oops").await.unwrap();
        let (status, reply) = send(&app, Method::GET, "/listen/mics", None).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(reply["message"].as_str().unwrap_or_default().contains("poškodené"), "{reply}");
        assert!(state.start_listen("127.0.0.1:0".parse().unwrap()).await);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while state.listen_status().socket.state != SocketState::Idle {
            assert!(tokio::time::Instant::now() < deadline, "{:?}", state.listen_status());
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(state.listen_status().mics.is_empty(), "nothing received from a corrupt list");
    }

    /// The integration test of the unit: real VBAN packets (the ported
    /// songplayer encoder) over real UDP into the server, read back through a
    /// real HTTP `GET /listen/status`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn real_vban_packets_over_udp_show_in_get_listen_status() {
        let state = AppState::in_memory().await.unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let app = build_router(state.clone());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let http = reqwest::Client::new();
        let guest = json!([{"id": "guest", "name": "Hosť", "streamName": "GUEST_EN",
            "senderIp": "127.0.0.1", "channel": "left", "language": "en", "role": "guest"}]);
        let saved = http.put(format!("{base}/listen/mics")).json(&guest).send().await.unwrap();
        assert_eq!(saved.status(), reqwest::StatusCode::OK);
        assert!(state.start_listen("127.0.0.1:0".parse().unwrap()).await);
        let status = poll_status(&http, &base, |s| s["socket"]["state"] == "bound").await;
        let vban: SocketAddr = status["socket"]["localAddr"].as_str().unwrap().parse().unwrap();

        let tx = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        tx.send_to(b"hello", vban).await.unwrap(); // not VBAN → rejected
        let mut stray = VbanEncoder::new("Stream1", 48_000, 2, VbanSampleType::Int24);
        tx.send_to(&stray.packet(&tone(48_000, 440.0, 0.25, 0, 64, 2)).unwrap(), vban).await.unwrap();
        // 95 counters of 256 frames, counter 10 left out: 94 packets (C1a-3: 1 lost,
        // 24 320 frames = 25 chunks = 8 000 samples = 5 frames at 16 kHz).
        let mut encoder = VbanEncoder::new("GUEST_EN", 48_000, 1, VbanSampleType::Int16);
        let block = tone(48_000, 1_000.0, 0.5, 0, 95 * 256, 1);
        for (k, chunk) in block.chunks(256).enumerate() {
            if k == 10 {
                encoder.skip(1);
                continue;
            }
            tx.send_to(&encoder.packet(chunk).unwrap(), vban).await.unwrap();
            if k % 4 == 3 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }

        let status = poll_status(&http, &base, |s| s["mics"][0]["packets"] == 94).await;
        let mic = &status["mics"][0];
        assert_eq!(mic["state"], "receiving");
        assert_eq!(mic["packets"], json!(94));
        assert_eq!(mic["format"], json!({"rateHz": 48000, "channels": 1, "sampleType": "int16"}));
        assert_eq!(mic["framesPerPacket"], json!(256));
        assert_eq!(mic["sender"], json!(tx.local_addr().unwrap().to_string()));
        assert_eq!(status["unmatched"][0]["streamName"], "Stream1");
        assert_eq!(status["unmatched"][0]["format"]["channels"], json!(2));
    }

    async fn poll_status(http: &reqwest::Client, base: &str, done: impl Fn(&Value) -> bool) -> Value {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let status: Value = http
                .get(format!("{base}/listen/status"))
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            if done(&status) {
                return status;
            }
            assert!(tokio::time::Instant::now() < deadline, "{status:#}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}
```

In `crates/presenter-server/src/state/tests.rs`, after `validate_startup_mode_starts_no_integrations_but_serves_healthz`:

```rust
/// #852 C1a: the schema probe boot must not bind the VBAN port either — a
/// stored `listen.mics` would otherwise make the probe take 6980 next to the
/// live service. The receiver is started only from `spawn_background_tasks`.
#[tokio::test]
async fn validate_startup_mode_never_starts_the_vban_receiver() {
    use presenter_core::listen::mics::LISTEN_MICS_KEY;
    use presenter_core::listen::status::SocketState;

    let tmp = tempfile::NamedTempFile::new().expect("temp file");
    let url = format!("sqlite://{}?mode=rwc", tmp.path().display());
    {
        let repo = presenter_persistence::Repository::connect(
            &presenter_persistence::DatabaseSettings::new(&url),
        )
        .await
        .expect("seed connection");
        repo.set_app_setting(
            LISTEN_MICS_KEY,
            r#"[{"id":"guest","name":"Hosť","streamName":"GUEST_EN","senderIp":null,"channel":"left","language":"en","role":"guest"}]"#,
        )
        .await
        .expect("seed listen.mics");
    }
    let mut config = config_for_db(url);
    config.startup_mode = crate::state::StartupMode::Validate;
    let state = AppState::from_config(config).await.expect("from_config (validate)");
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let status = state.listen_status();
    assert_eq!(status.socket.state, SocketState::NotStarted, "validate mode never starts the receiver");
    assert!(status.mics.is_empty(), "nothing was loaded for it");
    assert_eq!(state.listen_mics().await.expect("stored list").len(), 1, "the stored list is untouched");
}
```

At the end of `crates/presenter-server/src/state/listen.rs` (review 1a — the startup read and a save never interleave):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use presenter_core::listen::status::SocketState;
    use std::time::Duration;

    #[tokio::test]
    async fn the_startup_read_waits_for_a_save_in_progress() {
        let state = AppState::in_memory().await.unwrap();
        let save = state.listen.save_guard().await;
        let starter = {
            let state = state.clone();
            tokio::spawn(async move { state.start_listen("127.0.0.1:0".parse().unwrap()).await })
        };
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            state.listen_status().socket.state,
            SocketState::NotStarted,
            "start_listen waits for the save lock before it reads listen.mics"
        );
        drop(save);
        let started = tokio::time::timeout(Duration::from_secs(5), starter).await.unwrap().unwrap();
        assert!(started);
    }
}
```

In `crates/presenter-server/src/router/request_log.rs`, the test `the_timer_polled_gets_are_quiet_but_a_persons_writes_are_not` gets `"/listen/status",` as the last entry of its `for path in [...]` list, and `a_get_nobody_polls_stays_logged_and_the_quiet_paths_ignore_the_method` gets `"/listen/mics",` (a person's GET stays logged).

- [ ] **Step 2: How it is verified.** CI `Test` job: `router::listen::tests`, `state::tests::validate_startup_mode_never_starts_the_vban_receiver`, `router::request_log::tests`, `state::listen::tests`. The UDP/HTTP test binds only `127.0.0.1:0` ports.

- [ ] **Step 3: The state glue** — `crates/presenter-server/src/state/listen.rs`:

```rust
//! #852 C1a: `listen.mics` ↔ the receiver. Startup only READS it (never in
//! validate mode); `set_listen_mics` is the only writer (+ a best-effort
//! `settings_audit` row, the `ai-settings` pattern, #661).

use super::AppState;
use crate::listen::{vban_bind_addr, MIC_TARGET};
use presenter_core::listen::mics::{validate_mics, MicConfig, LISTEN_MICS_KEY};
use presenter_core::listen::status::ListenStatus;
use presenter_persistence::SettingsAuditSource;
use std::net::SocketAddr;

/// `settings_audit.setting_table` of the row (as `ai-settings`).
const LISTEN_AUDIT_TABLE: &str = "app_settings";

/// A stored `listen.mics` that is not a valid list: a 500, never replaced silently.
#[derive(Debug, thiserror::Error)]
#[error("Uložené nastavenie mikrofónov (listen.mics) je poškodené: {0}")]
pub(crate) struct StoredMicsCorrupt(String);

impl AppState {
    /// The stored list (empty when never saved).
    pub(crate) async fn listen_mics(&self) -> anyhow::Result<Vec<MicConfig>> {
        let Some(raw) = self.repository.get_app_setting(LISTEN_MICS_KEY).await? else {
            return Ok(Vec::new());
        };
        let mics: Vec<MicConfig> =
            serde_json::from_str(&raw).map_err(|err| StoredMicsCorrupt(err.to_string()))?;
        validate_mics(mics).map_err(|err| anyhow::Error::new(StoredMicsCorrupt(err.to_string())))
    }

    /// `PUT /listen/mics`: validate (→ `MicsValidationError`, 422), store, apply, audit.
    pub(crate) async fn set_listen_mics(
        &self,
        mics: Vec<MicConfig>,
        actor: &str,
    ) -> anyhow::Result<Vec<MicConfig>> {
        let mics = validate_mics(mics)?;
        let _save = self.listen.save_guard().await;
        let before = self.repository.get_app_setting(LISTEN_MICS_KEY).await?;
        self.repository
            .set_app_setting(LISTEN_MICS_KEY, &serde_json::to_string(&mics)?)
            .await?;
        self.listen.apply(mics.clone());
        tracing::info!(target: MIC_TARGET, mics = mics.len(), actor, "listen mics saved");
        self.audit_listen_mics(before, &mics, actor).await;
        Ok(mics)
    }

    pub(crate) fn listen_status(&self) -> ListenStatus {
        self.listen.status()
    }

    /// Load `listen.mics` into the receiver and start its socket task (once).
    /// The read and the apply run under the save lock: a save that comes
    /// during startup waits and is applied after it, never overwritten by
    /// this older read (review 1a).
    pub(crate) async fn start_listen(&self, bind: SocketAddr) -> bool {
        {
            let _save = self.listen.save_guard().await;
            match self.listen_mics().await {
                Ok(mics) => self.listen.apply(mics),
                Err(err) => tracing::warn!(
                    target: MIC_TARGET,
                    error = %err,
                    "listen.mics cannot be read — no microphone is received until it is saved again"
                ),
            }
        }
        self.listen.start(bind)
    }

    /// From `spawn_background_tasks` (never in validate mode).
    pub(super) fn spawn_listen_task(&self) {
        let state = self.clone();
        tokio::spawn(async move {
            state.start_listen(vban_bind_addr()).await;
        });
    }

    /// Best effort: the write already succeeded, an audit failure only WARNs.
    async fn audit_listen_mics(&self, before: Option<String>, after: &[MicConfig], actor: &str) {
        let before = before.map(|raw| serde_json::from_str(&raw).unwrap_or(serde_json::Value::String(raw)));
        let after = serde_json::to_value(after).unwrap_or_default();
        let recorded = self
            .repository
            .record_settings_audit(
                LISTEN_AUDIT_TABLE,
                LISTEN_MICS_KEY,
                SettingsAuditSource::HttpSetter,
                actor,
                before,
                after,
            )
            .await;
        if let Err(err) = recorded {
            tracing::warn!(target: MIC_TARGET, ?err, "failed to record the listen.mics audit row (the save itself succeeded)");
        }
    }
}
```

In `crates/presenter-server/src/state/mod.rs`:
- after `pub(crate) mod interpreter_tips;` add `mod listen;`;
- after the `interpreter_tips: interpreter_tips::InterpreterTipsState,` field:

```rust
    /// #852 C1a: the VBAN microphones → 16 kHz frames + `/listen/status`.
    listen: crate::listen::ListenService,
```

- in `new_with_heartbeat`, after `interpreter_tips: interpreter_tips::InterpreterTipsState::new(),`:

```rust
            listen: crate::listen::ListenService::new(),
```

In `crates/presenter-server/src/state/background_tasks.rs` `spawn_background_tasks`, after `self.spawn_interpreter_tips_task();`:

```rust

        // #852 C1a: the VBAN receiver — binds only while listen.mics is non-empty.
        self.spawn_listen_task();
```

- [ ] **Step 4: The router** — `crates/presenter-server/src/router/listen.rs` (before the test block):

```rust
//! #852 C1a: `GET|PUT /listen/mics` (the whole list; 422 with the Slovak rule)
//! and `GET /listen/status` (polled → a quiet GET in `request_log.rs`).

use super::integrations::extract_actor;
use super::AppError;
use crate::state::AppState;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::get;
use axum::{Json, Router};
use presenter_core::listen::mics::{MicConfig, MicsValidationError};
use presenter_core::listen::status::ListenStatus;
use tracing::instrument;

/// The listen sub-router.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/listen/mics", get(get_mics).put(put_mics))
        .route("/listen/status", get(get_status))
}

#[instrument(skip_all)]
async fn get_mics(State(state): State<AppState>) -> Result<Json<Vec<MicConfig>>, AppError> {
    Ok(Json(state.listen_mics().await?))
}

#[instrument(skip_all)]
async fn put_mics(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mics): Json<Vec<MicConfig>>,
) -> Result<Json<Vec<MicConfig>>, AppError> {
    let actor = extract_actor(&headers);
    state.set_listen_mics(mics, &actor).await.map(Json).map_err(map_mics_error)
}

async fn get_status(State(state): State<AppState>) -> Json<ListenStatus> {
    Json(state.listen_status())
}

/// `MicsValidationError` → 422 with its Slovak text; everything else → the
/// central mapping (a corrupt stored list → 500).
fn map_mics_error(err: anyhow::Error) -> AppError {
    match err.downcast_ref::<MicsValidationError>() {
        Some(refusal) => AppError::unprocessable(refusal.to_string()),
        None => AppError::from(err),
    }
}
```

In `crates/presenter-server/src/router.rs`: `mod listen;` after `mod libraries;` in the module list, and right after `.merge(interpreter_tips::router())`:

```rust
        // #852 C1a: listening microphones + the VBAN receiver status.
        .merge(listen::router())
```

In `crates/presenter-server/src/router/request_log.rs`, `POLLED_GETS` becomes `[&str; 7]` with `"/listen/status",` as its last entry.

- [ ] **Step 5: Keep E2E test servers off the dev service's port.** In `tests/e2e/support.ts`, in the `env` object of the test-server spawn, after the `PRESENTER_DRIVE_DOWNLOAD_URL` entry:

```ts
    // #852 C1a: a test server's VBAN receiver must never take the dev
    // service's 0.0.0.0:6980 (the runner host) — a copied DB may carry
    // listen.mics. A spec that sends VBAN reads the port from /listen/status.
    PRESENTER_LISTEN_VBAN_ADDR:
      process.env.PRESENTER_LISTEN_VBAN_ADDR ?? "127.0.0.1:0",
```

Then `node ../../../node_modules/typescript/bin/tsc --noEmit -p tsconfig.json` from the worktree root (or `node node_modules/typescript/bin/tsc …` in the main checkout).

- [ ] **Step 6: Gates.**

```bash
cd "$REPO" && cargo fmt --all -- --check
for f in crates/presenter-server/src/state/mod.rs crates/presenter-server/src/router.rs crates/presenter-server/src/state/listen.rs crates/presenter-server/src/router/listen.rs crates/presenter-server/src/state/background_tasks.rs crates/presenter-server/src/router/request_log.rs; do printf '%s ' "$f"; bash scripts/dev/count_prod_lines.sh "$f"; done
```

Expected: `state/mod.rs` ≤ 951 (one module line + a 2-line field + 1 constructor line), `router.rs` 770, `state/listen.rs` ≈ 75 (its inline test block is exempt), `router/listen.rs` ≈ 45. Then `QC_TARGETS` with the six files (`spawn_background_tasks` grows by 3 lines; `new_with_heartbeat` by 1) and `python3 scripts/dev/no_panic_check.py .`.

- [ ] **Step 7: Commit** — paths `crates/presenter-server/src/state/listen.rs crates/presenter-server/src/state/mod.rs crates/presenter-server/src/state/background_tasks.rs crates/presenter-server/src/state/tests.rs crates/presenter-server/src/router/listen.rs crates/presenter-server/src/router.rs crates/presenter-server/src/router/request_log.rs tests/e2e/support.ts`, message `feat(listen): GET|PUT /listen/mics and GET /listen/status, receiver started with the background tasks (#852 C1a-2)`.

### Task 8: The box sender, its CI step and the docs

The real streams come with the owner's routing (spec §20). Until then nothing sends VBAN to a deployed Presenter, so the post-deploy functional check needs a sender: small stdlib Python, built from the ported header layout and pinned against the Rust decoder by the shared golden packet (C2's bake-off can extend it with a WAV replay). The capture checker belongs to the box acceptance and lands there (see "After C1b and the owner's routing" at the end, review 3).

**Files:**
- Create: `scripts/dev/listen/vban_send.py`, `scripts/dev/listen/test_vban_send.py`
- Modify: `.github/workflows/pipeline.yml` (one step in the `quality` job, after `Test no-panic gate (section 10)`), `docs/configuration.md` (a section after "Stream playback — Google Drive folders (#843)"), `CLAUDE.md` (an Environment Variables row after `PRESENTER_PLAYBACK_MAX_GB`)

**Interfaces:**
- `vban_send.py --to HOST:PORT --stream NAME [--seconds 3] [--tone 1000] [--amplitude 0.5] [--rate 48000] [--channels 1] [--frames 256] [--burst 4] [--skip N]…` — INT16 tone in real time; prints the packet count. Functions `stream_name_bytes`, `int16`, `build_packet`, `tone`, `packets`, `send`.

- [ ] **Step 1: Write the tests** (plain `python3 -m unittest`, no third-party module). Run while planning: 8 tests green, and a loopback smoke (`--seconds 0.5 --skip 10` → 92 packets received, counter 10 absent).

`scripts/dev/listen/test_vban_send.py`:

```python
"""Tests for the VBAN test sender (#852 C1a). Run:
python3 -m unittest discover -s scripts/dev/listen -p 'test_vban_send.py' -v
"""

import math
import unittest

import vban_send

# The same packet `golden_packet_decodes_and_the_encoder_rebuilds_it` pins in
# crates/presenter-core/src/audio/vban/tests.rs: INT16 mono 48 kHz, 4 frames
# [0.0, 0.5, -0.25, 1.5], stream GUEST_EN, counter 0x01020304.
GOLDEN = (
    "5642414e"  # VBAN
    "03030001"  # SR index 3 (48 kHz), 4 frames, 1 channel, INT16 PCM
    "47554553545f454e"  # "GUEST_EN"
    "0000000000000000"  # name padding
    "04030201"  # counter 0x01020304
    "0000004000e0ff7f"  # 0, 16384, -8192, 32767
)


class VbanSendTest(unittest.TestCase):
    def test_golden_packet_matches_the_rust_decoder_test(self):
        packet = vban_send.build_packet("GUEST_EN", 0x01020304, 48000, 1, [0.0, 0.5, -0.25, 1.5])
        self.assertEqual(packet.hex(), GOLDEN)

    def test_header_fields_of_a_256_frame_mono_packet(self):
        packet = vban_send.build_packet("PRESENTER_TEST", 7, 48000, 1, [0.0] * 256)
        self.assertEqual(len(packet), 28 + 512)
        self.assertEqual(packet[:4], b"VBAN")
        self.assertEqual((packet[4], packet[5], packet[6], packet[7]), (3, 255, 0, 0x01))
        self.assertEqual(packet[8:24], b"PRESENTER_TEST\0\0")
        self.assertEqual(packet[24:28], (7).to_bytes(4, "little"))

    def test_the_sr_index_follows_the_spec_list(self):
        for rate, index in ((48000, 3), (96000, 4), (192000, 5), (44100, 16), (88200, 17), (16000, 8)):
            self.assertEqual(vban_send.build_packet("x", 0, rate, 1, [0.0])[4], index)

    def test_the_stream_name_is_16_ascii_bytes_zero_padded(self):
        self.assertEqual(vban_send.stream_name_bytes("sp-program"), b"sp-program\0\0\0\0\0\0")
        self.assertEqual(vban_send.stream_name_bytes("a-very-long-stream-name"), b"a-very-long-stre")
        self.assertEqual(vban_send.stream_name_bytes("é\tx"), b"__x" + b"\0" * 13)

    def test_int16_conversion_matches_songplayer(self):
        self.assertEqual(vban_send.int16(0.5), 16384)
        self.assertEqual(vban_send.int16(-0.25), -8192)
        self.assertEqual(vban_send.int16(1.5), 32767)
        self.assertEqual(vban_send.int16(-1.0), -32767)
        self.assertEqual(vban_send.int16(math.nan), 0)

    def test_skip_leaves_its_counter_out(self):
        counters = [c for c, _ in vban_send.packets("x", 48000, 1, 256, 0.1, 1000.0, 0.5, {3})]
        self.assertEqual(counters, [0, 1, 2] + list(range(4, 18)))

    def test_the_counter_wraps_at_u32_max(self):
        packet = vban_send.build_packet("x", 2**32 + 5, 48000, 1, [0.0])
        self.assertEqual(packet[24:28], (5).to_bytes(4, "little"))

    def test_bad_geometry_is_refused(self):
        with self.assertRaises(ValueError):
            vban_send.build_packet("x", 0, 44000, 1, [0.0])
        with self.assertRaises(ValueError):
            vban_send.build_packet("x", 0, 48000, 1, [0.0] * 257)
        with self.assertRaises(ValueError):
            vban_send.build_packet("x", 0, 48000, 2, [0.0] * 3)


if __name__ == "__main__":
    unittest.main()
```

- [ ] **Step 2: Implement** — `scripts/dev/listen/vban_send.py`:

```python
#!/usr/bin/env python3
"""Send a test tone as VBAN audio (#852 C1a): the box check of `/listen/status`.

The packet is songplayer's header layout (crates/sp-server/src/playback/vban_packet.rs
@ dd551b7d, VB-Audio "VBAN Protocol Specifications" rev. 13), INT16 PCM:

    'VBAN' | format_SR (SR index, sub-protocol AUDIO 0x00) | nbs = frames - 1 |
    nbc = channels - 1 | format_bit 0x01 (INT16, PCM) | stream name, 16 bytes,
    ASCII, zero padded | nuFrame, u32 LE, +1 per packet | interleaved INT16 LE

Samples use songplayer's symmetric scale (+-1.0 -> +-32767, rounded half away
from zero). Packets go out in real time, `--burst` at a time (Voicemeeter
senders burst 2-4 packets, songplayer `vban-out.md`). `--skip N` leaves the
packet with counter N out: the receiver must count it as lost.

    python3 scripts/dev/listen/vban_send.py --to 10.77.9.205:6980 \
        --stream PRESENTER_TEST --seconds 5 --skip 100
"""

import argparse
import math
import socket
import struct
import sys
import time

SAMPLE_RATES = (
    6000, 12000, 24000, 48000, 96000, 192000, 384000, 8000, 16000, 32000, 64000,
    128000, 256000, 512000, 11025, 22050, 44100, 88200, 176400, 352800, 705600,
)
INT16_PCM = 0x01
NAME_LEN = 16


def stream_name_bytes(name):
    """The 16-byte name field: the first 16 characters, a character that is
    not printable ASCII as `_` (songplayer `stream_name_bytes`), zero padded."""
    chars = (ord(c) if 0x20 <= ord(c) < 0x7F else ord("_") for c in name[:NAME_LEN])
    return bytes(chars).ljust(NAME_LEN, b"\0")


def int16(x):
    """One sample as INT16: NaN is silence, clamped, x 32767, half away from 0."""
    if math.isnan(x):
        return 0
    v = max(-1.0, min(1.0, x)) * 32767.0
    return int(math.floor(v + 0.5)) if v >= 0 else -int(math.floor(-v + 0.5))


def build_packet(name, counter, rate, channels, samples):
    """One INT16 VBAN packet of interleaved `samples` (whole frames)."""
    if rate not in SAMPLE_RATES:
        raise ValueError(f"VBAN carries no {rate} Hz")
    if not 1 <= channels <= 256 or len(samples) % channels:
        raise ValueError("samples must be whole frames of 1-256 channels")
    frames = len(samples) // channels
    if not 1 <= frames <= 256:
        raise ValueError("a packet carries 1-256 frames")
    header = (
        b"VBAN"
        + bytes([SAMPLE_RATES.index(rate), frames - 1, channels - 1, INT16_PCM])
        + stream_name_bytes(name)
        + struct.pack("<I", counter & 0xFFFFFFFF)
    )
    return header + struct.pack(f"<{len(samples)}h", *(int16(x) for x in samples))


def tone(rate, hz, amplitude, start_frame, frames, channels):
    """`frames` frames of a sine starting at `start_frame`, the same in every channel."""
    return [
        amplitude * math.sin(2.0 * math.pi * hz * (start_frame + i) / rate)
        for i in range(frames)
        for _ in range(channels)
    ]


def packets(name, rate, channels, frames, seconds, hz, amplitude, skip=()):
    """Yield (counter, packet) for `seconds` of tone; counters in `skip` are left out."""
    for counter in range(int(rate * seconds) // frames):
        if counter in skip:
            continue
        samples = tone(rate, hz, amplitude, counter * frames, frames, channels)
        yield counter, build_packet(name, counter, rate, channels, samples)


def send(addr, generated, rate, frames, burst):
    """Send in real time, `burst` packets at a time; returns the count sent."""
    sent = 0
    start = time.monotonic()
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
        for counter, packet in generated:
            sock.sendto(packet, addr)
            sent += 1
            if sent % burst == 0:
                due = start + (counter + 1) * frames / rate
                time.sleep(max(0.0, due - time.monotonic()))
    return sent


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--to", required=True, help="HOST:PORT of the receiver")
    parser.add_argument("--stream", required=True, help="VBAN stream name (1-16 ASCII)")
    parser.add_argument("--seconds", type=float, default=3.0)
    parser.add_argument("--tone", type=float, default=1000.0, help="Hz")
    parser.add_argument("--amplitude", type=float, default=0.5)
    parser.add_argument("--rate", type=int, default=48000)
    parser.add_argument("--channels", type=int, default=1)
    parser.add_argument("--frames", type=int, default=256, help="frames per packet")
    parser.add_argument("--burst", type=int, default=4, help="packets per send burst")
    parser.add_argument("--skip", type=int, action="append", default=[], help="leave this counter out")
    args = parser.parse_args(argv)
    host, _, port = args.to.rpartition(":")
    generated = packets(args.stream, args.rate, args.channels, args.frames, args.seconds,
                        args.tone, args.amplitude, set(args.skip))
    sent = send((host, int(port)), generated, args.rate, args.frames, max(1, args.burst))
    print(f"sent {sent} VBAN packets ({args.stream}, {args.rate} Hz, {args.channels} ch, "
          f"{args.frames} frames, skipped {sorted(args.skip)}) to {args.to}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
```

`chmod +x scripts/dev/listen/vban_send.py`; `python3 -m unittest discover -s scripts/dev/listen -p 'test_vban_*.py' -v` → `Ran 8 tests … OK`.

- [ ] **Step 3: The CI step.** In `.github/workflows/pipeline.yml`, `quality` job, after the `Test no-panic gate (section 10)` step (the pattern also picks up the acceptance checker's tests later):

```yaml
      # #852 C1a: the VBAN box tools (scripts/dev/listen). The sender's golden
      # packet is the one the Rust decoder test pins (presenter-core
      # audio/vban/tests.rs).
      - name: Test VBAN box tools (#852)
        run: |
          set -euo pipefail
          python3 -m unittest discover -s scripts/dev/listen -p 'test_vban_*.py' -v
```

Then `python3 scripts/ci/check_deploy_workflows.py` (it reads pipeline.yml; a step without ssh changes nothing) and `python3 -c "import sys, yaml; yaml.safe_load(open(sys.argv[1]))" .github/workflows/pipeline.yml`.

- [ ] **Step 4: The docs.** `docs/configuration.md`, after the "Stream playback" section's table:

```markdown
### Sermon listening — VBAN microphones (#852)

Presenter receives the guest and interpreter microphones as VBAN audio on ONE UDP socket, bound only
while at least one microphone is configured (`listen.mics`, `GET|PUT /listen/mics`). The list ships
EMPTY: no port is bound until the microphones are saved (PP stays empty; SNV gets its two microphones
on the routing day). `GET /listen/status` shows the socket, every microphone's packets, format,
sender and state (`waiting`, `receiving`, `no_audio` 3 s after the last packet) and the streams seen
but not assigned. Senders: VB-Matrix or
Voicemeeter on the FOH PC, 48 kHz, 16-bit, stream names `GUEST_EN` / `INTERP_SK` (spec §5.7).
`scripts/dev/listen/vban_send.py` sends a test tone.

| Variable                     | Default        | Purpose |
| ---------------------------- | -------------- | ------- |
| `PRESENTER_LISTEN_VBAN_ADDR` | `0.0.0.0:6980` | Bind address of the VBAN socket. The E2E harness uses `127.0.0.1:0`, so a test server never takes the dev service's port. |
```

CLAUDE.md Environment Variables row (after `PRESENTER_PLAYBACK_MAX_GB`):

```markdown
| `PRESENTER_LISTEN_VBAN_ADDR`  | `0.0.0.0:6980`          | Bind address of the VBAN receive socket for the listening microphones (#852 C1a); bound only while `listen.mics` is non-empty (it ships empty). The E2E harness uses `127.0.0.1:0`. |
```

- [ ] **Step 5: Commit** — paths `scripts/dev/listen/ .github/workflows/pipeline.yml docs/configuration.md CLAUDE.md`, message `feat(listen): VBAN test sender, its CI step and the listening docs (#852 C1a-2)`.

### Task 9: Close lane C1a-2 — measure, push, verify on dev, SNV and PP

- [ ] **Step 1: Measure the lane** (raw production lines; the review's budget ≤ ~700, the sender included):

```bash
cd "$REPO"
for f in crates/presenter-server/src/listen/mod.rs crates/presenter-server/src/listen/health.rs crates/presenter-server/src/listen/vban_rx.rs crates/presenter-server/src/state/listen.rs crates/presenter-server/src/router/listen.rs; do printf '%s ' "$f"; bash scripts/dev/count_prod_lines.sh "$f"; done
grep -cv '^\s*$' scripts/dev/listen/vban_send.py
```

Expected (the plan's code, before rustfmt) ≈ 100 + 165 + 205 + 75 + 40 + ≈ 15 lines of edits in `state/mod.rs`, `background_tasks.rs`, `router.rs`, `request_log.rs` ≈ 680 non-blank / ≈ 755 raw Rust, plus the ≈ 100-line Python sender (a dev tool). Packets per second and the counted rejects already moved to lane 3 to get here; if rustfmt pushes the Rust past ~760 raw, tell the supervisor before pushing.

- [ ] **Step 2: Local gates**

```bash
cargo fmt --all -- --check
bash scripts/dev/placeholder_check.sh . && bash scripts/dev/feature_router_check.sh .
QC_TARGETS="$(git diff --name-only origin/dev...HEAD -- '*.rs')" python3 scripts/dev/fn_length_check.py .
for f in $(git diff --name-only origin/dev...HEAD -- '*.rs'); do printf '%s ' "$f"; bash scripts/dev/count_prod_lines.sh "$f"; done
python3 scripts/dev/no_panic_check.py . && python3 -m unittest discover -s scripts/dev -p 'test_no_panic_check.py'
python3 -m unittest discover -s scripts/dev/listen -p 'test_vban_*.py'
python3 scripts/ci/check_deploy_workflows.py
node ../../../node_modules/typescript/bin/tsc --noEmit -p tsconfig.json   # from a worktree root
git status --short    # nothing unexpected
```

- [ ] **Step 3: Push and watch CI.** Load `ci-push-discipline`; `git fetch origin && git merge origin/dev`; push as the supervisor directs; monitor `pipeline.yml` to a terminal state — every job: Format, Clippy (`--all-features`), Test, Quality (incl. the new Python step), Coverage, Build, the E2E shards, deploy-dev. A red job: `gh run view <id> --log-failed`, root cause, ONE fix commit.

- [ ] **Step 4: Verify on dev (`$DEV`, the runner host).** Dev's journal filter hides `presenter::listen::mic` (observability rule) — the check reads the API. The list is empty (dev copies prod's DB, prod's is empty), so the test mic is saved and removed again:

```bash
curl -fsS "$DEV/healthz" | python3 -c 'import json,sys; print(json.load(sys.stdin)["version"])'
curl -fsS "$DEV/listen/status" | python3 -m json.tool        # socket "idle", mics []
curl -fsS -X PUT "$DEV/listen/mics" -H 'content-type: application/json' \
  -d '[{"id":"test","name":"Test C1a","streamName":"PRESENTER_TEST","senderIp":null,"channel":"left","language":"off","role":"guest"}]'
python3 scripts/dev/listen/vban_send.py --to "$DEV_HOST:6980" --stream PRESENTER_TEST --seconds 3
python3 - "$DEV" <<'EOF'
import json, sys, time, urllib.request
def status():
    with urllib.request.urlopen(f"{sys.argv[1]}/listen/status", timeout=5) as reply:
        return json.load(reply)
s = status()
mic = s["mics"][0]
print(s["socket"]["state"], {k: mic.get(k) for k in ("state", "packets", "packetsPerSec", "sender",
      "framesPerPacket", "format", "lostPackets", "framesOut", "levelDbfs", "voiced")})
time.sleep(3.5)
print("3.5 s later:", status()["mics"][0]["state"])
EOF
curl -fsS -X PUT "$DEV/listen/mics" -H 'content-type: application/json' -d '[]'
curl -fsS "$DEV/listen/status" | python3 -c 'import json,sys; print(json.load(sys.stdin)["socket"]["state"])'   # idle
```

Expected (3 s at 48 kHz in 256-frame packets = 562 packets): socket `bound`; `receiving`, `packets` 562, `framesPerPacket` 256, `format` `{48000, 1, int16}`, `sender` = this box's address; the reader prints `None` for the lane-3 fields (`packetsPerSec`, `lostPackets`, `framesOut`, `levelDbfs`, `voiced`); then `no_audio`; after the empty save `idle`.

- [ ] **Step 5: Release (the supervisor's call) and verify SNV after `deploy.yml`** (load `post-deploy-verification`): `/healthz` version and the operator footer `v<VERSION>` from the live DOM (Playwright, zero console errors); `curl -fsS "$SNV/listen/status"` → socket `idle`, `mics` `[]` (nothing bound before the routing day); then the same save → `vban_send.py --to 10.77.9.205:6980 --stream PRESENTER_TEST --seconds 5` (937 packets) → status reader with `"$SNV"` → `ssh -o BatchMode=yes newlevel@presenter.lan "journalctl -u presenter --since '-5 min' --no-pager | grep 'presenter::listen::mic'"` (lines `listen mics saved`, `VBAN socket bound`, `mic receives VBAN` with `mic="test"` `stream="PRESENTER_TEST"` `rate=48000`, `mic lost audio — no VBAN packet for 3 s`) → the empty save again → socket `idle` and `VBAN socket closed` in the journal (port 6980 released). If dev received and SNV shows nothing: a read-only look at the box firewall (`ssh … "sudo -n ufw status verbose; sudo -n nft list ruleset 2>/dev/null | grep -n 6980"`), record the finding on #852, change nothing on the prod box without the owner's go.

- [ ] **Step 6: PP** — after the release: `/healthz` version; `curl -fsS "$PP/listen/status"` → socket `idle`, `mics` `[]`; `curl -fsS "$PP/listen/mics"` → `[]` (C-S8).

- [ ] **Step 7: Close.** `playbook-review` (`📔 Playbook:` line); comment on #852 with the readings; next lane C1a-3.

---

## PR unit C1a-3 — the chain

Starts from `origin/dev` with C1a-2 merged. Version: dev is unreleased; no bump unless main caught up.

### Task 10: The frame-counter rule

**Files:**
- Modify: `crates/presenter-core/src/audio/mod.rs` (add `pub mod sequence;`)
- Create: `crates/presenter-core/src/audio/sequence.rs` (tests inline)

**Interfaces:**
- Produces: `presenter_core::audio::sequence::{SequenceVerdict { InOrder, Gap { missing: u32, silence_frames: usize }, Late, Resync }, StreamSequencer { observe(&mut self, counter: u32, frames: usize, rate_hz: u32) -> SequenceVerdict, reset(&mut self) }}` — used by `MicInput` in Task 13.

- [ ] **Step 1: The Python mirror** (the edge values the tests use):

```bash
python3 - <<'EOF'
M = 2**32
print("packets/s 48k/256:", 48000 // 256, " 44.1k/256:", 44100 // 256, " 48k/64:", 48000 // 64)
print("wrap ahead (expected MAX-1, counter 1):", (1 - (M - 2)) % M)
EOF
```

Expected: `187  172  750`, `3`.

- [ ] **Step 2: Implement + test the counter rule** — `crates/presenter-core/src/audio/sequence.rs`:

```rust
//! The frame-counter rule of one VBAN stream (#852 C1a, spec §5.3). Pure.
//!
//! - the expected counter → use the packet;
//! - ahead by packets totalling ≤ 1 s → that many were lost: insert their
//!   length of silence, then use the packet;
//! - behind by ≤ 1 s (late or a duplicate) → drop it;
//! - a jump of more than 1 s either way (a sender restart) → use it and start
//!   counting from it.
//!
//! "1 s" is counted in packets of the CURRENT packet's length (`rate / frames`:
//! 187 packets of 256 frames at 48 kHz). The counter wraps at `u32::MAX`
//! (songplayer's `VbanEncoder`), so all arithmetic is wrapping. No jitter
//! buffer: the ASR has no playout clock, and Voicemeeter's bursts of 2–4
//! packets are normal (songplayer `vban-out.md`).

/// What to do with one packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SequenceVerdict {
    /// The stream's first packet, or the expected one.
    InOrder,
    /// `missing` packets were lost; `silence_frames` of silence go before this one.
    Gap { missing: u32, silence_frames: usize },
    /// Late or a duplicate: dropped.
    Late,
    /// A jump of more than a second (a sender restart): used, counting starts over.
    Resync,
}

/// One stream's expected counter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StreamSequencer {
    expected: Option<u32>,
}

impl StreamSequencer {
    /// The verdict for a packet with `counter` carrying `frames` frames at `rate_hz`.
    pub fn observe(&mut self, counter: u32, frames: usize, rate_hz: u32) -> SequenceVerdict {
        let Some(expected) = self.expected else {
            self.expected = Some(counter.wrapping_add(1));
            return SequenceVerdict::InOrder;
        };
        let second = packets_per_second(frames, rate_hz);
        let ahead = counter.wrapping_sub(expected);
        if ahead == 0 {
            self.expected = Some(counter.wrapping_add(1));
            return SequenceVerdict::InOrder;
        }
        if ahead <= second {
            self.expected = Some(counter.wrapping_add(1));
            return SequenceVerdict::Gap { missing: ahead, silence_frames: ahead as usize * frames };
        }
        if expected.wrapping_sub(counter) <= second {
            return SequenceVerdict::Late;
        }
        self.expected = Some(counter.wrapping_add(1));
        SequenceVerdict::Resync
    }

    /// Forget the stream (a new sender): the next packet is a first packet.
    pub fn reset(&mut self) {
        self.expected = None;
    }
}

/// Whole packets of `frames` in one second at `rate_hz` (at least 1).
fn packets_per_second(frames: usize, rate_hz: u32) -> u32 {
    let frames = u32::try_from(frames.max(1)).unwrap_or(u32::MAX);
    (rate_hz / frames).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use SequenceVerdict::{Gap, InOrder, Late, Resync};

    const RATE: u32 = 48_000;
    /// 187 packets per second (the Python mirror: 48000 // 256).
    const FRAMES: usize = 256;

    fn run(counters: &[u32]) -> Vec<SequenceVerdict> {
        let mut sequencer = StreamSequencer::default();
        counters.iter().map(|&c| sequencer.observe(c, FRAMES, RATE)).collect()
    }

    #[test]
    fn consecutive_counters_are_in_order() {
        assert_eq!(run(&[7, 8, 9]), vec![InOrder; 3]);
    }

    #[test]
    fn a_gap_inserts_the_missing_packets_as_silence() {
        assert_eq!(
            run(&[0, 1, 4]),
            vec![InOrder, InOrder, Gap { missing: 2, silence_frames: 512 }]
        );
    }

    #[test]
    fn exactly_one_second_ahead_is_still_a_gap_and_one_more_is_a_resync() {
        assert_eq!(
            run(&[0, 188]),
            vec![InOrder, Gap { missing: 187, silence_frames: 187 * 256 }]
        );
        assert_eq!(run(&[0, 189]), vec![InOrder, Resync]);
        assert_eq!(run(&[0, 189, 190]), vec![InOrder, Resync, InOrder], "counting goes on");
    }

    #[test]
    fn late_and_duplicate_packets_are_dropped_without_moving_the_expectation() {
        assert_eq!(
            run(&[0, 1, 2, 2, 1, 3]),
            vec![InOrder, InOrder, InOrder, Late, Late, InOrder]
        );
    }

    #[test]
    fn a_second_behind_is_late_and_more_is_a_resync() {
        // expected 300 after 299: 300 − 187 = 113 is late, 112 is a restart.
        assert_eq!(run(&[299, 113]), vec![InOrder, Late]);
        assert_eq!(run(&[299, 112, 113]), vec![InOrder, Resync, InOrder]);
    }

    #[test]
    fn the_counter_wraps_at_u32_max() {
        assert_eq!(run(&[u32::MAX - 1, u32::MAX, 0, 1]), vec![InOrder; 4]);
        // the Python mirror: (1 − (2**32 − 2)) % 2**32 == 3
        assert_eq!(
            run(&[u32::MAX - 2, 1]),
            vec![InOrder, Gap { missing: 3, silence_frames: 768 }]
        );
        assert_eq!(run(&[1, u32::MAX]), vec![InOrder, Late], "behind across the wrap");
    }

    #[test]
    fn one_second_is_counted_in_packets_of_the_current_length() {
        let mut sequencer = StreamSequencer::default();
        assert_eq!(sequencer.observe(0, 64, 48_000), InOrder);
        assert_eq!(
            sequencer.observe(751, 64, 48_000),
            Gap { missing: 750, silence_frames: 48_000 },
            "48000 / 64 = 750 packets"
        );
        assert_eq!(packets_per_second(256, 44_100), 172);
        assert_eq!(packets_per_second(0, 48_000), 48_000, "0 frames counts as 1");
    }

    #[test]
    fn reset_makes_the_next_packet_a_first_packet() {
        let mut sequencer = StreamSequencer::default();
        sequencer.observe(5, FRAMES, RATE);
        sequencer.reset();
        assert_eq!(sequencer.observe(9_000, FRAMES, RATE), InOrder);
        assert_eq!(sequencer.observe(9_001, FRAMES, RATE), InOrder);
    }
}
```

`crates/presenter-core/src/audio/mod.rs` now lists `pub mod level; pub mod sequence; pub mod vban;` and the gated `vban_encode` (keep the module doc comment).

- [ ] **Step 3: How it is verified.** CI `Test` job; the edges come from the Step 1 mirror.

- [ ] **Step 4: Gates + commit.** `cargo fmt --all -- --check`; `QC_TARGETS` with `sequence.rs`; commit paths `crates/presenter-core/src/audio/`, message `feat(core): the VBAN frame-counter rule (#852 C1a-3)`.

### Task 11: rubato and the 16 kHz resampler

**Files:**
- Modify: `crates/presenter-server/Cargo.toml` (`rubato` after `read-fonts`), `Cargo.lock`, `crates/presenter-server/src/listen/mod.rs` (`mod resample;`)
- Create: `crates/presenter-server/src/listen/resample.rs` (tests inline)

**Interfaces:**
- Consumes: rubato `=5.0.1` (`Fft`, `FixedSync`, `Resampler`, `audioadapter_buffers::direct::InterleavedSlice`), `presenter_core::audio::vban::f32_to_int16`.
- Produces (`crate::listen::resample`): `OUTPUT_RATE_HZ: u32 = 16_000`, `FRAME_SAMPLES: usize = 1_600`, `struct Resampler16k { new(rate_hz: u32) -> Result<Self, String>, push(&mut self, mono: &[f32]) -> Vec<Vec<i16>>, #[cfg(test)] delay_frames(&self) -> usize }`.

- [ ] **Step 1: The manifest.** In `crates/presenter-server/Cargo.toml` `[dependencies]`, after the `read-fonts = "0.44"` entry (tokio's `net` and the core `test-helpers` dev-dependency came with lane 2):

```toml
# #852 C1a: VBAN mic → 16 kHz mono for the ASR (listen/resample.rs) — songplayer's
# converter (rubato's synchronous `Fft`, `FixedSync::Both`; sp-server
# playback/vban_rate.rs @ dd551b7d), pinned exactly like songplayer. Pure Rust,
# MIT OR Apache-2.0 (and its deps realfft / rustfft / audioadapter*, MIT or
# MIT OR Apache-2.0 — all on deny.toml's allow list).
rubato = "=5.0.1"
```

- [ ] **Step 2: Resolve rubato into the lockfile without compiling**

```bash
cd "$REPO" && cargo metadata --format-version 1 > /dev/null   # online once: fetches the index, compiles nothing
git diff Cargo.lock | grep '^+name = ' | sort
```

Expected exactly the new packages `audioadapter` (5.0.0), `audioadapter-buffers` (5.2.0), `audioadapter-sample` (5.2.0, a dependency of audioadapter-buffers — review 1e), `num-complex`, `primal-check`, `realfft`, `rubato` (5.0.1), `rustfft`, `strength_reduce`, `transpose`, `visibility`, `windowfunctions` (`num-integer` / `num-traits` are already locked), and no changed version of an existing package (`git diff Cargo.lock | grep '^-version'` prints nothing). rubato 5.0.1 and audioadapter declare `rust-version = 1.87`: the workspace uses resolver 2 with no `rust-version`, CI builds on stable, and `clippy.toml`'s msrv only lints OUR code, so this is fine (noted for the reviewer). Anything else in the diff → stop and look.

- [ ] **Step 3: The module line.** In `crates/presenter-server/src/listen/mod.rs`, after `pub(crate) mod health;`:

```rust
mod resample;
```

(`Resampler16k` gets its production caller, `MicChain`, in Task 12 — the same lane push.)

- [ ] **Step 4: Write the failing tests** — the `#[cfg(test)]` block that ends `crates/presenter-server/src/listen/resample.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use presenter_core::audio::vban::VBAN_SAMPLE_RATES;

    /// `seconds` of a mono sine at `rate_hz`.
    fn sine(rate_hz: u32, hz: f64, amplitude: f64, seconds: f64) -> Vec<f32> {
        let n = (f64::from(rate_hz) * seconds) as usize;
        (0..n)
            .map(|k| {
                let phase = std::f64::consts::TAU * hz * k as f64 / f64::from(rate_hz);
                (amplitude * phase.sin()) as f32
            })
            .collect()
    }

    /// Fed in pieces of 256 frames (one VBAN packet's worth).
    fn convert(resampler: &mut Resampler16k, input: &[f32]) -> Vec<Vec<i16>> {
        input.chunks(256).flat_map(|piece| resampler.push(piece)).collect()
    }

    #[test]
    fn a_16k_stream_passes_through_unconverted() {
        let mut resampler = Resampler16k::new(16_000).unwrap();
        let frames = resampler.push(&[0.5; 3_200]);
        assert_eq!(frames.len(), 2);
        assert!(frames.iter().flatten().all(|&s| s == 16_384));
        assert_eq!(resampler.delay_frames(), 0);
    }

    #[test]
    fn every_supported_rate_gives_exact_100_ms_frames() {
        // songplayer `every_other_rate_gives_exactly_rate_over_30_frames_per_block`,
        // for every rate of the VBAN list (the Python mirror in the plan's
        // Task 4 Step 6 checks rubato's sizing rule for each).
        let rates = VBAN_SAMPLE_RATES.iter().filter(|&&rate| rate != 11_025 && rate != 16_000);
        for &rate in rates {
            let mut resampler = Resampler16k::new(rate).unwrap_or_else(|err| panic!("{rate}: {err}"));
            let frames = convert(&mut resampler, &sine(rate, 440.0, 0.5, 1.0));
            assert_eq!(frames.len(), 10, "{rate} Hz: 1 s in = 10 frames out");
            assert!(frames.iter().all(|frame| frame.len() == FRAME_SAMPLES), "{rate}");
            assert_eq!(resampler.delay_frames(), 160, "{rate}: half of the 320-frame FFT");
        }
    }

    #[test]
    fn eleven_kilohertz_and_zero_are_refused_with_a_slovak_reason() {
        let err = Resampler16k::new(11_025).err().unwrap();
        assert_eq!(err, "nepodporovaná vzorkovacia frekvencia 11025 Hz");
        assert!(Resampler16k::new(0).is_err());
    }

    #[test]
    fn a_1khz_tone_stays_1khz_from_48k_to_16k() {
        // songplayer `a_1khz_tone_stays_1khz_at_96k`, here 48 → 16 kHz.
        let mut resampler = Resampler16k::new(48_000).unwrap();
        let out: Vec<i16> = convert(&mut resampler, &sine(48_000, 1_000.0, 0.5, 1.0)).concat();
        let steady = &out[1_600..];
        assert_eq!(steady.len(), 14_400, "0.9 s past the first frame");
        let rising = steady.windows(2).filter(|w| w[0] < 0 && w[1] >= 0).count();
        assert!((898..=902).contains(&rising), "{rising} rising zero crossings in 0.9 s");
        let peak = steady.iter().map(|s| s.unsigned_abs()).max().unwrap();
        assert!((14_745..=18_022).contains(&peak), "the level is kept: {peak}");
    }

    #[test]
    fn the_delay_is_a_constant_160_samples() {
        // A Gaussian pulse (σ = 0.5 ms, far inside the 7.66 kHz pass band) at
        // t = 0.25 s comes out at 0.25 s × 16 kHz + 160 = sample 4160.
        let mut resampler = Resampler16k::new(48_000).unwrap();
        let pulse: Vec<f32> = (0..24_000)
            .map(|k| {
                let t = (f64::from(k) / 48_000.0 - 0.25) / 0.000_5;
                (0.8 * (-0.5 * t * t).exp()) as f32
            })
            .collect();
        let out: Vec<i16> = convert(&mut resampler, &pulse).concat();
        let peak_at = out
            .iter()
            .enumerate()
            .max_by_key(|(_, sample)| sample.unsigned_abs())
            .map(|(index, _)| index)
            .unwrap();
        assert!((4_159..=4_161).contains(&peak_at), "pulse peak at {peak_at}, expected 4160");
        assert_eq!(resampler.delay_frames(), 160);
    }

    #[test]
    fn silence_in_is_silence_out() {
        let mut resampler = Resampler16k::new(44_100).unwrap();
        let frames = convert(&mut resampler, &[0.0; 22_050]);
        assert_eq!(frames.len(), 5);
        assert!(frames.iter().flatten().all(|&s| s == 0));
    }

    #[test]
    fn odd_piece_sizes_still_give_whole_frames() {
        let mut resampler = Resampler16k::new(48_000).unwrap();
        let input = sine(48_000, 440.0, 0.5, 0.5);
        let frames: Vec<Vec<i16>> = input.chunks(97).flat_map(|piece| resampler.push(piece)).collect();
        assert_eq!(frames.len(), 5);
    }
}
```

- [ ] **Step 5: Implement** — `crates/presenter-server/src/listen/resample.rs` (before the test block):

```rust
//! A mic's converter to 16 kHz mono s16 (#852 C1a, spec §5.4).
//!
//! songplayer's converter pattern (`crates/sp-server/src/playback/vban_rate.rs`
//! @ dd551b7d): rubato's synchronous `Fft` with BOTH sides fixed, so every
//! input chunk gives an exact number of output frames; built in its own fn
//! (cargo-mutants never mutates `new`); a refusal is a reason string; a chunk
//! rubato refuses goes out as silence. Two deliberate differences:
//! - the chunk is 20 ms of input (`rate / 50` frames → 320 frames at 16 kHz),
//!   not one 100 ms block: with `FixedSync::Both` the FFT is the whole chunk
//!   and the delay half of it, so a 100 ms chunk would add 50 ms, and spec
//!   §8.5 budgets ≤ 120 ms for the 100 ms framing + the resampler (here
//!   100 + 10 ms). rubato's cut-off at a 320-frame block is 0.958 × 8 kHz ≈
//!   7.66 kHz (`calculate_cutoff`, BlackmanHarris2) — plenty for speech;
//! - mono in, mono out (`mic_chain` picks the channel first).
//!
//! 16 kHz passes through; every other rate of the VBAN list whose 20 ms is a
//! whole number of frames is converted (all but 11 025 Hz, refused). Output:
//! 100 ms frames of 1600 samples, `f32_to_int16` (songplayer's symmetric scale).

use presenter_core::audio::vban::f32_to_int16;
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};

/// The ASR input rate (spec §5.4: Soniox and Gemini Live take 16 kHz s16le).
pub(crate) const OUTPUT_RATE_HZ: u32 = 16_000;
/// One 100 ms frame at 16 kHz.
pub(crate) const FRAME_SAMPLES: usize = 1_600;
const CHUNKS_PER_SECOND: u32 = 50;
const CHUNK_OUT: usize = (OUTPUT_RATE_HZ / CHUNKS_PER_SECOND) as usize;

pub(crate) struct Resampler16k {
    /// `None` = a 16 kHz stream, passed through.
    fft: Option<Fft<f32>>,
    chunk_in: usize,
    pending_in: Vec<f32>,
    chunk_out: Vec<f32>,
    pending_out: Vec<i16>,
}

impl Resampler16k {
    pub(crate) fn new(rate_hz: u32) -> Result<Self, String> {
        let fft = if rate_hz == OUTPUT_RATE_HZ { None } else { Some(build_fft(rate_hz)?) };
        Ok(Self {
            fft,
            chunk_in: (rate_hz / CHUNKS_PER_SECOND) as usize,
            pending_in: Vec::new(),
            chunk_out: vec![0.0; CHUNK_OUT],
            pending_out: Vec::with_capacity(2 * FRAME_SAMPLES),
        })
    }

    /// Mono samples at the input rate in; every completed 100 ms frame out.
    pub(crate) fn push(&mut self, mono: &[f32]) -> Vec<Vec<i16>> {
        match self.fft.as_mut() {
            None => self.pending_out.extend(mono.iter().map(|&x| f32_to_int16(x))),
            Some(fft) => {
                self.pending_in.extend_from_slice(mono);
                let mut used = 0;
                while self.pending_in.len() - used >= self.chunk_in {
                    let chunk = &self.pending_in[used..used + self.chunk_in];
                    if convert_chunk(fft, chunk, &mut self.chunk_out).is_some() {
                        self.pending_out.extend(self.chunk_out.iter().map(|&x| f32_to_int16(x)));
                    } else {
                        let silent = self.pending_out.len() + CHUNK_OUT;
                        self.pending_out.resize(silent, 0);
                    }
                    used += self.chunk_in;
                }
                self.pending_in.drain(..used);
            }
        }
        self.take_frames()
    }

    /// The converter's delay in 16 kHz samples (0 when passing through).
    #[cfg(test)]
    pub(crate) fn delay_frames(&self) -> usize {
        self.fft.as_ref().map_or(0, |fft| fft.output_delay())
    }

    fn take_frames(&mut self) -> Vec<Vec<i16>> {
        let whole = self.pending_out.len() / FRAME_SAMPLES;
        (0..whole)
            .map(|_| self.pending_out.drain(..FRAME_SAMPLES).collect())
            .collect()
    }
}

/// rubato's converter for `rate_hz` → 16 kHz, 20 ms chunks, or why not.
fn build_fft(rate_hz: u32) -> Result<Fft<f32>, String> {
    let refused = format!("nepodporovaná vzorkovacia frekvencia {rate_hz} Hz");
    if rate_hz == 0 || rate_hz % CHUNKS_PER_SECOND != 0 {
        return Err(refused);
    }
    let chunk_in = (rate_hz / CHUNKS_PER_SECOND) as usize;
    let fft = Fft::<f32>::new(rate_hz as usize, OUTPUT_RATE_HZ as usize, chunk_in, 1, FixedSync::Both)
        .map_err(|err| format!("{refused}: {err}"))?;
    if fft.input_frames_next() == chunk_in && fft.output_frames_next() == CHUNK_OUT {
        Ok(fft)
    } else {
        Err(refused)
    }
}

/// One chunk through rubato (songplayer's adapter calls); `None` = refused.
fn convert_chunk(fft: &mut Fft<f32>, input: &[f32], out: &mut [f32]) -> Option<()> {
    let frames_out = out.len();
    let adapter_in = InterleavedSlice::new(input, 1, input.len()).ok()?;
    let mut adapter_out = InterleavedSlice::new_mut(out, 1, frames_out).ok()?;
    fft.process_into_buffer(&adapter_in, &mut adapter_out, None).ok()?;
    Some(())
}
```

- [ ] **Step 6: The Python mirror of rubato's sizing rule** (rubato 5.0.1 `Fft::new_custom` + `calc_chunk_sizes` for `FixedSync::Both`, read from the crate source): every VBAN rate but 11 025 gives exactly `rate/50` in → 320 out, delay 160.

```bash
python3 - <<'EOF'
from math import gcd
rates = [6000, 12000, 24000, 48000, 96000, 192000, 384000, 8000, 16000, 32000, 64000,
         128000, 256000, 512000, 11025, 22050, 44100, 88200, 176400, 352800, 705600]
for r in rates:
    if r == 16000 or r % 50:
        print(r, "passthrough" if r == 16000 else "refused"); continue
    cs = r // 50; g = gcd(r, 16000); mi, mo = r // g, 16000 // g
    fc = max(-(-cs // mi), 1); fi, fo = fc * mi, fc * mo
    sub = -(-cs // fi); cin, cout = sub * fi, sub * fo
    print(r, cs, cin == cs and cout == 320, "delay", fo // 2)
EOF
```

Expected: `True delay 160` for the 19 converted rates, `passthrough` for 16000, `refused` for 11025.

- [ ] **Step 7: How it is verified.** CI `Test` job (`resample::tests`), the Python mirror for the sizes.

- [ ] **Step 8: Gates + commit.** `cargo fmt --all -- --check`; `count_prod_lines.sh crates/presenter-server/src/listen/resample.rs` (≈ 95); `QC_TARGETS` it; `python3 scripts/dev/no_panic_check.py .`. Commit paths `crates/presenter-server/Cargo.toml Cargo.lock crates/presenter-server/src/listen/mod.rs crates/presenter-server/src/listen/resample.rs`, message `feat(listen): 16 kHz resampler on songplayer's rubato converter pattern (#852 C1a-3)`.

### Task 12: The per-mic chain and its counters

**Files:**
- Modify: `crates/presenter-core/src/listen/status.rs` (`MicState::Error`, `packets_per_sec` + the counter-rule and chain fields of `MicStatus`, `RejectedSender` + `ListenStatus::rejected`, the wire test)
- Modify: `crates/presenter-server/src/listen/health.rs` (the counter rule's and the chain's counters), `crates/presenter-server/src/listen/mod.rs` (add `mod mic_chain;` after `pub(crate) mod health;`)
- Create: `crates/presenter-server/src/listen/mic_chain.rs` (tests inline)
- Test: `crates/presenter-server/src/listen/tests.rs` (append)

**Interfaces:**
- Consumes: `Resampler16k` (Task 11), core `rms_dbfs`, `SequenceVerdict`, `MicChannel`, `VbanAudioFormat`; `ListenService::with_health`.
- Produces (core): `MicState::Error`; `struct RejectedSender { sender, not_vban, non_audio, unsupported_codec, unsupported_type, corrupt, last_packet_age_ms }` and `ListenStatus::rejected: Vec<RejectedSender>`; `MicStatus` gains `packets_per_sec: f32`, `lost_packets`, `late_packets`, `resyncs`, `dropped_blocks`, `other_sender_packets`, `frames_out: u64`, `level_dbfs: Option<f32>`, `voiced: bool`, `error: Option<String>` (camelCase on the wire).
- Produces (`crate::listen::health`): the counters moved out of lane 2 to keep it within its size budget — `on_tick(now)` (packets per second) and `note_reject(ip, reject, now) -> u64` (the sender's reject total; the `rejected` list, 16 most recent); `ListenHealth::set_mics(kept, fresh) -> u64` (now returns the update's generation: fresh mics carry it, kept ones keep theirs); `record_packet(id, PacketSeen, SequenceVerdict, dropped_block: bool) -> PacketTally`; `struct PacketTally { events: u64, dropped_blocks: u64 }` (the counts after the packet, for the #484 gate — no second set of event counters in `MicInput`, review 1d); `record_other_sender(id) -> u64`; `record_chain(id, generation, frames, level, error)`.
- Produces (`crate::listen::mic_chain`): `QUEUE_CAPACITY = 64`; `struct Block { format, silence_frames: usize, samples: Vec<f32> }`; `struct BlockQueue { new(capacity), push(Block) -> bool /* dropped the oldest */, async pop() -> Option<Block>, close(), #[cfg(test)] peek() -> Vec<(usize, usize)> }`; `fn pick_channel(&[f32], channels: u16, MicChannel) -> Vec<f32>`; `struct MicChain { new(MicChannel), push(&Block) -> Vec<Vec<i16>>, format(), error() }`; `async fn run_chain(ListenService, mic_id: String, MicChannel, generation: u64, Arc<BlockQueue>)`. The frames are counted (`framesOut`) and the last one's level kept (`levelDbfs`); nothing else receives them until C1b / C3 add the channel (review 2).

- [ ] **Step 1: The status fields (core).** In `crates/presenter-core/src/listen/status.rs`, `MicState` gains, after `NoAudio,`:

```rust
    /// Packets arrive, but the converter refused their format.
    Error,
```

`MicStatus` gains `pub packets_per_sec: f32,` after `pub packets: u64,`, and, after `pub last_packet_age_ms: Option<u64>,`:

```rust
    pub lost_packets: u64,
    pub late_packets: u64,
    pub resyncs: u64,
    pub dropped_blocks: u64,
    pub other_sender_packets: u64,
    /// 100 ms frames of 16 kHz produced.
    pub frames_out: u64,
    /// The last frame's RMS level; only while receiving.
    pub level_dbfs: Option<f32>,
    pub voiced: bool,
    /// Slovak, for the operator (e.g. an unsupported sample rate).
    pub error: Option<String>,
```

After `UnmatchedStream`:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RejectedSender {
    /// The sender's IP.
    pub sender: String,
    pub not_vban: u64,
    pub non_audio: u64,
    pub unsupported_codec: u64,
    pub unsupported_type: u64,
    pub corrupt: u64,
    pub last_packet_age_ms: u64,
}
```

and `ListenStatus` gains `pub rejected: Vec<RejectedSender>,` after `unmatched`; the module doc's "lane C1a-3 adds the refused senders and its counters" becomes "and the senders whose datagrams were refused".

In `the_status_wire_shape_is_exact`, the `ListenStatus` literal gains `rejected: vec![RejectedSender { sender: "198.51.100.7".into(), not_vban: 1, non_audio: 2, unsupported_codec: 0, unsupported_type: 0, corrupt: 0, last_packet_age_ms: 10 }],` and the JSON `"rejected": [{"sender": "198.51.100.7", "notVban": 1, "nonAudio": 2, "unsupportedCodec": 0, "unsupportedType": 0, "corrupt": 0, "lastPacketAgeMs": 10}]`; the `MicStatus` literal gains `packets_per_sec: 0.0,` and `lost_packets: 1, late_packets: 0, resyncs: 0, dropped_blocks: 0, other_sender_packets: 0, frames_out: 49, level_dbfs: None, voiced: false, error: None,` and the expected mic JSON gains `"packetsPerSec": 0.0, "lostPackets": 1, "latePackets": 0, "resyncs": 0, "droppedBlocks": 0, "otherSenderPackets": 0, "framesOut": 49, "levelDbfs": null, "voiced": false, "error": null`; add `assert_eq!(serde_json::to_value(MicState::Error).unwrap(), json!("error"));`. Grep the other construction sites before pushing (E0063, `quality-gates.md`): `grep -rn 'MicStatus {' crates/` → only `health.rs::mic_status` and that test.

- [ ] **Step 2: Write the failing tests.** The block that ends `crates/presenter-server/src/listen/mic_chain.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::listen::health::PacketSeen;
    use presenter_core::audio::sequence::SequenceVerdict;
    use presenter_core::audio::vban::VbanSampleType;
    use presenter_core::audio::vban_encode::tone;
    use presenter_core::listen::mics::{MicConfig, MicLanguage, MicRole};
    use std::time::Duration;
    use tokio::time::Instant;

    const MONO_48K: VbanAudioFormat =
        VbanAudioFormat { rate_hz: 48_000, channels: 1, sample_type: VbanSampleType::Int16 };
    const STEREO_48K: VbanAudioFormat =
        VbanAudioFormat { rate_hz: 48_000, channels: 2, sample_type: VbanSampleType::Int24 };

    fn block(format: VbanAudioFormat, silence_frames: usize, samples: Vec<f32>) -> Block {
        Block { format, silence_frames, samples }
    }

    #[test]
    fn a_full_queue_drops_the_oldest_block() {
        let queue = BlockQueue::new(3);
        let dropped: Vec<bool> = (0..5).map(|k| queue.push(block(MONO_48K, k, Vec::new()))).collect();
        assert_eq!(dropped, vec![false, false, false, true, true]);
        assert_eq!(queue.peek(), vec![(2, 0), (3, 0), (4, 0)]);
    }

    #[tokio::test]
    async fn pop_waits_for_a_push_and_close_ends_it() {
        let queue = Arc::new(BlockQueue::new(4));
        let popper = {
            let queue = Arc::clone(&queue);
            tokio::spawn(async move {
                let first = queue.pop().await.map(|b| b.silence_frames);
                let second = queue.pop().await.map(|b| b.silence_frames);
                (first, second)
            })
        };
        tokio::task::yield_now().await;
        queue.push(block(MONO_48K, 7, Vec::new()));
        queue.close();
        let popped = tokio::time::timeout(Duration::from_secs(5), popper).await.unwrap().unwrap();
        assert_eq!(popped, (Some(7), None), "the last block first, then the end");
    }

    #[test]
    fn the_channel_pick_follows_the_config() {
        let stereo = [0.25, 0.75, 0.5, 0.0];
        assert_eq!(pick_channel(&stereo, 2, MicChannel::Left), vec![0.25, 0.5]);
        assert_eq!(pick_channel(&stereo, 2, MicChannel::Right), vec![0.75, 0.0]);
        assert_eq!(pick_channel(&stereo, 2, MicChannel::Mix), vec![0.5, 0.25]);
        for pick in [MicChannel::Left, MicChannel::Right, MicChannel::Mix] {
            assert_eq!(pick_channel(&[0.5, -0.5], 1, pick), vec![0.5, -0.5], "mono: {pick:?}");
        }
        assert_eq!(pick_channel(&[0.25, 0.5, 0.75], 3, MicChannel::Mix), vec![0.5]);
    }

    #[test]
    fn the_chain_picks_the_configured_channel() {
        // left silent, right a 1 kHz tone at half scale; 2 × 100 ms
        let stereo: Vec<f32> = tone(48_000, 1_000.0, 0.5, 0, 4_800, 1)
            .into_iter()
            .flat_map(|x| [0.0, x])
            .collect();
        for (pick, expected) in [(MicChannel::Right, -9.03), (MicChannel::Left, -120.0)] {
            let mut chain = MicChain::new(pick);
            let mut frames = chain.push(&block(STEREO_48K, 0, stereo.clone()));
            frames.extend(chain.push(&block(STEREO_48K, 0, stereo.clone())));
            assert_eq!(frames.len(), 2, "{pick:?}: 200 ms in, 2 frames out");
            let level = rms_dbfs(&frames[1]);
            assert!((level - expected).abs() <= 0.3, "{pick:?}: {level}");
        }
    }

    #[test]
    fn inserted_silence_is_part_of_the_output() {
        let mut chain = MicChain::new(MicChannel::Left);
        let mut frames = chain.push(&block(MONO_48K, 4_800, Vec::new()));
        for start in [0, 4_800] {
            frames.extend(chain.push(&block(MONO_48K, 0, tone(48_000, 1_000.0, 0.5, start, 4_800, 1))));
        }
        assert_eq!(frames.len(), 3, "100 ms of inserted silence + 200 ms of tone");
        assert!(rms_dbfs(&frames[0]) < -60.0, "{}", rms_dbfs(&frames[0]));
        assert!((rms_dbfs(&frames[2]) + 9.03).abs() <= 0.3, "{}", rms_dbfs(&frames[2]));
    }

    #[test]
    fn a_format_change_resets_the_converter_and_an_unsupported_rate_says_why() {
        let mut chain = MicChain::new(MicChannel::Left);
        assert_eq!(chain.push(&block(MONO_48K, 0, vec![0.0; 4_800])).len(), 1);
        let at_44k = VbanAudioFormat { rate_hz: 44_100, ..MONO_48K };
        assert_eq!(chain.push(&block(at_44k, 0, vec![0.0; 4_410])).len(), 1, "a fresh converter");
        assert_eq!((chain.format(), chain.error()), (Some(at_44k), None));
        let at_11k = VbanAudioFormat { rate_hz: 11_025, ..MONO_48K };
        assert!(chain.push(&block(at_11k, 0, vec![0.0; 1_102])).is_empty());
        assert_eq!(chain.error(), Some("nepodporovaná vzorkovacia frekvencia 11025 Hz"));
        assert_eq!(chain.push(&block(MONO_48K, 0, vec![0.0; 4_800])).len(), 1);
        assert_eq!(chain.error(), None, "a supported format clears the error");
    }

    #[tokio::test]
    async fn the_chain_task_counts_its_frames_and_keeps_the_last_level() {
        let service = ListenService::new();
        let generation = service.with_health(|health| health.set_mics(&[], &["guest"]));
        let queue = Arc::new(BlockQueue::new(QUEUE_CAPACITY));
        let task = tokio::spawn(run_chain(
            service.clone(),
            "guest".to_string(),
            MicChannel::Left,
            generation,
            Arc::clone(&queue),
        ));
        queue.push(block(MONO_48K, 0, tone(48_000, 1_000.0, 0.5, 0, 9_600, 1)));
        queue.close();
        tokio::time::timeout(Duration::from_secs(5), task).await.unwrap().unwrap();
        let now = Instant::now();
        let seen = PacketSeen { from: "192.0.2.10:6980".parse().unwrap(), format: MONO_48K, frames: 256, at: now };
        service.with_health(|health| health.record_packet("guest", seen, SequenceVerdict::InOrder, false));
        let guest = MicConfig {
            id: "guest".into(),
            name: "Hosť".into(),
            stream_name: "GUEST_EN".into(),
            sender_ip: None,
            channel: MicChannel::Left,
            language: MicLanguage::En,
            role: MicRole::Guest,
        };
        let mic = &service.with_health(|health| health.snapshot(&[guest], now)).mics[0];
        assert_eq!(mic.frames_out, 2, "200 ms in, 2 frames");
        let level = mic.level_dbfs.unwrap();
        assert!((level + 9.03).abs() <= 0.3, "{level}");
        assert!(mic.voiced);
    }
}
```

Append to `crates/presenter-server/src/listen/tests.rs` (add `use presenter_core::audio::sequence::SequenceVerdict;`, `use presenter_core::audio::vban::VbanReject;` and `use std::net::IpAddr;` to its `use` lines):

```rust
// ---- the counter rule's and the chain's counters (C1a-3) -------------------

#[tokio::test]
async fn the_verdicts_land_in_their_counters_and_the_tally_feeds_the_gate() {
    let t0 = Instant::now();
    let mut health = ListenHealth::default();
    health.set_mics(&[], &["guest"]);
    let gap = SequenceVerdict::Gap { missing: 3, silence_frames: 768 };
    let tallies: Vec<(u64, u64)> = [
        (SequenceVerdict::InOrder, false),
        (gap, false),
        (gap, true),
        (SequenceVerdict::Late, false),
        (SequenceVerdict::Resync, false),
    ]
    .into_iter()
    .map(|(verdict, dropped)| {
        let tally = health.record_packet("guest", seen(t0), verdict, dropped);
        (tally.events, tally.dropped_blocks)
    })
    .collect();
    assert_eq!(tallies, vec![(0, 0), (1, 0), (2, 1), (1, 0), (1, 0)]);
    assert_eq!(health.record_other_sender("guest"), 1);
    assert_eq!(health.record_other_sender("nobody"), 0);
    let mic = &health.snapshot(&[mic("guest", "GUEST_EN", None)], t0).mics[0];
    assert_eq!(
        (mic.packets, mic.lost_packets, mic.late_packets, mic.resyncs, mic.dropped_blocks, mic.other_sender_packets),
        (5, 6, 1, 1, 1, 1)
    );
}

#[tokio::test]
async fn packets_per_second_is_measured_between_ticks() {
    let t0 = Instant::now();
    let mut health = ListenHealth::default();
    health.set_mics(&[], &["guest"]);
    health.on_tick(t0);
    for k in 0..375u64 {
        let at = t0 + Duration::from_millis(k * 2);
        health.record_packet("guest", seen(at), SequenceVerdict::InOrder, false);
    }
    health.on_tick(t0 + Duration::from_secs(2));
    let mics = vec![mic("guest", "GUEST_EN", None)];
    let mic = &health.snapshot(&mics, t0 + Duration::from_secs(2)).mics[0];
    assert_eq!(mic.packets_per_sec, 187.5, "375 packets in 2 s");
    let silent = &health.snapshot(&mics, t0 + Duration::from_secs(4)).mics[0];
    assert_eq!(silent.packets_per_sec, 0.0, "no audio: no rate shown");
}

#[tokio::test]
async fn rejected_senders_are_counted_by_kind_and_keep_the_16_most_recent() {
    let t0 = Instant::now();
    let mut health = ListenHealth::default();
    let ip = addr(OTHER).ip();
    assert_eq!(health.note_reject(ip, VbanReject::NotVban, t0), 1);
    assert_eq!(health.note_reject(ip, VbanReject::NonAudio(0x40), t0), 2);
    assert_eq!(health.note_reject(ip, VbanReject::Corrupt, t0), 3);
    let rejected = &health.snapshot(&[], t0).rejected[0];
    assert_eq!(rejected.sender, "198.51.100.7");
    assert_eq!(
        (rejected.not_vban, rejected.non_audio, rejected.unsupported_codec, rejected.unsupported_type, rejected.corrupt),
        (1, 1, 0, 0, 1)
    );
    for k in 0..20u8 {
        let other: IpAddr = format!("203.0.113.{k}").parse().unwrap();
        health.note_reject(other, VbanReject::NotVban, t0 + Duration::from_millis(u64::from(k) + 1));
    }
    assert_eq!(health.snapshot(&[], t0).rejected.len(), RECENT_CAP);
}

#[tokio::test]
async fn a_chain_of_an_older_update_cannot_write_and_a_kept_mics_chain_can() {
    let t0 = Instant::now();
    let mics = vec![mic("guest", "GUEST_EN", None)];
    let mut health = ListenHealth::default();
    let old = health.set_mics(&[], &["guest"]);
    let new = health.set_mics(&[], &["guest"]);
    assert_ne!(old, new);
    health.record_chain("guest", old, 5, Some(-20.0), None);
    health.record_chain("guest", new, 1, Some(-30.0), None);
    assert_eq!(health.snapshot(&mics, t0).mics[0].frames_out, 1, "the older chain is ignored");
    health.set_mics(&["guest"], &[]); // a rename: the mic is kept
    health.record_chain("guest", new, 1, None, None);
    assert_eq!(health.snapshot(&mics, t0).mics[0].frames_out, 2, "the kept chain keeps counting");
    health.record_packet("guest", seen(t0), SequenceVerdict::InOrder, false);
    let refused = "nepodporovaná vzorkovacia frekvencia 11025 Hz";
    health.record_chain("guest", new, 0, None, Some(refused));
    let mic = &health.snapshot(&mics, t0 + Duration::from_millis(100)).mics[0];
    assert_eq!((mic.state, mic.error.as_deref()), (MicState::Error, Some(refused)));
    let silent = &health.snapshot(&mics, t0 + Duration::from_secs(3)).mics[0];
    assert_eq!((silent.state, silent.level_dbfs, silent.voiced), (MicState::NoAudio, None, false));
}
```

The lane-2 calls `health.record_packet("guest", seen(…))` in `the_mic_state_follows_the_last_packet` and `a_saved_list_keeps_kept_counters_and_starts_fresh_ones` become `health.record_packet("guest", seen(…), SequenceVerdict::InOrder, false);` (the new signature; the tests' assertions stay).

- [ ] **Step 3: How it is verified.** CI `Test` job (`mic_chain::tests`, `listen::tests`, core `status::tests`).

- [ ] **Step 4: The counters** — in `crates/presenter-server/src/listen/health.rs`:
- imports gain `use presenter_core::audio::level::is_voiced;`, `use presenter_core::audio::sequence::SequenceVerdict;`, `IpAddr` (`use std::net::{IpAddr, SocketAddr};`), `VbanReject` and `RejectedSender`; the module doc names the rejected senders again;
- `MicCounters` gains, after `last_packet`:

```rust
    packets_per_sec: f32,
    packets_at_tick: u64,
    tick_at: Option<Instant>,
    /// The mic list update that started these counters: a closed chain of an
    /// older update can no longer write them; a kept mic keeps its value.
    generation: u64,
    lost_packets: u64,
    late_packets: u64,
    resyncs: u64,
    /// Gaps (events, not packets): the #484 gate of the "lost packets" WARN.
    gap_events: u64,
    dropped_blocks: u64,
    other_sender_packets: u64,
    frames_out: u64,
    level_dbfs: Option<f32>,
    error: Option<String>,
```

- `ListenHealth` gains `generation: u64,` (after `socket`) and `rejected: Recent<IpAddr, RejectCounts>,` (after `unmatched_names`); the `RECENT_CAP` doc names the rejected senders again;
- the reject counts, after `UnmatchedEntry`:

```rust
#[derive(Debug, Default)]
struct RejectCounts {
    not_vban: u64,
    non_audio: u64,
    unsupported_codec: u64,
    unsupported_type: u64,
    corrupt: u64,
}

impl RejectCounts {
    /// Count one reject; the sender's total.
    fn count(&mut self, reject: VbanReject) -> u64 {
        match reject {
            VbanReject::NotVban => self.not_vban += 1,
            VbanReject::NonAudio(_) => self.non_audio += 1,
            VbanReject::UnsupportedCodec(_) => self.unsupported_codec += 1,
            VbanReject::UnsupportedType(_) => self.unsupported_type += 1,
            VbanReject::Corrupt => self.corrupt += 1,
        }
        self.not_vban + self.non_audio + self.unsupported_codec + self.unsupported_type + self.corrupt
    }
}
```

- new type, after `PacketSeen`:

```rust
/// The counts after one packet — the #484 log gate reads them, so `MicInput`
/// keeps no second set of event counters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct PacketTally {
    /// The count of this packet's verdict kind (gaps, late packets, resyncs); 0 in order.
    pub(crate) events: u64,
    /// `droppedBlocks` when this packet's block pushed out the oldest, else 0.
    pub(crate) dropped_blocks: u64,
}
```

- `set_mics`, `record_packet` are replaced; `note_reject`, `on_tick`, `record_other_sender` and `record_chain` are added:

```rust
    /// A refused datagram of `from`; that sender's reject total.
    pub(crate) fn note_reject(&mut self, from: IpAddr, reject: VbanReject, now: Instant) -> u64 {
        self.rejected.touch(from, now).count(reject)
    }

    /// Packets per second over the time since the last tick.
    pub(crate) fn on_tick(&mut self, now: Instant) {
        for mic in self.mics.values_mut() {
            if let Some(at) = mic.tick_at {
                let seconds = now.saturating_duration_since(at).as_secs_f32();
                if seconds > 0.0 {
                    let packets = mic.packets.saturating_sub(mic.packets_at_tick) as f32;
                    mic.packets_per_sec = (packets / seconds * 10.0).round() / 10.0;
                }
            }
            mic.packets_at_tick = mic.packets;
            mic.tick_at = Some(now);
        }
    }

    /// A saved mic list: `kept` mics keep their counters and generation (same
    /// stream, sender and channel — a rename or a language change resets
    /// nothing), `fresh` ones start from zero with this update's generation,
    /// mics no longer listed go. Returns the generation.
    pub(crate) fn set_mics(&mut self, kept: &[&str], fresh: &[&str]) -> u64 {
        self.generation += 1;
        self.mics.retain(|id, _| kept.contains(&id.as_str()));
        for id in fresh {
            let counters = MicCounters { generation: self.generation, ..MicCounters::default() };
            self.mics.insert((*id).to_string(), counters);
        }
        self.generation
    }

    pub(crate) fn record_packet(
        &mut self,
        mic_id: &str,
        seen: PacketSeen,
        verdict: SequenceVerdict,
        dropped_block: bool,
    ) -> PacketTally {
        let Some(mic) = self.mics.get_mut(mic_id) else {
            return PacketTally::default();
        };
        mic.packets += 1;
        mic.last_packet = Some(seen.at);
        mic.sender = Some(seen.from);
        mic.format = Some(seen.format);
        mic.frames_per_packet = Some(seen.frames);
        let events = match verdict {
            SequenceVerdict::Gap { missing, .. } => {
                mic.lost_packets += u64::from(missing);
                mic.gap_events += 1;
                mic.gap_events
            }
            SequenceVerdict::Late => {
                mic.late_packets += 1;
                mic.late_packets
            }
            SequenceVerdict::Resync => {
                mic.resyncs += 1;
                mic.resyncs
            }
            SequenceVerdict::InOrder => 0,
        };
        if dropped_block {
            mic.dropped_blocks += 1;
        }
        PacketTally { events, dropped_blocks: if dropped_block { mic.dropped_blocks } else { 0 } }
    }

    /// A packet of a second sender of the mic's stream; that count.
    pub(crate) fn record_other_sender(&mut self, mic_id: &str) -> u64 {
        self.mics.get_mut(mic_id).map_or(0, |mic| {
            mic.other_sender_packets += 1;
            mic.other_sender_packets
        })
    }

    /// A chain's output; ignored unless `generation` is the mic's current one.
    pub(crate) fn record_chain(
        &mut self,
        mic_id: &str,
        generation: u64,
        frames: u64,
        level_dbfs: Option<f32>,
        error: Option<&str>,
    ) {
        let Some(mic) = self.mics.get_mut(mic_id).filter(|mic| mic.generation == generation) else {
            return;
        };
        mic.frames_out += frames;
        if level_dbfs.is_some() {
            mic.level_dbfs = level_dbfs;
        }
        mic.error = error.map(str::to_string);
    }
```

- `mic_state` gains the arm `Some(_) if counters.error.is_some() => MicState::Error,` before `Some(_) => MicState::Receiving,` (its doc: "…; error while packets arrive in a format the converter refused");
- `snapshot` builds the rejected list after the unmatched one and returns it in `ListenStatus { …, rejected }`:

```rust
        let mut rejected: Vec<RejectedSender> = self
            .rejected
            .entries
            .iter()
            .map(|(ip, (counts, seen))| RejectedSender {
                sender: ip.to_string(),
                not_vban: counts.not_vban,
                non_audio: counts.non_audio,
                unsupported_codec: counts.unsupported_codec,
                unsupported_type: counts.unsupported_type,
                corrupt: counts.corrupt,
                last_packet_age_ms: age_ms(*seen, now),
            })
            .collect();
        rejected.sort_by(|a, b| a.sender.cmp(&b.sender));
```

- `mic_status` becomes:

```rust
fn mic_status(config: &MicConfig, counters: Option<&MicCounters>, now: Instant) -> MicStatus {
    let empty = MicCounters::default();
    let c = counters.unwrap_or(&empty);
    let state = mic_state(c, now);
    let receiving = state == MicState::Receiving;
    MicStatus {
        config: config.clone(),
        state,
        sender: c.sender.map(|addr| addr.to_string()),
        format: c.format,
        frames_per_packet: c.frames_per_packet,
        packets: c.packets,
        packets_per_sec: if receiving { c.packets_per_sec } else { 0.0 },
        last_packet_age_ms: c.last_packet.map(|at| age_ms(at, now)),
        lost_packets: c.lost_packets,
        late_packets: c.late_packets,
        resyncs: c.resyncs,
        dropped_blocks: c.dropped_blocks,
        other_sender_packets: c.other_sender_packets,
        frames_out: c.frames_out,
        level_dbfs: if receiving { c.level_dbfs } else { None },
        voiced: receiving && c.level_dbfs.is_some_and(is_voiced),
        error: c.error.clone(),
    }
}
```

- [ ] **Step 5: The chain** — `crates/presenter-server/src/listen/mic_chain.rs` (before the test block):

```rust
//! One microphone's chain (#852 C1a, spec §5.1 + §5.4). The receive loop hands
//! each decoded packet over as a [`Block`] through a bounded [`BlockQueue`]
//! and never waits: on a full queue the OLDEST block goes and is counted
//! (songplayer `vban-out.md` "The queue never blocks"). One task per mic
//! ([`run_chain`]) picks the channel, converts to 16 kHz mono
//! (`resample::Resampler16k`), cuts 100 ms frames and records how many and the
//! last one's level. The frames themselves reach no consumer yet: the channel
//! arrives with C1b's recorder / C3's ASR.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use presenter_core::audio::level::rms_dbfs;
use presenter_core::audio::vban::VbanAudioFormat;
use presenter_core::listen::mics::MicChannel;
use tokio::sync::Notify;
use tracing::{info, warn};

use super::resample::Resampler16k;
use super::{ListenService, MIC_TARGET};

/// ≈ 340 ms of 256-frame packets at 48 kHz — far above the ≥ 6 × 256
/// samples a Voicemeeter receiver keeps (songplayer `vban-out.md`).
pub(crate) const QUEUE_CAPACITY: usize = 64;

/// One packet's audio for the chain: `silence_frames` of inserted silence
/// (lost packets, spec §5.3) followed by the interleaved samples.
#[derive(Debug)]
pub(crate) struct Block {
    pub(crate) format: VbanAudioFormat,
    pub(crate) silence_frames: usize,
    pub(crate) samples: Vec<f32>,
}

/// A bounded single-consumer queue whose push never waits.
pub(crate) struct BlockQueue {
    state: Mutex<QueueState>,
    ready: Notify,
    capacity: usize,
}

#[derive(Default)]
struct QueueState {
    blocks: VecDeque<Block>,
    closed: bool,
}

impl BlockQueue {
    pub(crate) fn new(capacity: usize) -> Self {
        Self { state: Mutex::new(QueueState::default()), ready: Notify::new(), capacity: capacity.max(1) }
    }

    /// Queue a block; `true` when the oldest one had to go.
    pub(crate) fn push(&self, block: Block) -> bool {
        let dropped = {
            let mut state = self.lock();
            let dropped = state.blocks.len() >= self.capacity && state.blocks.pop_front().is_some();
            state.blocks.push_back(block);
            dropped
        };
        self.ready.notify_one();
        dropped
    }

    /// The next block; `None` once the queue is closed and empty.
    pub(crate) async fn pop(&self) -> Option<Block> {
        loop {
            {
                let mut state = self.lock();
                if let Some(block) = state.blocks.pop_front() {
                    return Some(block);
                }
                if state.closed {
                    return None;
                }
            }
            self.ready.notified().await;
        }
    }

    /// No more blocks: the chain drains what is queued and ends.
    pub(crate) fn close(&self) {
        self.lock().closed = true;
        self.ready.notify_one();
    }

    /// `(silence_frames, samples)` of every queued block.
    #[cfg(test)]
    pub(crate) fn peek(&self) -> Vec<(usize, usize)> {
        self.lock().blocks.iter().map(|b| (b.silence_frames, b.samples.len())).collect()
    }

    fn lock(&self) -> MutexGuard<'_, QueueState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The mic's channel of interleaved samples: `left` = channel 1, `right` =
/// channel 2 (channel 1 of a mono stream), `mix` = the mean of all channels.
pub(crate) fn pick_channel(interleaved: &[f32], channels: u16, pick: MicChannel) -> Vec<f32> {
    let width = usize::from(channels.max(1));
    interleaved
        .chunks_exact(width)
        .map(|frame| match pick {
            MicChannel::Left => frame[0],
            MicChannel::Right => frame.get(1).copied().unwrap_or(frame[0]),
            MicChannel::Mix => frame.iter().sum::<f32>() / width as f32,
        })
        .collect()
}

/// Channel pick + converter of one mic; a new stream format starts a fresh
/// converter (spec §5.3 "reset the converter").
pub(crate) struct MicChain {
    channel: MicChannel,
    format: Option<VbanAudioFormat>,
    resampler: Option<Resampler16k>,
    error: Option<String>,
}

impl MicChain {
    pub(crate) fn new(channel: MicChannel) -> Self {
        Self { channel, format: None, resampler: None, error: None }
    }

    /// One block → the 100 ms frames it completes (none while the format is refused).
    pub(crate) fn push(&mut self, block: &Block) -> Vec<Vec<i16>> {
        if self.format != Some(block.format) {
            self.reset(block.format);
        }
        let Some(resampler) = self.resampler.as_mut() else {
            return Vec::new();
        };
        let mut mono = vec![0.0; block.silence_frames];
        mono.extend(pick_channel(&block.samples, block.format.channels, self.channel));
        resampler.push(&mono)
    }

    pub(crate) fn format(&self) -> Option<VbanAudioFormat> {
        self.format
    }

    pub(crate) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    fn reset(&mut self, format: VbanAudioFormat) {
        self.format = Some(format);
        match Resampler16k::new(format.rate_hz) {
            Ok(resampler) => {
                self.resampler = Some(resampler);
                self.error = None;
            }
            Err(error) => {
                self.resampler = None;
                self.error = Some(error);
            }
        }
    }
}

/// The task of one mic: blocks in, frames counted with the last one's level
/// (only while `generation` is the mic's current one). Ends when the queue is
/// closed (the mic was dropped or changed, or the socket closed) and drained.
pub(crate) async fn run_chain(
    service: ListenService,
    mic_id: String,
    channel: MicChannel,
    generation: u64,
    queue: Arc<BlockQueue>,
) {
    let mut chain = MicChain::new(channel);
    while let Some(block) = queue.pop().await {
        let before = chain.format();
        let frames = chain.push(&block);
        if before != chain.format() {
            log_format(&mic_id, before, block.format, chain.error());
        }
        let level = frames.last().map(|samples| rms_dbfs(samples));
        service.with_health(|health| {
            health.record_chain(&mic_id, generation, frames.len() as u64, level, chain.error())
        });
    }
}

fn log_format(mic_id: &str, before: Option<VbanAudioFormat>, after: VbanAudioFormat, error: Option<&str>) {
    if let Some(error) = error {
        warn!(
            target: MIC_TARGET,
            mic = mic_id,
            rate = after.rate_hz,
            error,
            "mic converter refused the VBAN format — this mic gives no audio"
        );
    } else if let Some(before) = before {
        info!(target: MIC_TARGET, mic = mic_id, from = ?before, to = ?after, "mic VBAN format changed — converter reset");
    }
}
```

`crates/presenter-server/src/listen/mod.rs`: `mod mic_chain;` and `mod resample;` (Task 11) after `pub(crate) mod health;`; the module doc's last sentence becomes "Each mic has a chain (counter rule → 16 kHz → level, `mic_chain`); no frame channel yet: it arrives with its first consumer (C1b recorder / C3 ASR)."

- [ ] **Step 6: Gates + commit.** `cargo fmt --all -- --check`; `count_prod_lines.sh` on `mic_chain.rs` (≈ 150) and `health.rs` (≈ 300); `QC_TARGETS` both + `status.rs`; `no_panic_check.py`. The chain compiles; `run_chain` / `BlockQueue::push` get their production caller in Task 13 (one lane push, CI once). Commit paths `crates/presenter-core/src/listen/status.rs crates/presenter-server/src/listen/`, message `feat(listen): per-mic chain — drop-oldest queue, channel pick, 16 kHz frames, chain counters (#852 C1a-3)`.

### Task 13: The receiver hands over — sender lock, counter rule, queue

**Files:**
- Modify: `crates/presenter-server/src/listen/vban_rx.rs` (`MicInput`, `MicRouter::update`, `handle_datagram`, the counted rejects, `on_tick`, the module doc)
- Test: `crates/presenter-server/src/listen/tests.rs` (append — incl. the two reject tests that left lane 2 with the counters; `use super::mic_chain::QUEUE_CAPACITY;` added to its `use` lines), `crates/presenter-server/src/router/listen.rs` (the integration test's assertions extended)

**Interfaces:**
- Consumes: `StreamSequencer`, `SequenceVerdict` (Task 10), `run_chain`, `Block`, `BlockQueue`, `QUEUE_CAPACITY` (Task 12), `ListenHealth::{set_mics → u64, record_packet → PacketTally, record_other_sender}` (Task 12).
- Produces: `MicInput { config, sender_ip, source, last_packet, receiving, sequencer, queue }` with `Drop` (closes the queue → the chain drains and ends); `MicInput::start(&ListenService, &MicConfig, generation)` spawns the chain; `receives_like` now also compares the channel; `#[cfg(test)] MicRouter::queued(&self, mic_id) -> Vec<(usize, usize)>`.

- [ ] **Step 1: Write the failing tests.** Append to `crates/presenter-server/src/listen/tests.rs`:

```rust
// ---- the hand-over (C1a-3): via handle_datagram, explicit instants ---------

#[tokio::test]
async fn a_counter_gap_inserts_silence_and_counts_the_lost_packets() {
    let (service, mut router) = router_for(vec![mic("guest", "GUEST_EN", None)]);
    let t0 = Instant::now();
    let mut guest = guest_encoder();
    for _ in 0..2 {
        router.handle_datagram(&next_packet(&mut guest), addr(GUEST), t0);
    }
    guest.skip(2);
    router.handle_datagram(&next_packet(&mut guest), addr(GUEST), t0);
    let mic = status_of(&service, "guest", t0);
    assert_eq!((mic.packets, mic.lost_packets, mic.late_packets, mic.resyncs), (3, 2, 0, 0));
    assert_eq!(router.queued("guest"), vec![(0, 256), (0, 256), (512, 256)]);
}

#[tokio::test]
async fn a_late_or_duplicate_packet_is_dropped_and_counted() {
    let (service, mut router) = router_for(vec![mic("guest", "GUEST_EN", None)]);
    let t0 = Instant::now();
    let mut guest = guest_encoder();
    let packets: Vec<Vec<u8>> = (0..3).map(|_| next_packet(&mut guest)).collect();
    for packet in packets.iter().chain([&packets[1], &packets[2]]) {
        router.handle_datagram(packet, addr(GUEST), t0);
    }
    let mic = status_of(&service, "guest", t0);
    assert_eq!((mic.packets, mic.late_packets, mic.lost_packets), (5, 2, 0));
    assert_eq!(router.queued("guest").len(), 3, "late packets never reach the chain");
}

#[tokio::test]
async fn a_jump_of_more_than_a_second_resyncs_and_counting_goes_on() {
    let (service, mut router) = router_for(vec![mic("guest", "GUEST_EN", None)]);
    let t0 = Instant::now();
    let mut guest = guest_encoder();
    router.handle_datagram(&next_packet(&mut guest), addr(GUEST), t0);
    guest.skip(187); // exactly one second of 256-frame packets
    router.handle_datagram(&next_packet(&mut guest), addr(GUEST), t0);
    guest.skip(188); // one packet more than a second: a sender restart
    router.handle_datagram(&next_packet(&mut guest), addr(GUEST), t0);
    router.handle_datagram(&next_packet(&mut guest), addr(GUEST), t0);
    let mic = status_of(&service, "guest", t0);
    assert_eq!((mic.lost_packets, mic.resyncs, mic.packets), (187, 1, 4));
    let silences: Vec<usize> = router.queued("guest").iter().map(|(silence, _)| *silence).collect();
    assert_eq!(silences, vec![0, 187 * 256, 0, 0], "a resync inserts nothing");
}

#[tokio::test]
async fn the_counter_wraps_at_u32_max_without_a_gap() {
    let (service, mut router) = router_for(vec![mic("guest", "GUEST_EN", None)]);
    let t0 = Instant::now();
    let mut guest = guest_encoder().starting_at(u32::MAX - 1);
    for _ in 0..4 {
        router.handle_datagram(&next_packet(&mut guest), addr(GUEST), t0);
    }
    let mic = status_of(&service, "guest", t0);
    assert_eq!((mic.packets, mic.lost_packets, mic.late_packets, mic.resyncs), (4, 0, 0, 0));
}

#[tokio::test]
async fn a_full_queue_drops_the_oldest_block_and_counts_it() {
    // The chain task cannot run before this test awaits (current-thread
    // runtime), so the queue fills exactly as on a box whose chain fell
    // behind — and the receive path never waits.
    let (service, mut router) = router_for(vec![mic("guest", "GUEST_EN", None)]);
    let t0 = Instant::now();
    let mut guest = guest_encoder();
    for _ in 0..QUEUE_CAPACITY + 5 {
        router.handle_datagram(&next_packet(&mut guest), addr(GUEST), t0);
    }
    assert_eq!(status_of(&service, "guest", t0).dropped_blocks, 5);
    assert_eq!(router.queued("guest").len(), QUEUE_CAPACITY);
}

#[tokio::test]
async fn a_second_sender_of_the_stream_is_ignored_until_the_first_is_silent_for_3_s() {
    let (service, mut router) = router_for(vec![mic("guest", "GUEST_EN", None)]);
    let t0 = Instant::now();
    let (mut first, mut second) = (guest_encoder(), guest_encoder().starting_at(5_000));
    router.handle_datagram(&next_packet(&mut first), addr(GUEST), t0);
    let t1 = t0 + Duration::from_millis(1_000);
    router.handle_datagram(&next_packet(&mut second), addr(OTHER), t1);
    let mic = status_of(&service, "guest", t1);
    assert_eq!((mic.packets, mic.other_sender_packets), (1, 1));
    assert_eq!(mic.sender.as_deref(), Some(GUEST));
    let t3 = t0 + Duration::from_millis(3_000);
    router.handle_datagram(&next_packet(&mut second), addr(OTHER), t3);
    let mic = status_of(&service, "guest", t3);
    assert_eq!(
        (mic.packets, mic.other_sender_packets, mic.resyncs),
        (2, 1, 0),
        "the new sender starts its own count: no resync"
    );
    assert_eq!(mic.sender.as_deref(), Some(OTHER));
}

#[tokio::test]
async fn lost_packet_warnings_follow_the_power_of_two_gate() {
    let capture = Capture::install();
    let (service, mut router) = router_for(vec![mic("guest", "GUEST_EN", None)]);
    let t0 = Instant::now();
    let mut guest = guest_encoder();
    router.handle_datagram(&next_packet(&mut guest), addr(GUEST), t0);
    for _ in 0..5 {
        guest.skip(1);
        router.handle_datagram(&next_packet(&mut guest), addr(GUEST), t0);
    }
    assert_eq!(status_of(&service, "guest", t0).lost_packets, 5);
    let warnings = capture.events_with_message("mic lost VBAN packets");
    let events: Vec<&str> = warnings.iter().filter_map(|w| w.field("events")).collect();
    assert_eq!(events, vec!["1", "2", "4"]);
}

#[tokio::test]
async fn a_rename_keeps_the_chain_and_a_new_channel_restarts_it() {
    let (service, mut router) = router_for(vec![mic("guest", "GUEST_EN", None)]);
    let t0 = Instant::now();
    let mut guest = guest_encoder();
    for _ in 0..3 {
        router.handle_datagram(&next_packet(&mut guest), addr(GUEST), t0);
    }
    let mut renamed = mic("guest", "GUEST_EN", None);
    renamed.name = "Hosť z Londýna".into();
    router.update(&[renamed]);
    assert_eq!(router.queued("guest").len(), 3, "the same chain (and its queue) goes on");
    assert_eq!(status_of(&service, "guest", t0).packets, 3);
    let mut right = mic("guest", "GUEST_EN", None);
    right.channel = MicChannel::Right;
    router.update(&[right]);
    assert!(router.queued("guest").is_empty(), "another channel: a new chain");
    assert_eq!(status_of(&service, "guest", t0).packets, 0);
}

#[tokio::test]
async fn rejects_are_counted_per_sender_and_kind() {
    let (service, mut router) = router_for(vec![mic("guest", "GUEST_EN", None)]);
    let t0 = Instant::now();
    let good = next_packet(&mut guest_encoder());
    let with = |index: usize, value: u8| {
        let mut bytes = good.clone();
        bytes[index] = value;
        bytes
    };
    router.handle_datagram(b"hello", addr(GUEST), t0);
    router.handle_datagram(&with(4, 0x40 | 3), addr(GUEST), t0); // VBAN-TEXT
    router.handle_datagram(&with(7, 0x10 | 1), addr(GUEST), t0); // VBCA codec
    router.handle_datagram(&with(7, 0x05), addr(GUEST), t0); // FLOAT64
    router.handle_datagram(&good[..good.len() - 2], addr(GUEST), t0); // one sample short
    router.handle_datagram(b"x", addr(OTHER), t0);
    let status = service.with_health(|health| health.snapshot(&[], t0));
    assert_eq!(status.rejected.len(), 2);
    let guest = status.rejected.iter().find(|r| r.sender == "192.0.2.10").unwrap();
    assert_eq!(
        (guest.not_vban, guest.non_audio, guest.unsupported_codec, guest.unsupported_type, guest.corrupt),
        (1, 1, 1, 1, 1)
    );
    assert_eq!(status_of(&service, "guest", t0).packets, 0);
}

#[tokio::test]
async fn non_audio_vban_is_counted_but_logged_at_debug_only() {
    let capture = Capture::install();
    let (_service, mut router) = router_for(vec![mic("guest", "GUEST_EN", None)]);
    let t0 = Instant::now();
    let mut text = next_packet(&mut guest_encoder());
    text[4] = 0x40 | 3;
    for _ in 0..4 {
        router.handle_datagram(&text, addr(GUEST), t0);
    }
    let warnings: Vec<_> = capture
        .events(MIC_TARGET)
        .into_iter()
        .filter(|event| event.level == tracing::Level::WARN)
        .collect();
    assert!(warnings.is_empty(), "VB-Matrix's VBAN-TEXT is normal traffic: {warnings:?}");
    router.handle_datagram(b"garbage", addr(GUEST), t0);
    let warned = capture.events_with_message("dropped a datagram that is not usable VBAN audio");
    assert_eq!(warned.len(), 0, "the sender's 5th reject is not a power of two");
    router.handle_datagram(b"garbage", addr(OTHER), t0);
    let warned = capture.events_with_message("dropped a datagram that is not usable VBAN audio");
    assert_eq!(warned.len(), 1, "another sender's 1st reject warns");
}

#[tokio::test]
async fn real_udp_packets_come_out_as_16_khz_frames_at_the_tone_level() {
    let service = ListenService::new();
    service.apply(vec![mic("guest", "GUEST_EN", Some("127.0.0.1"))]);
    service.start(addr("127.0.0.1:0"));
    let bound = wait_for(&service, "bound", |s| s.socket.state == SocketState::Bound).await;
    let vban: SocketAddr = bound.socket.local_addr.unwrap().parse().unwrap();
    let tx = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let mut guest = guest_encoder();
    // 75 packets = 19 200 frames = 20 chunks of 20 ms = 6 400 samples = 4 frames
    let block = tone(48_000, 1_000.0, 0.5, 0, 75 * 256, 1);
    for (k, chunk) in block.chunks(256).enumerate() {
        tx.send_to(&guest.packet(chunk).unwrap(), vban).await.unwrap();
        if k % 4 == 3 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }
    let status = wait_for(&service, "4 frames counted", |s| s.mics[0].frames_out == 4).await;
    let mic = &status.mics[0];
    assert_eq!((mic.packets, mic.lost_packets, mic.state), (75, 0, MicState::Receiving));
    let level = mic.level_dbfs.unwrap();
    assert!((-9.6..=-8.5).contains(&level), "1 kHz at half scale ≈ −9.03 dBFS: {level}");
    assert!(mic.voiced);
}
```

In `crates/presenter-server/src/router/listen.rs`, `real_vban_packets_over_udp_show_in_get_listen_status` now waits for `|s| s["mics"][0]["framesOut"] == 5` (instead of `packets == 94`) and adds, after the `framesPerPacket` assertion:

```rust
        // C1a-3: the counter rule and the chain (counter 10 was left out).
        assert_eq!(mic["lostPackets"], json!(1));
        assert_eq!(
            (mic["latePackets"].clone(), mic["resyncs"].clone(), mic["droppedBlocks"].clone()),
            (json!(0), json!(0), json!(0))
        );
        let level = mic["levelDbfs"].as_f64().unwrap();
        assert!((-9.6..=-8.5).contains(&level), "1 kHz at half scale ≈ −9.03 dBFS: {level}");
        assert_eq!(mic["voiced"], json!(true));
        assert_eq!(status["rejected"][0]["notVban"], json!(1));
```

- [ ] **Step 2: How it is verified.** CI `Test` job.

- [ ] **Step 3: Implement** — in `crates/presenter-server/src/listen/vban_rx.rs`:
- imports gain `use presenter_core::audio::sequence::{SequenceVerdict, StreamSequencer};`, `use super::health::PacketTally;` (with `PacketSeen`) and `use super::mic_chain::{run_chain, Block, BlockQueue, QUEUE_CAPACITY};`; the module doc's "Lane C1a-3 adds…" sentence becomes "Per mic: a sender lock (a second sender of the same stream is ignored until the locked one is silent for 3 s), the counter rule and a bounded drop-oldest queue to the mic's chain (`mic_chain`).";
- `MicRouter::update`'s last statement becomes (a fresh mic starts a chain with this update's generation; a kept mic keeps its chain):

```rust
        let generation = self.service.with_health(|health| health.set_mics(&kept, &fresh));
        let service = self.service.clone();
        self.mics = configs
            .iter()
            .map(|config| match old.remove(&config.id) {
                Some(mut input) if input.receives_like(config) => {
                    input.config = config.clone();
                    input
                }
                _ => MicInput::start(&service, config, generation),
            })
            .collect();
```

  (replacing the lane-2 `with_health(|health| health.set_mics(&kept, &fresh))` line and the `MicInput::new` map);
- `handle_datagram` passes the packet by value: `Some(index) => self.mics[index].accept(&self.service, packet, from, now),`, and a refused datagram is counted: the `Err(reject)` arm calls `self.reject(from, reject, now);` (instead of the lane-2 DEBUG line), with, in `impl MicRouter`:

```rust
    fn reject(&self, from: SocketAddr, reject: VbanReject, now: Instant) {
        let total = self.service.with_health(|health| health.note_reject(from.ip(), reject, now));
        if matches!(reject, VbanReject::NonAudio(_)) {
            // VB-Matrix also speaks VBAN-TEXT / SERVICE: counted, never a WARN.
            debug!(target: MIC_TARGET, %from, %reject, total, "ignored a non-audio VBAN packet");
        } else if should_log_event(total) {
            warn!(target: MIC_TARGET, %from, %reject, total, "dropped a datagram that is not usable VBAN audio");
        }
    }
```

- `on_tick` starts with `self.service.with_health(|health| health.on_tick(now));` (packets per second), and `TICK`'s doc reads "Packets per second and the "lost audio" line are evaluated this often.";
- the `vban::` import gains `VbanReject`;
- a test accessor in `impl MicRouter`:

```rust
    /// `(silence_frames, samples)` of the blocks a mic's chain has not taken yet.
    #[cfg(test)]
    pub(super) fn queued(&self, mic_id: &str) -> Vec<(usize, usize)> {
        self.mics
            .iter()
            .find(|mic| mic.config.id == mic_id)
            .map(|mic| mic.queue.peek())
            .unwrap_or_default()
    }
```

- `MicInput` and its `impl` are replaced by:

```rust
struct MicInput {
    config: MicConfig,
    /// Parsed once per saved list, never per packet.
    sender_ip: Option<IpAddr>,
    /// The sender (`ip:port`) the mic is locked to.
    source: Option<SocketAddr>,
    last_packet: Option<Instant>,
    /// "receives" was logged and "lost audio" not yet.
    receiving: bool,
    sequencer: StreamSequencer,
    queue: Arc<BlockQueue>,
}

impl Drop for MicInput {
    /// The chain drains its queue and ends (the mic was dropped or changed,
    /// or the socket closed).
    fn drop(&mut self) {
        self.queue.close();
    }
}

impl MicInput {
    /// The mic's state and its chain task, counting with `generation`.
    fn start(service: &ListenService, config: &MicConfig, generation: u64) -> Self {
        let queue = Arc::new(BlockQueue::new(QUEUE_CAPACITY));
        tokio::spawn(run_chain(
            service.clone(),
            config.id.clone(),
            config.channel,
            generation,
            Arc::clone(&queue),
        ));
        Self {
            sender_ip: config.sender_ip_addr(),
            config: config.clone(),
            source: None,
            last_packet: None,
            receiving: false,
            sequencer: StreamSequencer::default(),
            queue,
        }
    }

    /// The same stream name, sender and channel: the mic keeps its state,
    /// counters and chain on a save (a rename or a language change).
    fn receives_like(&self, config: &MicConfig) -> bool {
        self.config.stream_name == config.stream_name
            && self.config.sender_ip == config.sender_ip
            && self.config.channel == config.channel
    }

    /// Exact stream name, and the configured sender if there is one.
    fn matches(&self, stream_name: &str, from: IpAddr) -> bool {
        self.config.stream_name == stream_name && self.sender_ip.is_none_or(|ip| ip == from)
    }

    fn accept(&mut self, service: &ListenService, packet: VbanPacket, from: SocketAddr, now: Instant) {
        let silent = self.last_packet.is_none_or(|at| now.saturating_duration_since(at) >= NO_AUDIO_AFTER);
        if !silent && self.source.is_some_and(|source| source != from) {
            self.other_sender(service, from);
            return;
        }
        if self.source != Some(from) {
            self.sequencer.reset();
            self.source = Some(from);
        }
        if silent {
            self.log_receiving(&packet, from);
        }
        self.last_packet = Some(now);
        let seen = PacketSeen { from, format: packet.format, frames: packet.frames, at: now };
        let counter = packet.counter;
        let verdict = self.sequencer.observe(counter, usize::from(packet.frames), packet.format.rate_hz);
        let silence_frames = match verdict {
            SequenceVerdict::Gap { silence_frames, .. } => silence_frames,
            _ => 0,
        };
        let dropped = verdict != SequenceVerdict::Late
            && self.queue.push(Block { format: packet.format, silence_frames, samples: packet.samples });
        let tally = service.with_health(|health| health.record_packet(&self.config.id, seen, verdict, dropped));
        self.log_verdict(verdict, counter, tally);
    }

    fn log_receiving(&mut self, packet: &VbanPacket, from: SocketAddr) {
        self.receiving = true;
        info!(
            target: MIC_TARGET,
            mic = self.config.id.as_str(),
            stream = self.config.stream_name.as_str(),
            %from,
            rate = packet.format.rate_hz,
            channels = packet.format.channels,
            sample_type = packet.format.sample_type.as_str(),
            frames = packet.frames,
            "mic receives VBAN"
        );
    }

    fn other_sender(&self, service: &ListenService, from: SocketAddr) {
        let packets = service.with_health(|health| health.record_other_sender(&self.config.id));
        if should_log_event(packets) {
            warn!(
                target: MIC_TARGET,
                mic = self.config.id.as_str(),
                locked = ?self.source,
                %from,
                packets,
                "mic ignores a second sender of its VBAN stream"
            );
        }
    }

    /// WARN on the 1st and every power-of-two event of a kind, read from the
    /// health counters (`PacketTally`), never from a second set of counters.
    fn log_verdict(&self, verdict: SequenceVerdict, counter: u32, tally: PacketTally) {
        let mic = self.config.id.as_str();
        if should_log_event(tally.events) {
            match verdict {
                SequenceVerdict::Gap { missing, .. } => {
                    warn!(target: MIC_TARGET, mic, missing, events = tally.events, "mic lost VBAN packets — silence inserted");
                }
                SequenceVerdict::Late => {
                    warn!(target: MIC_TARGET, mic, counter, events = tally.events, "mic dropped a late or duplicate VBAN packet");
                }
                SequenceVerdict::Resync => {
                    warn!(target: MIC_TARGET, mic, counter, events = tally.events, "mic VBAN counter jumped more than a second — resynced");
                }
                SequenceVerdict::InOrder => {}
            }
        }
        if should_log_event(tally.dropped_blocks) {
            warn!(target: MIC_TARGET, mic, dropped_blocks = tally.dropped_blocks, "mic chain queue full — dropped the oldest block");
        }
    }
}
```

(`MicInput::new` goes — every mic now starts with its chain. The lane-2 test `a_rename_keeps_the_mic_and_a_new_stream_name_starts_it_fresh` still holds: the stream name is part of `receives_like`.)

- [ ] **Step 4: Gates + commit.** `cargo fmt --all -- --check`; `count_prod_lines.sh crates/presenter-server/src/listen/vban_rx.rs` (≈ 300); `QC_TARGETS` it (`supervise` ≈ 50 lines, `accept` ≈ 30); `no_panic_check.py`. Commit paths `crates/presenter-server/src/listen/vban_rx.rs crates/presenter-server/src/listen/tests.rs crates/presenter-server/src/router/listen.rs`, message `feat(listen): sender lock, counter rule and the hand-over to each mic's chain (#852 C1a-3)`.

### Task 14: The playbook rule, then close lane C1a-3

**Files:**
- Create: `.claude/rules/listen-vban.md`
- Modify: `CLAUDE.md` (one Playbook Router row after the "Šepkár tips" row), `docs/configuration.md` (the listening section names the chain's fields)

- [ ] **Step 1: The rule** — `.claude/rules/listen-vban.md`:

```markdown
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
  `BlockQueue::push`, all synchronous; a full queue drops the OLDEST block (`droppedBlocks`). Never put
  an `.await` on a downstream consumer into `handle_datagram`.
- **Routing:** exact stream name + the sender IP parsed once per saved list (`MicInput::matches`); no
  match → `unmatched` (per (name, sender), 16 most recent) and an INFO on power-of-two packets per stream
  NAME (bounded map). A mic locks to its first sender (`ip:port`); another sender of the same name counts
  `otherSenderPackets` and is ignored until the locked one is silent ≥ 3 s, then takes over with a
  fresh counter. Non-audio VBAN (VB-Matrix's VBAN-TEXT / SERVICE) is counted, DEBUG only.
- **A save** (`MicRouter::update`): a mic whose stream name, sender and channel stay keeps its state,
  counters and chain (a rename or a language change resets nothing); another mic starts fresh with the
  update's generation; an unlisted mic's chain drains and ends. `record_chain` ignores a generation
  that is not the mic's current one.
- **Counter rule** (`StreamSequencer`): gap ≤ 1 s → silence of `missing × frames`; behind ≤ 1 s → drop
  (`latePackets`); more than 1 s either way → resync (used, `resyncs`). "1 s" = `rate / frames` of the
  CURRENT packet; everything wrapping (u32 counter).
- **Chain:** channel `left` = ch 1, `right` = ch 2 (ch 1 if mono), `mix` = mean; rubato `Fft` +
  `FixedSync::Both` with **20 ms** chunks (delay 160 samples = 10 ms; a whole-100-ms FFT would add 50 ms,
  over spec §8.5); 11 025 Hz is refused (`error`, mic state `error`); a format change starts a fresh
  converter. Frames = 1600 s16 samples, counted (`framesOut`) with the last one's level (`levelDbfs`);
  **no frame channel until its first consumer** (C1b recorder / C3 ASR) adds it.
- **Health / status:** one `std::sync::Mutex<ListenHealth>`, never across an `.await`; ages at read time
  (no-audio exactly 3 s after the last packet); the WARN gates read the health counters
  (`PacketTally`, `record_other_sender`, `note_reject`) — no second set of counters. `GET /listen/status`
  is a quiet GET in `request_log.rs`.
- **Config:** whole-list `GET|PUT /listen/mics`, validated in core `validate_mics` (≤ 4, id
  `[a-z0-9-]{1,20}` unique, name 1–40, stream 1–16 printable ASCII, IP canonical, no two mics one packet
  could match) → 422 Slovak text; a corrupt stored value → 500, receiver idle. Every save writes one
  best-effort `settings_audit` row (`app_settings` / `listen.mics`).
- **Logs** (`presenter::listen::mic`): INFO socket bound / closed, `listen mics saved`, `mic receives
  VBAN` (first packet or after ≥ 3 s silence), `mic lost audio`, format change, unassigned stream (gated);
  WARN through the #484 power-of-two gate (lost / late / resync / dropped / second sender per mic, rejects
  per sender). Dev's `gst-debug.conf` hides this target (observability rule) — read it on SNV.
- **Tests:** packets from the ported encoder; routing, late, resync, drop-oldest and sender-lock cases go
  through `MicRouter::handle_datagram` with RFC 5737 senders and explicit `Instant`s (on the
  current-thread runtime the chain cannot drain before the test awaits); one real loopback UDP test per
  path covers the socket; the golden packet is shared with `scripts/dev/listen/test_vban_send.py` and
  written as ≤ 16-char hex pieces (the staging secret scanner refuses a 32+ char hex blob).
- **Routing day (owner action, spec §20, after C1b):** the owner routes `GUEST_EN` / `INTERP_SK`
  (48 kHz, 16-bit, after the channel mute) to 10.77.9.205:6980; then the two mics are saved (the C1b
  card, or `PUT /listen/mics` with the spec §5.6 list) and the box acceptance runs (spec §5.8): status
  before / after, a 10-min `tcpdump`, `scripts/dev/listen/vban_capture_check.py` (added at that step).
- **FOH routing** (VB-Matrix on fohabl, songplayer `vban-out.md`): only with the owner's go; `Mute` only
  (VB-Matrix ignores `dBGain=-inf`); record the prior values; read back every point after each write.
```

CLAUDE.md Playbook Router row (after the "Šepkár tips" row):

```markdown
| Listening ingest (#852 C1a): VBAN codec ported from songplayer `vban_packet.rs` @ dd551b7d + typed-reject decode, mic list ships EMPTY (no socket until saved, validate mode never binds, startup read under the save lock), the receive loop never waits (drop-oldest chain queue), sender IP parsed once, counter rule gap ≤ 1 s = silence / late = drop / > 1 s = resync, a second sender locked out until the first is silent 3 s, a rename keeps the mic and its chain, rubato `Fft` 20 ms chunks → 16 kHz frames counted with their level (no frame channel until C1b / C3), whole-list `GET|PUT /listen/mics`, quiet `GET /listen/status` | `.claude/rules/listen-vban.md` (auto-loads on core `audio/**` + `listen/**`, server `listen/**`, `state/listen.rs`, `router/listen.rs`, `scripts/dev/listen/**`) |
```

In `docs/configuration.md`'s listening section, the `GET /listen/status` sentence becomes: "`GET /listen/status` shows the socket, every microphone's packets, packets per second, lost / late packets, resyncs, dropped blocks, 16 kHz frames, level, format, sender and state (`waiting`, `receiving`, `no_audio` 3 s after the last packet, `error` for a refused sample rate), the streams seen but not assigned and the senders whose datagrams were refused."

Commit — paths `.claude/rules/listen-vban.md CLAUDE.md docs/configuration.md`, message `docs(playbook): listening ingest rules (#852 C1a-3) [no-test: docs only]`.

- [ ] **Step 2: Measure the lane** (raw production lines, the review's budget ≤ ~700):

```bash
cd "$REPO"
for f in crates/presenter-core/src/audio/sequence.rs crates/presenter-server/src/listen/resample.rs crates/presenter-server/src/listen/mic_chain.rs; do printf '%s ' "$f"; bash scripts/dev/count_prod_lines.sh "$f"; done
git diff --stat origin/dev...HEAD -- crates/presenter-server/src/listen/health.rs crates/presenter-server/src/listen/vban_rx.rs crates/presenter-core/src/listen/status.rs
```

Expected (the plan's code) ≈ 650 non-blank / ≈ 695 raw: sequence ≈ 55, resample ≈ 95, mic_chain ≈ 150 new; health ≈ 200, vban_rx ≈ 115, status ≈ 35 added or rewritten (the rewritten functions counted whole); plus the rule (≈ 70 lines of markdown).

- [ ] **Step 3: Local gates** — as Task 9 Step 2 (fmt, placeholder, fn-length and file-size on every touched `.rs`, no-panic + its self-test, the Python tests, `git status`).

- [ ] **Step 4: Resolve rubato in CI's eyes and push.** `Cargo.lock` carries the Task 11 additions; load `ci-push-discipline`; `git fetch origin && git merge origin/dev`; push; monitor `pipeline.yml` to a terminal state — every job, Cargo Deny (licenses of rubato's tree) included. If `the_delay_is_a_constant_160_samples` measures a peak outside 4159..=4161, investigate rubato's real delay (`output_delay()` vs the measured peak) before touching the test — never widen the window.

- [ ] **Step 5: Verify on dev** — Task 9 Step 4's commands, but send with `--skip 100`; the status reader now prints the chain fields. Expected (3 s = counters 0..561, counter 100 left out): `packets` 561, `lostPackets` 1, `latePackets` 0, `resyncs` 0, `droppedBlocks` 0, `framesOut` 29, `levelDbfs` ≈ −9.0, `voiced` true; then `no_audio`; the empty save leaves the socket `idle`.

- [ ] **Step 6: Release (the supervisor's call) and verify SNV** — version + DOM footer as in Task 9; the same temporary test mic, `vban_send.py --to 10.77.9.205:6980 --stream PRESENTER_TEST --seconds 5 --skip 100` → `packets` 936, `lostPackets` 1, `framesOut` 49, `levelDbfs` ≈ −9.0, `voiced` true; journal adds `mic lost VBAN packets — silence inserted` (`missing=1 events=1`); then the empty save again — SNV stays without a socket until the routing day. PP: `/healthz` version, `/listen/status` socket `idle`, `mics` `[]`.

- [ ] **Step 7: Close the unit.** `playbook-review` (`📔 Playbook:` line); comment on #852: C1a complete (three lanes), the readings, the routing-day step for the owner action and the box acceptance after C1b. Spec §15 asks for a `/mutation-sweep` after the decoder unit: it is user-fired, so the comment asks for it; survivors in `audio/vban.rs`, `audio/sequence.rs`, `listen/vban_rx.rs` or `listen/health.rs` are C1a's work (a killing test in a follow-up commit). `vban_encode.rs` is excluded (test-only).

---

## After C1b and the owner's routing: routing day and box acceptance (spec §5.8)

Not part of the three lanes. The owner action of spec §20 (§16 order: C1a → C1b → routing → acceptance → recordings):

1. **The owner routes** `GUEST_EN` / `INTERP_SK` to 10.77.9.205:6980 (48 kHz, 16-bit, taken after the channel mute; or Claude does the VB-Matrix routing with the owner's go, `Mute` only, prior values recorded, every point read back).
2. **The mics are saved** — through the C1b Settings card, or:

```bash
curl -fsS -X PUT "$SNV/listen/mics" -H 'content-type: application/json' -d '[
 {"id":"guest","name":"Hosť","streamName":"GUEST_EN","senderIp":null,"channel":"left","language":"en","role":"guest"},
 {"id":"interp","name":"Tlmočník","streamName":"INTERP_SK","senderIp":null,"channel":"left","language":"sk","role":"interpreter"}]'
curl -fsS "$SNV/listen/status" | python3 -m json.tool     # socket "bound" 0.0.0.0:6980, both mics "receiving"
```

3. **Box acceptance** (songplayer's method): `curl -fsS "$SNV/listen/status" > before.json`; on presenter.lan `sudo timeout 600 tcpdump -i any -n -w /tmp/vban-acceptance.pcap udp port 6980`; copy the capture to dev2 and run the checker below (both streams `pass`: 0 missing packets, inter-arrival p99 < 15 ms); `after.json` diff: `lostPackets` / `latePackets` / `resyncs` / `droppedBlocks` +0, `packets` ≈ +112 500 per mic (187.5/s × 600 s); someone speaks into each mic → `voiced`, a muted mic → silence (level ≤ −50 dBFS, still `receiving`).

The capture checker lands with that step (`scripts/dev/listen/vban_capture_check.py` + `test_vban_capture_check.py`, picked up by Task 8's CI pattern; commit `feat(listen): VBAN capture checker for the box acceptance (#852)`). It was written and run while planning (7 tests green):

`scripts/dev/listen/test_vban_capture_check.py`:

```python
"""Tests for the VBAN capture checker (#852 C1a, spec §5.8). Run:
python3 -m unittest discover -s scripts/dev/listen -p 'test_vban_capture_check.py' -v
"""

import struct
import unittest

import vban_capture_check as check
import vban_send

SENDER = bytes([192, 0, 2, 10])
BOX = bytes([203, 0, 113, 5])


def udp_frame(payload, dport=6980, sport=6980, link=check.LINK_ETHERNET, fragment=0):
    udp = struct.pack(">HHHH", sport, dport, 8 + len(payload), 0) + payload
    ip = struct.pack(">BBHHHBBH4s4s", 0x45, 0, 20 + len(udp), 1, fragment, 64, 17, 0, SENDER, BOX) + udp
    if link == check.LINK_ETHERNET:
        return b"\x02" * 6 + b"\x04" * 6 + b"\x08\x00" + ip
    if link == check.LINK_SLL2:
        return b"\x08\x00" + b"\x00" * 18 + ip
    return b"\x00" * 14 + b"\x08\x00" + ip


def pcap(frames, link=check.LINK_ETHERNET):
    out = struct.pack("<IHHiIII", 0xA1B2C3D4, 2, 4, 0, 0, 65535, link)
    for ts, frame in frames:
        sec, usec = int(ts), int(round((ts - int(ts)) * 1e6))
        out += struct.pack("<IIII", sec, usec, len(frame), len(frame)) + frame
    return out


def vban(name, counter):
    return vban_send.build_packet(name, counter, 48000, 1, [0.0] * 256)


def run(data, max_p99_ms=15.0):
    return check.analyse(check.read_records(data), 6980, max_p99_ms)


class CaptureCheckTest(unittest.TestCase):
    def test_a_clean_stream_passes(self):
        frames = [(100.0 + k * 0.005333, udp_frame(vban("GUEST_EN", k))) for k in range(10)]
        [summary] = run(pcap(frames))
        self.assertEqual(summary["stream"], "GUEST_EN")
        self.assertEqual(summary["sender"], "192.0.2.10:6980")
        self.assertEqual((summary["packets"], summary["missing"], summary["late"]), (10, 0, 0))
        self.assertAlmostEqual(summary["interArrivalMs"]["p99"], 5.333, places=2)
        self.assertTrue(summary["pass"])

    def test_a_counter_gap_fails_and_is_counted(self):
        frames = [(100.0 + k * 0.005333, udp_frame(vban("GUEST_EN", k))) for k in range(10) if k != 3]
        [summary] = run(pcap(frames))
        self.assertEqual((summary["packets"], summary["missing"]), (9, 1))
        self.assertAlmostEqual(summary["interArrivalMs"]["max"], 10.666, places=2)
        self.assertFalse(summary["pass"])

    def test_late_and_duplicate_packets_are_counted_not_missing(self):
        order = [0, 1, 2, 1, 3]
        frames = [(100.0 + i * 0.005, udp_frame(vban("GUEST_EN", c))) for i, c in enumerate(order)]
        [summary] = run(pcap(frames))
        self.assertEqual((summary["missing"], summary["late"]), (0, 1))

    def test_a_slow_stream_fails_the_p99_bound(self):
        frames = [(100.0 + k * 0.020, udp_frame(vban("INTERP_SK", k))) for k in range(10)]
        [summary] = run(pcap(frames))
        self.assertEqual(summary["missing"], 0)
        self.assertFalse(summary["pass"])

    def test_streams_are_split_by_name_and_other_traffic_is_ignored(self):
        frames = [
            (1.0, udp_frame(vban("GUEST_EN", 0))),
            (1.001, udp_frame(vban("INTERP_SK", 0))),
            (1.002, udp_frame(b"not vban")),
            (1.003, udp_frame(vban("GUEST_EN", 1), dport=6981)),
            (1.004, udp_frame(b"VBAN" + bytes([0x40]) + bytes(23) + b"text")),
            (1.005, udp_frame(vban("GUEST_EN", 1), fragment=0x2000)),
        ]
        names = [s["stream"] for s in run(pcap(frames))]
        self.assertEqual(names, ["GUEST_EN", "INTERP_SK"])

    def test_linux_cooked_captures_are_read(self):
        for link in (check.LINK_SLL, check.LINK_SLL2):
            frames = [(5.0 + k * 0.005, udp_frame(vban("GUEST_EN", k), link=link)) for k in range(4)]
            [summary] = run(pcap(frames, link=link))
            self.assertEqual(summary["packets"], 4, link)

    def test_not_a_pcap_is_refused(self):
        with self.assertRaises(ValueError):
            list(check.read_records(b"\x0a\x0d\x0d\x0a" + bytes(40)))


if __name__ == "__main__":
    unittest.main()
```

`scripts/dev/listen/vban_capture_check.py`:

```python
#!/usr/bin/env python3
"""Check a tcpdump capture of VBAN audio (#852 C1a, spec §5.8 box acceptance).

    sudo timeout 600 tcpdump -i any -n -w /tmp/vban-acceptance.pcap udp port 6980
    python3 scripts/dev/listen/vban_capture_check.py /tmp/vban-acceptance.pcap

Per stream (name + sender): packets, missing packets (counter gaps), late or
duplicate packets, and the inter-arrival p50 / p99 / max in ms. Exit 0 when
every stream has no missing packet and an inter-arrival p99 under
`--max-p99-ms` (15, spec §5.8), else 1 — songplayer's box method
(`vban-out.md` "Box acceptance": 0 counter gaps, interval p99).

Reads classic pcap (µs or ns timestamps; tcpdump -w writes it) with link type
Ethernet (1), Linux cooked (113) or Linux cooked v2 (276), IPv4 UDP only (the
receiver binds IPv4). IP fragments are skipped (a VBAN packet is ≤ 1464 B).
"""

import argparse
import json
import math
import struct
import sys

LINK_ETHERNET = 1
LINK_SLL = 113
LINK_SLL2 = 276
ETHERTYPE_IPV4 = b"\x08\x00"
ETHERTYPE_VLAN = b"\x81\x00"
PCAP_MAGIC = {
    b"\xd4\xc3\xb2\xa1": ("<", 1e-6),
    b"\xa1\xb2\xc3\xd4": (">", 1e-6),
    b"\x4d\x3c\xb2\xa1": ("<", 1e-9),
    b"\xa1\xb2\x3c\x4d": (">", 1e-9),
}


def read_records(data):
    """Yield (timestamp in s, link type, frame) of a classic pcap file."""
    if len(data) < 24 or data[:4] not in PCAP_MAGIC:
        raise ValueError("not a classic pcap file (tcpdump -w writes one; pcapng is not read)")
    endian, unit = PCAP_MAGIC[data[:4]]
    link = struct.unpack(endian + "I", data[20:24])[0] & 0xFFFF
    offset = 24
    while offset + 16 <= len(data):
        sec, frac, length, _ = struct.unpack(endian + "IIII", data[offset:offset + 16])
        offset += 16
        yield sec + frac * unit, link, data[offset:offset + length]
        offset += length


def ipv4_packet(link, frame):
    """The IPv4 packet inside a link-layer frame, or None."""
    if link == LINK_ETHERNET:
        offset, ethertype = 14, frame[12:14]
        while ethertype == ETHERTYPE_VLAN and len(frame) >= offset + 4:
            ethertype, offset = frame[offset + 2:offset + 4], offset + 4
    elif link == LINK_SLL:
        offset, ethertype = 16, frame[14:16]
    elif link == LINK_SLL2:
        offset, ethertype = 20, frame[0:2]
    else:
        raise ValueError(f"unsupported pcap link type {link}")
    return frame[offset:] if ethertype == ETHERTYPE_IPV4 else None


def udp_datagram(ip, port):
    """(sender 'ip:port', payload) of an unfragmented IPv4 UDP datagram to `port`, or None."""
    if len(ip) < 20 or ip[0] >> 4 != 4 or ip[9] != 17:
        return None
    if struct.unpack(">H", ip[6:8])[0] & 0x3FFF:
        return None
    udp = ip[(ip[0] & 0x0F) * 4:]
    if len(udp) < 8:
        return None
    sport, dport, length = struct.unpack(">HHH", udp[:6])
    if dport != port:
        return None
    sender = ".".join(str(b) for b in ip[12:16])
    return f"{sender}:{sport}", udp[8:length]


def vban_audio_header(payload):
    """(stream name, counter) of a VBAN AUDIO packet, or None."""
    if len(payload) < 28 or payload[:4] != b"VBAN" or payload[4] & 0xE0:
        return None
    name = payload[8:24].split(b"\0", 1)[0].decode("ascii", "replace")
    return name, struct.unpack("<I", payload[24:28])[0]


def percentile(values, q):
    """Nearest-rank percentile; 0.0 for no values."""
    if not values:
        return 0.0
    ordered = sorted(values)
    return ordered[max(1, math.ceil(q / 100.0 * len(ordered))) - 1]


def analyse(records, port, max_p99_ms):
    """One summary dict per (stream, sender), sorted."""
    streams = {}
    for ts, link, frame in records:
        ip = ipv4_packet(link, frame)
        datagram = udp_datagram(ip, port) if ip else None
        header = vban_audio_header(datagram[1]) if datagram else None
        if header is None:
            continue
        state = streams.setdefault((header[0], datagram[0]), {
            "packets": 0, "missing": 0, "late": 0, "expected": None, "last": None, "gaps": [],
        })
        state["packets"] += 1
        counter = header[1]
        ahead = 0 if state["expected"] is None else (counter - state["expected"]) % 2**32
        if ahead < 2**31:
            state["missing"] += ahead
            state["expected"] = (counter + 1) % 2**32
        else:
            state["late"] += 1
        if state["last"] is not None:
            state["gaps"].append((ts - state["last"]) * 1000.0)
        state["last"] = ts
    summaries = []
    for (name, sender), state in sorted(streams.items()):
        p99 = percentile(state["gaps"], 99)
        summaries.append({
            "stream": name,
            "sender": sender,
            "packets": state["packets"],
            "missing": state["missing"],
            "late": state["late"],
            "interArrivalMs": {
                "p50": round(percentile(state["gaps"], 50), 3),
                "p99": round(p99, 3),
                "max": round(max(state["gaps"], default=0.0), 3),
            },
            "pass": state["missing"] == 0 and p99 < max_p99_ms,
        })
    return summaries


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("pcap")
    parser.add_argument("--port", type=int, default=6980)
    parser.add_argument("--max-p99-ms", type=float, default=15.0)
    args = parser.parse_args(argv)
    with open(args.pcap, "rb") as handle:
        summaries = analyse(read_records(handle.read()), args.port, args.max_p99_ms)
    if not summaries:
        print(f"no VBAN audio to port {args.port} in {args.pcap}")
        return 1
    for summary in summaries:
        print(json.dumps(summary, ensure_ascii=False))
    return 0 if all(s["pass"] for s in summaries) else 1


if __name__ == "__main__":
    sys.exit(main())
```

---

## Self-review against the spec

| Spec requirement | Task (lane) |
|---|---|
| §5.1 one socket `0.0.0.0:6980`, configurable, bound only with a mic; the receive loop never waits; bounded per-mic queue, oldest dropped + counted | 5, 6, 7 (2); 12, 13 (3) |
| §5.2 decode checks 1–8 in order, typed rejects counted per sender, nothing panics | 1 (1); 6 (2, dropped); 12, 13 (3, counted) |
| §5.3 stream name (exact, ≤ 16 ASCII) + optional sender IP; unmatched listed; counter rule (push / silence ≤ 1 s / late drop / > 1 s resync); format change resets the converter + INFO; no jitter buffer | 3 (1); 6 (2); 10, 12, 13 (3) |
| §5.4 channel left / right / mix; 16 kHz mono s16 via rubato `Fft` + `FixedSync`; 100 ms frames; RMS dBFS per 100 ms, voiced > −50; health (packets/s, lost, late, resyncs, last-packet age, level, format) | 2 (1); 5 (2: packets, age, format); 11, 12 (3: packets/s, lost, late, resyncs, level) |
| §5.5 recorder | **not in C1a** — C1b (§16); it brings the frame channel |
| §5.6 `listen.mics` JSON list, `language` en/sk/off, `role` guest/interpreter/preacher, empty list = idle, no socket | 3 (1); 6, 7 (2) — and it ships empty |
| §5.7 sender side (owner action) | "Routing day" + the rule (FOH: owner's go, Mute only, read back) |
| §5.8 box acceptance: tcpdump 10 min, 0 gaps, p99 < 15 ms, status before / after, level moves / silence when muted | "After C1b and the owner's routing" (capture checker there); the counters are diffable (5, 12) |
| §8.5 VBAN ≤ 30 ms (no buffering in the receive path); framing + resampler ≤ 120 ms | 6 (2); 11 (3, 20 ms chunks, 10 ms delay) |
| §11 "VBAN sender off" (no audio after 3 s, status + INFO) and "packet loss / sender restart" (silence ≤ 1 s, resync, rate-limited WARN) | 5, 6 (2); 13 (3) |
| §14 INFO `presenter::listen::mic` on state changes, WARNs through the #484 gate, `GET /listen/status` per-mic health | 6, 7 (2); 12, 13 (3) |
| §15 decoder tests (15 geometry rows, mono + stereo INT16, INT32, the rejects, counter wrap) | 1 (1) |
| §15 receiver tests (real loopback; name + IP; gap → silence; late drop; resync; drop-oldest) | 6 (2), 13 (3) — the split per "How the tests are split" |
| §15 resampler tests (1 kHz keeps its frequency, exact block length, constant delay) | 11 (3) |
| §15 "server integration: VBAN over loopback → decode → resample → mock ASR" | 13 (frames through the socket, status) + the HTTP test (7, extended in 13); the mock ASR consumer is C3 |
| §15 mutation: `/mutation-sweep` after the decoder unit merges | 14 Step 7 (user-fired); `vban_encode.rs` excluded (1) |
| C-S1 (2 h on prod, every gap counted + logged, no audio within 3 s) | 5, 6, 13; the 2 h run follows the routing |
| C-S8 (no mic = no subsystem; daily cap) | 6, 9 Step 6, 14 Step 6; the daily cap is C3 |
| §16 C1a content + rule `listen-vban.md` with the unit | 1–14 |
| §18 default 9 (no mic configured = nothing runs) | 6 — and the empty default everywhere |

Placeholder scan: no "TBD" / "TODO" / "similar to Task N"; every code step carries its code. Type consistency: `VbanAudioFormat` / `VbanSampleType` / `VbanReject` / `SequenceVerdict` / `MicConfig` / `MicState` / `SocketState` / `PacketSeen` / `PacketTally` / `Block` / `BlockQueue` / `ListenService::{apply, status, start, save_guard, with_health}` / `ListenHealth::{set_mics, record_packet, record_other_sender, record_chain, note_unmatched, note_reject, on_tick, snapshot}` / `MicRouter::{new, update, handle_datagram, on_tick, queued}` / `AppState::{listen_mics, set_listen_mics, listen_status, start_listen, spawn_listen_task}` carry the same names in every task; lane 3 changes `set_mics` (→ generation) and `record_packet` (→ verdict + tally) and updates every lane-2 caller in the same task. Every lane's head compiles and has no orphan: in lane 1 everything is `pub` core API; in lane 2 every `pub(crate)` item has a production caller (`ListenService` from `AppState`, `vban_rx::receive` from `supervise`) or is `#[cfg(test)]`; in lane 3 `run_chain` / `BlockQueue` / `Resampler16k` are called by `MicInput::start` / `MicChain` within the lane (intermediate commits inside a lane are not built on their own).

## Spec gaps found while planning (decided defaults — the owner may override any)

1. **Resampler chunk (§5.4 vs §8.5).** songplayer's converter FFTs one whole block; with 100 ms blocks that adds 50 ms (framing + resampler 150 ms, over §8.5's 120 ms). Plan: 20 ms chunks (`FixedSync::Both`, delay 160 samples = 10 ms, cut-off ≈ 7.66 kHz), assembled into the spec's 100 ms frames.
2. **11 025 Hz (§5.2 accepts every SR index).** The only rate whose 20 ms is not a whole number of frames: refused by the chain with the Slovak reason, mic state `error`. VB-Matrix / Voicemeeter send 48 kHz (§5.7).
3. **`left` / `right` / `mix` on mono and > 2 channels.** `left` = channel 1, `right` = channel 2 (channel 1 on a mono stream), `mix` = mean of all channels.
4. **Two senders of one stream name (§5.3 silent).** A mic locks to its first sender (`ip:port`) and switches only after that one is silent ≥ 3 s; the other sender's packets count as `otherSenderPackets`.
5. **"Ahead by packets totalling ≤ 1 s" (§5.3).** Counted in packets of the current packet's length (`rate / frames`); the inserted silence is `missing × frames`; more than 1 s away in either direction is a resync (used, no silence).
6. **SR index ≥ 21 (§5.2 step 5 names no class).** `corrupt`.
7. **Decode scale.** INT16 / INT24 by songplayer's symmetric encoder scale (32767 / 8388607), INT32 by `i32::MAX`, clamped to ±1 — lossless round trip with the ported encoder.
8. **Non-audio VBAN (VB-Matrix also speaks VBAN-TEXT, §5.2 step 2).** From lane 3: counted per sender, logged at DEBUG only; the other rejects WARN on power-of-two counts per sender (lane 2 only drops them with a DEBUG line). An unassigned stream logs INFO on power-of-two packets per stream NAME (bounded map, lane 2).
9. **The config API (§5.6 names only the setting and the C1b card).** Whole-list `GET|PUT /listen/mics` (the JSON array = the stored value), validated in core (≤ 4 mics, id `[a-z0-9-]{1,20}` unique, name 1–40 chars, stream 1–16 printable ASCII without edge spaces, sender IP optional and canonical, no two mics one packet could match); 422 Slovak text; unknown enum values are axum's JSON 422 (C1b's selects never send them); a corrupt stored value is a 500, never silently replaced; each save writes a `settings_audit` row like `ai-settings`. If C1b's `ListCard` needs per-item calls, C1b adds thin wrappers over the same state method.
10. **`/listen/status` shape (§14 lists content, not shape).** `{socket {state not_started|idle|bound|retrying, bindAddr, localAddr, error}, mics [config + state waiting|receiving|no_audio|error + counters], unmatched [...], rejected [...]}`, grown per lane (lane 3 adds the counter-rule and chain fields); the ASR state, language mismatch, cost meter and latency figures join in C3 / C7.
11. **The bind address (§5.1 "configurable").** Env `PRESENTER_LISTEN_VBAN_ADDR`, default `0.0.0.0:6980`; E2E servers `127.0.0.1:0`.
12. **A save while receiving (review 1c).** A mic whose stream name, sender and channel stay keeps its state, counters and chain (a rename or a language change resets nothing); a changed mic starts fresh (its counters from 0, a new chain); an unlisted mic's chain drains and ends.
13. **A bind / receive failure.** Status `retrying` with a Slovak error, retried after 1, 2, 4 … 60 s, WARN on power-of-two attempts.
14. **Bounded diagnostics.** `unmatched`, the per-name INFO gate and `rejected` keep the 16 most recently seen entries each.
15. **`/listen/status` is a quiet GET** in the request log (the C1b card polls it).
16. **Box tools.** The test sender lands with lane 2 (the post-deploy check needs a packet source); the capture checker lands with the acceptance step (review 3).
17. **The mic list ships EMPTY everywhere (review 4, reversing round 0).** No port is bound until mics are saved; dev copies prod's DB, so prod must not bind 6980 before the routing day. On the routing day the two mics are saved through `PUT /listen/mics` (or the C1b card) — "Routing day" above.
18. **No frame channel in C1a (review 2).** The chain counts its frames and keeps the last level; `framesOut` / `levelDbfs` prove it. The per-mic channel arrives with its first consumer (C1b recorder / C3 ASR), which also decides its capacity.
19. **Sizes (§16: ≈ 450 + 250 LoC; review: ≤ ~700 raw per lane).** Measured from this plan's code (before rustfmt): C1a-1 ≈ 455 non-blank / 495 raw + the 166-line test-only encoder; C1a-2 ≈ 680 non-blank / 755 raw Rust + the ≈ 100-line Python sender; C1a-3 ≈ 650 non-blank / 695 raw (+ the rule). To get lane 2 there, packets per second and the per-sender reject counters (`rejected`) moved to lane 3's counters; lane 2 drops a refused datagram with a DEBUG line. Lane 2 is still ≈ 55 raw lines over ~700 if blank lines count — the remaining cuts would be the settings-audit row or the unassigned-stream list, both of which the review keeps in lane 2, so it is left to the supervisor.
20. **rubato's msrv.** rubato 5.0.1 and audioadapter declare `rust-version = 1.87` while `clippy.toml` pins 1.85: harmless (resolver 2, no workspace `rust-version`, CI on stable; clippy's msrv lints only our code), noted so a reviewer does not "fix" it.

## Owner questions

None. The only prod-visible behaviour (decided default 17) is now the conservative one: nothing listens on SNV until the routing day.
