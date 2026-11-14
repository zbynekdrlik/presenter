# Listening C3 — the guest's live English transcript with Soniox stt-rt-v6 (#852) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship unit **C3** of #852 as amended on 2026-11-14 (spec §22): while the paid switch is on, a guest mic is
chosen, a key exists, the daily cap is not reached and either today's start was pressed or the stage layout is
**Kázeň** (`preach`, or left it < 60 s ago), Presenter runs ONE continuous **Soniox `stt-rt-v6`** real-time session
for the **guest's microphone only**, fed from the existing per-mic 16 kHz frame broadcast, turns its tokens into
sentences + a live tail, serves them on `GET /listen/ws` (snapshot + events), shows them in the **EN column of
`/ui/interpreter`** (tablet layout), reports state / minutes / cost / latency in `GET /listen/status`, and gives the
owner a "Prepis hosťa" section in Settings → Počúvanie (switch, guest mic, daily cap, start / stop / automatic). Built
and tested with NO key (a scripted Rust mock + a TypeScript fake Soniox for the E2E); a manual real-API acceptance on a
replayed English sermon closes it.

**Architecture:** A pure core (presenter-core: token → sentence assembly, the `/listen/ws` wire, the transcription
rows + start decision + cost, the Soniox context terms) under a server stack built bottom-up: `listen/soniox/wire.rs`
(config / control / response JSON, error classes, env settings) → `listen/soniox/machine.rs` (a sans-I/O state machine
of the logical session: lazy connect, silence gate with `finalize` + `keepalive`, park, reconnect with backoff, resend
since the last final, stall, rollover before 300 min, cost guards) → `listen/soniox/conn.rs` (one WebSocket connection
= one task, never awaited by the session; NO WS pings) → `listen/soniox/session.rs` (the provider task behind the
channel-based `StreamingAsr` trait). On top: the transcript hub + `GET /listen/ws`, the transcription supervisor (one
session task for the guest mic), the settings rows + endpoints + `preach` feed + meter, the key plumbing
(`/etc/presenter/listen.env`), the Settings section and the interpreter's EN column. Every new server module is
declared `#[allow(dead_code, reason = "#852 C3: wired by lane C3-7")]` until lane C3-7 wires the last caller (a grep
step there proves none is left).

**Tech Stack:** Rust 2021 (clippy msrv 1.85, toolchain floor 1.87), tokio 1.52 (`sync::{mpsc, broadcast, watch}`,
`time` incl. the paused test clock, `io::duplex` in tests), axum 0.8 (`ws`), **tokio-tungstenite 0.29** (moves from
dev-dependency to `[dependencies]` with a rustls TLS feature; client + the tests' scripted server), serde /
serde_json, chrono, thiserror, tracing; Leptos 0.7 CSR (presenter-ui); Playwright + the `ws` npm package (already a
devDependency) for the fake Soniox; Python 3 (`scripts/dev/listen/vban_send.py`, the eval's `timeline_score.py`) and
Node 24 (a `/listen/ws` tap) for the acceptance.

**Spec:** `docs/superpowers/specs/2026-10-14-sermon-listening-interpreter-assist-design.md` — **§22 (AMENDED
2026-11-14) first**, then §1 (C-S2 as amended in §22.7, C-S7, C-S8, C-S10), §4 (one shared transcription,
`/listen/ws`), §6.1 (provider trait), §6.4 (start / stop, paid gate, cap, meter — as adjusted in §22.5), §6.5 (as
adjusted in §22.6), §8.1 / §8.2 (the interpreter page, EN column only here), §11 (+ §22.12), §12 (keys), §14, §15,
§18 defaults 1, 4, 8, 9. Executors read §22 next to this plan.

**Builds on:** C1a + C1b + the Dante mics (#933) + S1a/S1b (the interpreter tablet shell), all on `dev`. Base of
this plan: `dev` @ 660dd4b6 (v0.4.378). Read `.claude/rules/listen-vban.md` (auto-loads on every `listen/**` file)
before any task; lane C3-8 adds `.claude/rules/listen-asr.md`.

**Supersedes:** `~/.claude/work-products/presenter/2026-10-15-listen-c3-live-translate.md` (the Gemini Live
Translate plan, never started). Its engine-neutral parts are reused here in idea — the channel-based trait
(its Decision 4), the sans-I/O machine (5), the hub + `/listen/ws` (9), one supervisor (10), the rows (11), the key
plumbing (14), the "testable with no key" approach (15), the dead-code allowances (17) — but NOT its code: every
Gemini-specific rule (GoAway overlap, resumption handles, `audioStreamEnd`, base64 frames, `usageMetadata`) is gone.

**How this plan was checked:** written against the tree at 660dd4b6 (file sizes below measured with
`scripts/dev/count_prod_lines.sh`), the Soniox protocol facts of spec §22.4 and the evaluation's working Python
client (`~/.claude/work-products/presenter/eval-audio/en-only/runs/soniox_stream.py` on dev2: the measured config,
the Bearer header, the ping failure). **No code in this plan was compiled or formatted** — it gives files,
interfaces and tests, not finished code; the lane sizes are ESTIMATES. Every lane is measured before its push
(Global Constraints) and split at the named split point if it passes 600.

---

## Decisions (with reasons)

1. **Engine + config = the measured one** (spec §22.3): `stt-rt-v6`, `pcm_s16le` / 16 000 / 1 channel,
   `language_hints: ["en"]`, `context {general: [domain, topic] (the measured wording), terms: [...]}`, no
   translation, no endpoint detection by default. Env knobs, each read once at startup: `PRESENTER_LISTEN_SONIOX_API_KEY`
   (unset = "Chýba kľúč"), `PRESENTER_LISTEN_SONIOX_MODEL` (default `stt-rt-v6`; `stt-rt-v5` is removed 2026-12-15),
   `PRESENTER_LISTEN_SONIOX_URL` (default `wss://stt-rt.soniox.com/transcribe-websocket`; tests/E2E point it at a
   mock, `ws://` allowed), `PRESENTER_LISTEN_SONIOX_ENDPOINT_DELAY_MS` (unset = endpoint detection off = the measured
   config; set = `enable_endpoint_detection: true` + `max_endpoint_delay_ms` clamped 500–3000 — the acceptance decides
   the default).

2. **Framework-first — what carries the WebSocket** (architecture-first #414). Candidates: (a) a Soniox Rust SDK —
   none official (Soniox ships Python / JS / … SDKs); `cargo search soniox` on 2026-11-14 lists only `sonisub` (an
   SRT generator over the ASYNC API, not a real-time client) → rejected; (b) `async-tungstenite` (already locked via
   gst-plugin-webrtc, futures-io first) → rejected; (c) **`tokio-tungstenite` 0.29** — already locked (axum `ws`, the
   #471 test), tokio-native, client + server handshake (the scripted test server needs both) → **chosen**, moved to
   `[dependencies]` with `rustls-tls-webpki-roots` (reqwest's root set). The Gemini plan verified at 72deb66a that this
   adds exactly ONE package to the lock (`webpki-roots 0.26.11`, a CDLA-Permissive-2.0 shim; licence on `deny.toml`'s
   allow list) and that rustls's only provider in the graph is `ring` — **re-verify at the lane's base** with
   `cargo metadata --offline` + the `Cargo.lock` diff (only `+` lines, no version moves; `listen-vban.md` rule).
   The protocol itself is ~250 lines of our own serde over documented JSON, pinned by fixture tests.
   **Owner's repos first (#1227):** `zbynekdrlik/voiceagent` runs Soniox real-time in production through LiveKit's
   Python plugin; that plugin's source (`livekit/agents` → `livekit-plugins-soniox/livekit/plugins/soniox/stt.py`, read
   2026-11-14) confirms this plan's choices — `Authorization: Bearer` header, `autoping=False` (no WS pings),
   `{"type": "keepalive"}` every 5 s while no audio is sent, `<end>` / `<fin>` treated as markers. voiceagent's
   `agent/soniox_context.py` also found that `context.text` (a plain instruction) decided how Soniox writes spoken
   numbers — the lever for "John 10 10" (Task 10.3).

3. **Guest mic only, chosen by a setting** (spec §22.1 / §22.5): the Dante mics are fixed with language `off`, so
   `listen.transcription.guestMic` names the mic (any id of the published list). The interpreter mic is never
   transcribed in C3. `MicConfig.language` is NOT consulted for the guest session (the session is always `en`); the
   Settings select lists every published mic by name.

4. **The trait is channel-based** (spec §6.1's `&mut self` pair cannot be awaited concurrently by one task — the
   Gemini plan's Decision 4, kept): `StreamingAsr::open(AsrSessionConfig) -> AsrSession { input:
   mpsc::Sender<AsrInput>, events: mpsc::Receiver<AsrEvent> }`, sync `open`, the session runs in its own task.

5. **A sans-I/O state machine for the session** (`SonioxCore`): `on_input` / `on_report` / `on_tick(now)` →
   `Vec<Action>`, `next_deadline()`; every rule is a deterministic unit test on hand-made `Instant`s; the driver only
   executes actions. Rules (spec §22.5):
   - **lazy connect** on the first voiced frame (> −50 dBFS, core `audio/level.rs` `VOICED_DBFS`), 1 s pre-roll ring
     (10 frames) sent first; frames that arrive while connecting are queued (≤ 5 s, older dropped + counted);
   - **silence gate:** 10 s without a voiced frame (quiet frames or no frames) → `finalize` once, stop audio,
     `keepalive` every 5 s; a voiced frame → pre-roll + live on the same connection; **park** after 5 min gated =
     empty text frame + close; the next voiced frame reconnects (same context, no backoff);
   - **reconnect** on a close without `finished`, a connect failure, error class Timeout / RateLimited / Server, or a
     **stall** (no response for 15 s while ≥ 10 s of audio went out since the last one; measured cadence p50 0.19 s,
     max 1.19 s): after 1, 5, 20, 60, 60 … s; first the frames after the end of the last FINAL token (≤ 5 s,
     from a 5 s replay ring tagged with session ms), then live;
   - **stop, no retry** on Auth (401 / 403), Payment (402), BadRequest (400) → `Finished(Some(reason))`; > 20
     connections in 10 min → `Finished(Some(TooManyConnections))`;
   - **rollover:** connection age ≥ 280 min → at the first gated moment, or at 295 min regardless: `finalize`,
     wait for `<fin>` (≤ 3 s, frames buffered meanwhile), empty text frame, new connection, buffered frames first.
     A `TooLong` error (the 300-min limit hit anyway) → immediate reconnect, no backoff;
   - **NEVER a WS Ping** from the client (Soniox does not answer them, spec §22.4); tungstenite still answers a
     server Ping automatically.

6. **Session time.** Soniox token times are per connection. Each connection carries `offset_ms` = the session ms of
   its first audio byte (the resend start after a reconnect); `TokenBatch { connection, offset_ms, tokens }` →
   session ms = `offset_ms + start_ms`. The machine tracks `last_final_end_ms` (session ms) for the resend rule.

7. **Assembly** (core, pure, spec §22.6): final tokens append to the open sentence, non-final tokens are the tail
   (replaced by every batch), translation tokens dropped, `<end>` / `<fin>` close the open sentence and are never
   text; close at `. ! ? …` (+ closing quote) when the NEXT final token starts with whitespace, at 20 words
   (songplayer's cap; a word starts at a token with leading whitespace), never on a pause. A connection that ends
   (close / reconnect / rollover) drops its non-final tail (the resend recreates it) but keeps its finals. `flush()`
   closes the open sentence when the session finishes.

8. **`GET /listen/ws`** (spec §4): on connect ONE snapshot (the newest 500 sentences, the tail, the session state of
   the guest mic), then `sentence` / `tail` / `session` messages; publishing happens under the hub's lock so
   subscribe + snapshot never miss or double a message; a reader 512 messages behind gets a fresh snapshot
   (`resyncs`, logged). `tail` messages are coalesced to ≤ 10 per second per mic (a response arrives every ~0.2 s;
   no coalescing needed in practice, the cap protects tablets). `session` messages only when state / reason / error
   change. No `LiveEvent` (stage / operator sockets never see transcript traffic) → `companion/variables.rs`
   untouched.

9. **One supervisor decides** (core `start_decision`: switch → guest mic chosen → guest mic published → key → cap →
   today's override → `preach` automation with a 60 s stop delay), re-evaluated on every rows / mic-list change and
   on a 1 s tick (which also reads `AppState.stage_layout` — no new channel into `state/mod.rs`, 974 lines). A mic
   the provider stopped for a person to fix (Auth, Payment, BadRequest, TooManyConnections) stays stopped until the
   switch, the guest mic or the override changes or the day ends — never a restart loop.

10. **Three `app_settings` rows** (the `listen.recorder` pattern): `listen.transcription` `{enabled, guestMic,
    dailyCapMinutes}` — absent = OFF, guestMic null, cap 240; `listen.session` `{day, mode: on|off, extraMinutes}` —
    today's choice (spec §6.4: the toggle overrides the automation for the rest of the day; a restart keeps it); a
    start after the cap adds 60 min to `extraMinutes`; `listen.usage` `{day, mics: {id: {audioMs, connectedMs}}}` —
    written every 60 s and at session end, so the cap survives an auto-deploy. Dev never inherits any of them
    (deleted in `pipeline.yml`'s redirect step, like `listen.recorder`).

11. **Metering:** the cap counts CONNECTED time (a conservative upper bound while Soniox's billing of a kept-alive
    stream is unverified; the 5-min park bounds it); audio sent is shown beside it; cost = connected h × $0.12.
    Latency = per voiced onset: first voiced frame sent → first token (any) received for it, last value + p50 / p90
    over the last 50 onsets (spec §14 "last latency figures"); the acceptance measures the real word latency.

12. **Context terms** (core `listen/context.rs`): priority order books (66, from a generated `BOOK_LONG_EN` table)
    → today's prepared references (#851 sermons of today, "Romans 8:37" with English long names) → today's active
    šepkár source terms → the curated Christian list; deduplicated case-insensitively; the whole JSON `context`
    capped at 8 000 characters by dropping from the end of the priority list [unverified: Soniox's own limit —
    the acceptance sends the largest real context once]. Built once per connection (a reconnect picks up a tip
    sent meanwhile).

13. **Key plumbing** (spec §12): `${ETC_DIR}/listen.env` (0600, root-only) written by `pipeline.yml` (dev),
    `deploy-site.yml` (SNV + PRBA) and `release.yml` (PP) from the GitHub secret `LISTEN_SONIOX_API_KEY` (+ optional
    variable `LISTEN_SONIOX_MODEL`), the `Configure AI backend` step's exact pattern (printf over ssh stdin, `umask
    077`, an EMPTY secret REMOVES the file); the three units get `EnvironmentFile=-<etc>/listen.env`. Every site gets
    the key; nothing runs without the switch + a guest mic (C-S8), and the rows are site-local.

14. **Testable with no key:** unit — the machine on hand-made `Instant`s; integration — the real `conn` + `session`
    against a scripted Soniox server over `tokio::io::duplex` (a `Connector` seam: production = TLS connect,
    tests = duplex; the paused clock stays exact); supervisor — a fake `StreamingAsr`; E2E — the real test-server
    binary against `tests/e2e/fake-soniox.ts` through `PRESENTER_LISTEN_SONIOX_URL`. The real API is a manual lane
    (C3-10), never a CI test.

15. **The interpreter's EN column** (spec §8.2, the transcript half only): sentences newest at the bottom (final text
    normal, the live tail lighter), older ones dimmed, the newest 300 in the DOM; auto-follow while the view is at the
    bottom, scrolling up pauses it and shows "▼ Naživo · N nové", a tap jumps back; a banner from the `session`
    message (C-S7: "Služba prepisu nedostupná — skúšam znova", "Prepis vypnutý — problém s účtom služby", "Denný
    limit prepisu dosiahnutý", "Nie je zvuk z mikrofónu hosťa"). The ⏸ button, A−/A+, the SK rows, the Bible area and
    the TV layout stay in C5a / C6 / C5b.

16. **Observability:** `presenter::listen::asr` (connect / close / error / reconnect / gate / park / rollover / stop,
    one summary per session: `audio_ms`, `connected_ms`, `connections`, `tokens_final`, `cost_usd`) and
    `presenter::listen::ws` (reader connect / disconnect / resync); every repeating WARN through the #484 power-of-two
    gate (`listen::should_log_event`); the key never in a log line (`wire::redact`, the request is built without
    `Debug` of headers). Dev's `gst-debug.conf` hides these targets (observability rule) — the acceptance reads
    `/listen/status` and a `/listen/ws` tap instead.

## Protocol facts — sources and status

| Fact | Used as | Source | Status |
|---|---|---|---|
| WS URL, `Authorization: Bearer` on the upgrade, no `api_key` in the config | `conn::request` | docs.soniox.com (2026-11-14); eval client | verified (eval) |
| Non-default `User-Agent` | `presenter-listen/<version>` | songplayer `dubbing-eval.md` (Cloudflare 1010) | verified (songplayer) |
| Config fields + the measured values | `wire::config_message` | docs; eval client | verified (eval) |
| Binary audio, empty TEXT frame = end, `finished: true` then close | `conn`, `machine` | docs; eval client | verified (eval) |
| `finalize` → finals + `<fin>`; `keepalive` | gate, rollover | docs; LiveKit plugin source | documented; **[unverified]** in our client → acceptance |
| Keepalive every 5 s while no audio | gate | LiveKit plugin (`_keepalive_task`, 5 s) | production practice (voiceagent); acceptance 2-min gated pause |
| No answer to WS pings | no client pings | eval run 2026-11-14 (1011 at 234 s); LiveKit plugin `autoping=False` | measured |
| `context.text` steers number formatting | candidate for "John 10 10" | voiceagent `agent/soniox_context.py` (Slovak sizes) | **[unverified]** for references → acceptance |
| Response cadence while audio flows (0.19 / 0.41 / max 1.19 s) | stall rule 15 s | eval `slice.sxctx.jsonl` | measured |
| Non-final replaced each response, finals once, sub-word tokens with leading spaces | assembler | docs; eval logs | verified (eval) |
| `<end>` only with endpoint detection | assembler | docs; eval logs (0 `<end>` without it) | verified (eval) |
| Error fields + codes 400 / 401 / 402 / 403 / 408 / 413 / 429 / 5xx | `wire::classify` | docs | documented; bad key → acceptance |
| ≤ 300 min per stream | rollover 280 / 295 min | docs | documented |
| Billing of a kept-alive stream without audio | cap counts connected time; park after 5 min | — | **[unverified]** |
| Context size limit | 8 000-char cap | — | **[unverified]** → acceptance |
| $0.12 / h | `cost_usd` | docs (pricing) | documented |

## Global Constraints

- **Ten lanes; nine are PR units** (spec §16: version bump first, ≤ 600 production lines, CI all green, auto-merge,
  deploy, post-deploy verification; C3-10 is the manual real-API acceptance). Sizes are ESTIMATES:

| Lane | Content | Est. production lines | Split point if > 600 |
|---|---|---|---|
| C3-1 | core: token assembly, `/listen/ws` wire | ≈ 380 | the wire (`ws.rs`) into its own PR |
| C3-2 | core: rows, start decision, cost, status types, context terms (+ generated `BOOK_LONG_EN`) | ≈ 430 | `context.rs` into C3-3 |
| C3-3 | server: Soniox wire + env settings; `tokio-tungstenite` to `[dependencies]` | ≈ 300 | — |
| C3-4 | server: the session machine (pure) | ≈ 560 | rollover + stall rules into `machine/timing.rs` in C3-5 |
| C3-5 | server: the connection task, the provider task, `StreamingAsr`, the scripted mock | ≈ 520 | the log helpers into `soniox/log.rs` in C3-6 |
| C3-6 | server: transcript hub + `GET /listen/ws` | ≈ 330 | — |
| C3-7 | server: supervisor + mic session task + rows endpoints + `preach` feed + meter + status; allowances removed | ≈ 580 | the meter + usage writer into its own PR |
| C3-8 | deploy: `listen.env` (3 workflows, 3 units), dev wipe, docs, rule, E2E harness defaults | 0 (YAML / docs / TS) | — |
| C3-9 | UI: Settings "Prepis hosťa" + interpreter EN column + the fake Soniox E2E | ≈ 480 UI | the EN column + its spec into C3-9b |
| C3-10 | real-API acceptance on dev (manual) | — | — |

- **Measure every lane** with the C1b plan's `lane_measure.sh` (`docs/superpowers/plans/2026-10-15-listen-c1b-settings-recorder.md`,
  "Measure honestly"; save it once to `$SCRATCH/lane_measure.sh`) before each push; > 600 → split at the named point.
- **Tier-0 checks before every push** (`.claude/rules/quality-gates.md`): `cargo fmt --all -- --check && (cd
  crates/presenter-ui && cargo fmt --check)`; `QC_TARGETS="<touched .rs>" python3 scripts/dev/fn_length_check.py .`;
  `bash scripts/dev/count_prod_lines.sh <each touched .rs>`; `python3 scripts/dev/no_panic_check.py .`;
  `bash scripts/dev/placeholder_check.sh . && bash scripts/dev/feature_router_check.sh .`; TS: `node
  node_modules/typescript/bin/tsc --noEmit -p tsconfig.json`; workflows: `python3 scripts/ci/check_deploy_workflows.py`
  + its unit test (delete `scripts/ci/__pycache__` after). Then a fresh-context review of the lane's diff, then push.
- **Version bump first** in every lane: `git fetch origin`; if `[workspace.package].version` is not greater than the
  latest GitHub release, bump the patch in its own commit `chore: bump version to X.Y.Z [no-test: version bump only]`.
- **Public repo:** no key anywhere (test literals `test-key-not-real` / `e2e-test-key-not-real`), loopback / RFC 5737
  addresses in tests, no e-mail or name in any header. The real key reaches GitHub only from the dev2 vault over
  stdin (lane C3-10), never argv or chat.
- **No new `LiveEvent`.** `companion/variables.rs` untouched.
- **Files near their caps** (production lines at 660dd4b6): `state/mod.rs` 974 (+2 at most: one `mod` line, one
  field — prefer keeping everything inside `ListenService`), `router.rs` 803 (+2: the `/listen/ws` route line, the
  module), presenter-ui `settings/listen.rs` 590 (+3: mount the new section; the section lives in its own
  `listen_transcription.rs`), `listen/mod.rs` 318, `state/listen.rs` 302, `router/listen.rs` 68,
  `pages/interpreter.rs` 100.
- **std APIs ≤ 1.85** (`clippy.toml` msrv), no lock across an `.await`, every spawned future `Send`.
- **Logs:** repeating WARNs through the #484 gate; client/service strings as quoted `&str` fields.
- **Deploy freeze:** no lane is pushed until the owner lifts the pre-Sunday freeze (2026-11-14 instruction); the plan
  is written now, executed after.

## Review Focus

The five real-world conditions most likely to bite the interpreter — each pinned by a test in the lane that owns it:

1. **The guest pauses, then goes on** → the gate sends ONE `finalize` after 10 s (the last words turn final at once,
   not stuck as grey tail), keepalives every 5 s, the next voiced frame resumes on the SAME connection with the 1 s
   pre-roll (the first word is not clipped); after 5 min the connection ends and the next speech reconnects without a
   banner. Tests: C3-4 `ten_quiet_seconds_finalize_once_then_keep_alive_every_five_seconds`,
   `voice_after_the_gate_resumes_on_the_same_connection_with_the_pre_roll`, `five_gated_minutes_park_and_the_next_voice_reconnects_without_backoff`;
   C3-1 `fin_closes_the_open_sentence_and_is_never_text`.
2. **The venue's internet drops for 17 s mid-sentence** (spec §11, 2026-10-12) → state `reconnecting` (the banner)
   for the outage, retries 1, 6, 26 s after the drop, then the audio since the last FINAL word (≤ 5 s) goes first:
   no word transcribed twice, none lost, the old tail replaced. Tests: C3-4
   `a_17_second_outage_retries_on_schedule_and_resends_from_the_last_final`; C3-1
   `a_connection_end_drops_its_tail_but_keeps_its_finals`; C3-5 `a_dropped_connection_resends_unconfirmed_audio_once`.
3. **A silent dead socket** (no close, no response) → no WS ping is ever sent; 15 s without a response while audio
   flows drops it and reconnects; while gated, silence is normal and never triggers it. Tests: C3-4
   `a_stall_while_audio_flows_reconnects`, `no_stall_while_gated`; C3-5 `the_client_never_sends_a_ping`.
4. **A cost runaway** → no voice = no connection; the cap counts connected time and stops the mic; 401/402/403/400
   never loop; > 20 connections in 10 min stop. Tests: C3-4 `nothing_connects_before_a_voiced_frame`,
   `auth_payment_and_bad_request_stop_without_retry`, `more_than_twenty_connections_in_ten_minutes_stop`; C3-7
   `the_daily_cap_stops_the_guest_session_and_a_start_adds_an_hour`, `a_provider_stop_blocks_the_mic_until_the_setting_changes`.
5. **No key / no guest mic chosen (PP, PRBA, SNV before setup)** → status says why ("Chýba kľúč služby prepisu
   (Soniox)", "Nie je vybraný mikrofón hosťa"), nothing connects, nothing WARNs in a loop; an empty secret removes a
   stale `listen.env`. Tests: C3-7 `nothing_starts_without_the_switch_the_guest_mic_the_key_and_the_layout`; C3-5
   `without_a_key_a_session_finishes_at_once_and_never_connects`; C3-9 the Settings key warning; C3-8 the workflow
   step's `else` branch.

## Spec gaps found — decided defaults

1. **§6.4 "start when `preach`"** stays; the console chip (§7.4) is C4 — until then Settings carries
   "Spustiť teraz / Zastaviť / Automaticky (Kázeň)" (`PUT /listen/session`), the chip later calls the same endpoint.
2. **`listen.enabled` (§6.4)** → `listen.transcription.enabled`, absent = OFF (the owner's money first).
3. **Persistence (§12 `listen_transcript`)** → NOT in C3 (scope: transcript first); the hub keeps the newest 500
   sentences in memory; a restart mid-sermon loses the history shown before it. Own lane "C3-persist" after C3
   (spec §22.14) — a `- [ ]` line on #852.
4. **Wrong-language detection (§11)** → C7; `enable_language_identification` stays off (the measured config).
5. **C-S8 "no microphone = nothing runs"** → the supervisor ticks once a second without a guest mic (it decides
   nothing then) — the same stated deviation as C1b's recorder supervisor; no connection, no cost.
6. **§14 "post-deploy live gate per listen provider"** → C7; C3 adds `docs/listen-providers.md` and the manual
   acceptance.
7. **Which mics the Settings select offers** → every published mic (VBAN + Dante) by name; a chosen mic that later
   disappears from the list → status `GuestMicMissing`, the setting is kept (it comes back with the mic).

## File map

| File | Lane | Role |
|---|---|---|
| core `listen/transcript.rs` (new) | C3-1 | `AsrToken`, `TokenBatch`, `TranscriptAssembler`, `AssemblyOutput`, `ClosedSentence`, `TailText`, `ends_sentence` |
| core `listen/ws.rs` (new) | C3-1 | `TranscriptSentence`, `TranscriptTail`, `TranscriptSnapshot`, `ListenWsMessage` |
| core `listen/transcription.rs` (new) | C3-1 (types), C3-2 | rows, `SessionMode`, `TranscriptionState`, `OffReason`, `MicTranscription`, `TranscriptionStatus`, `start_decision`, `cap_ms`, `cost_usd`, `preach_active`, `validate_transcription` |
| core `listen/context.rs` (new), `bible/resolver/names_table.rs` (generated), `scripts/dev/gen_bible_book_names.py` | C3-2 | context terms; `BOOK_LONG_EN` |
| core `listen/mod.rs`, `listen/status.rs` | C3-1, C3-2 | modules; `ListenStatus.transcription` (`#[serde(default)]`) |
| server `Cargo.toml`, `Cargo.lock` | C3-3 | `tokio-tungstenite` with rustls to `[dependencies]` |
| server `listen/soniox/{mod,wire,config}.rs` (new) | C3-3 | protocol JSON, error classes, env settings |
| server `listen/soniox/machine.rs` + `machine/tests.rs` (new) | C3-4 | `SonioxCore` |
| server `listen/asr.rs`, `listen/soniox/{conn,session}.rs`, `listen/soniox/tests/{mod,mock,session}.rs` (new) | C3-5 | trait, connection task, provider task, scripted mock |
| server `listen/transcript_hub.rs`, `router/listen_ws.rs` (new); `listen/mod.rs`, `router.rs` | C3-6 | hub, socket |
| server `listen/transcription/{mod,mic,meter,tests}.rs`, `state/listen_transcription.rs` (new); `state/listen.rs`, `router/listen.rs`, `listen/mod.rs`, `router/request_log.rs` | C3-7 | supervisor, session task, rows, endpoints, feed, meter, status |
| `.github/workflows/{pipeline,deploy-site,release}.yml`, `scripts/deploy/presenter{,-dev,-prba}.service`, `scripts/ci/check_deploy_workflows.py` (+ test), `docs/{configuration,listen-providers}.md`, `.claude/rules/listen-asr.md`, `CLAUDE.md`, `tests/e2e/support.ts` | C3-8 | key plumbing, docs, rule, harness defaults |
| presenter-ui `pages/settings/listen_transcription.rs` (new), `settings/listen.rs`, `api/listen.rs`, `components/interpreter/transcript.rs` (new), `ws/listen.rs` (new), `pages/interpreter.rs`, `styles/interpreter.css`; `tests/e2e/{fake-soniox.ts, listen-transcript.spec.ts, fixtures/listen/soniox-basic.jsonl}` (new) | C3-9 | the owner's section, the EN column, E2E |

---

## The lane close procedure (every PR lane ends with it)

1. `bash "$SCRATCH/lane_measure.sh"` → ≤ 600 (else split at the lane's named point, before pushing).
2. The Tier-0 checks of Global Constraints on every touched file.
3. A fresh-context review of `git diff origin/dev...HEAD` (requesting-code-review): the Tier-0 traps of
   `.claude/rules/quality-gates.md` (unused imports under `-D warnings`, E0063 struct literals, `if let` on widened
   enums, `doc_lazy_continuation`, `pub use` re-exports, `private_interfaces`), `Send` of every spawned future, no
   lock across an `.await`, no key in any `Debug` / log.
4. `git push origin dev`; monitor the pipeline to its end in ONE bounded foreground loop (ci-monitoring rule); a red
   job → `gh run view <id> --log-failed`, fix the cause in one commit.
5. Open the `dev → main` PR (`#852 C3-N: <lane title>`), gates green → merge (merge commit), monitor `deploy.yml` to
   the end, run the lane's post-deploy checks on SNV (`http://10.77.9.205`) — GET-only on prod: never switch the paid
   transcription on there.
6. `playbook-review` (the lane's rule lines: `listen-vban.md` before C3-8, `listen-asr.md` from C3-8 on), tick the
   lane's `- [ ]` line on #852, then the completion report.

---

## Lane C3-1 — core: token assembly, `/listen/ws` wire (≈ 380 lines)

### Task 1.1: Version check

- [ ] `git fetch origin`; bump per Global Constraints.

### Task 1.2: The assembler (`crates/presenter-core/src/listen/transcript.rs`, new)

**Interfaces:**

```rust
pub struct AsrToken {
    pub text: String,
    pub start_ms: Option<u64>, pub end_ms: Option<u64>,   // per connection
    pub confidence: f32, pub is_final: bool,
    pub language: Option<String>, pub is_translation: bool,
}
pub struct TokenBatch { pub connection: u32, pub offset_ms: u64, pub tokens: Vec<AsrToken>, pub received_at: DateTime<Utc> }
pub struct ClosedSentence { pub text: String, pub start_ms: u64, pub end_ms: u64, pub closed_at: DateTime<Utc>, pub language: Option<String> }
pub struct TailText { pub final_text: String, pub partial_text: String, pub start_ms: Option<u64> }
pub struct AssemblyOutput { pub closed: Vec<ClosedSentence>, pub tail: Option<TailText> /* Some = changed */ }
pub const END_MARKER: &str = "<end>"; pub const FIN_MARKER: &str = "<fin>";
pub const MAX_SENTENCE_WORDS: usize = 20;
pub fn ends_sentence(text: &str) -> bool;            // . ! ? … then optional ” " ’ ' )
pub struct TranscriptAssembler { /* open sentence finals, partial tail, word count, current connection */ }
impl TranscriptAssembler {
    pub fn new() -> Self;
    pub fn apply(&mut self, batch: TokenBatch) -> AssemblyOutput;
    pub fn connection_ended(&mut self, connection: u32) -> AssemblyOutput; // drops that connection's partial tail
    pub fn flush(&mut self, at: DateTime<Utc>) -> AssemblyOutput;          // session end: close the open sentence
}
```

Rules (Decision 7): sub-word tokens joined as written (their own leading spaces); the first token of a sentence is
left-trimmed; session ms = `offset_ms + token ms`; a pending punctuation close is decided by the next FINAL token
(starts with whitespace → close before it) or by `<end>` / `<fin>` / `flush`; a batch from a connection older than
the current one is ignored except its finals (a draining rollover connection).

**Tests** (in `listen/transcript/tests.rs` or `#[cfg(test)]` at the bottom, excluded from the count):

- [ ] `finals_append_and_partials_replace_the_tail` (three responses as Soniox sends them in the eval log: B / eca / use …).
- [ ] `a_period_closes_only_when_the_next_final_starts_a_word` ("5:13" and "Dr." mid-token never close; "it. Then" closes).
- [ ] `twenty_words_close_at_a_word_boundary_not_inside_a_word`.
- [ ] `end_marker_closes_the_open_sentence_and_is_never_text`, `fin_closes_the_open_sentence_and_is_never_text`.
- [ ] `translation_tokens_are_ignored`.
- [ ] `a_connection_end_drops_its_tail_but_keeps_its_finals`.
- [ ] `session_ms_adds_the_connection_offset` (a resend connection with `offset_ms` 61 200).
- [ ] `a_pause_never_closes_a_sentence` (a 4 s gap between two finals, no punctuation → one sentence).
- [ ] `flush_closes_an_open_sentence_and_empties_the_tail`.
- [ ] a fixture test over a 60-response excerpt of the eval's Soniox log for the Sloan slice
  (`crates/presenter-core/src/listen/testdata/soniox-sloan-excerpt.jsonl`, ≤ 30 KB, our own transcript text of a
  published sermon reduced to tokens — check with the owner's §13 rule that a short excerpt may be committed; if not,
  hand-write an equivalent 60-response fixture): every final word appears exactly once across the closed sentences,
  in order.

### Task 1.3: The wire (`crates/presenter-core/src/listen/ws.rs`, new)

```rust
#[serde(rename_all = "camelCase")] pub struct TranscriptSentence { pub id: u64, pub mic_id: String, pub text: String, pub start_ms: u64, pub end_ms: u64, pub closed_at: DateTime<Utc>, pub language: Option<String> }
#[serde(rename_all = "camelCase")] pub struct TranscriptTail { pub mic_id: String, pub final_text: String, pub partial_text: String, pub updated_at: DateTime<Utc> }
#[serde(rename_all = "camelCase")] pub struct TranscriptSnapshot { pub sentences: Vec<TranscriptSentence>, pub tails: Vec<TranscriptTail>, pub sessions: Vec<MicTranscription> }
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ListenWsMessage { Snapshot { snapshot: TranscriptSnapshot }, Sentence { sentence: TranscriptSentence }, Tail { tail: TranscriptTail }, Session { session: MicTranscription } }
```

**C3-1 also creates `listen/transcription.rs` with the types the wire needs — `MicTranscription`,
`TranscriptionState`, `OffReason` (fields as in Task 2.2, no logic) — and C3-2 adds the rows and the logic to the
same file.** Both crates (server + presenter-ui) use these types (presenter-ui already
depends on presenter-core).

- [ ] Tests: JSON round-trip of each message with literal expected JSON (`{"type":"tail","tail":{...}}`), so the UI and
  the E2E's assertions share one shape.

### Task 1.4: Close the lane

- [ ] Lane close procedure. Post-deploy: `curl -s http://10.77.9.205/healthz | jq .version` = the lane's version
  (core-only lane, nothing else observable).

---

## Lane C3-2 — core: rows, start decision, cost, status, context terms (≈ 430 lines)

### Task 2.1: Version check

### Task 2.2: Rows and the decision (`listen/transcription.rs`)

```rust
pub const TRANSCRIPTION_KEY: &str = "listen.transcription";
pub const SESSION_KEY: &str = "listen.session";
pub const USAGE_KEY: &str = "listen.usage";
pub const DEFAULT_DAILY_CAP_MINUTES: u32 = 240;
pub const PREACH_STOP_DELAY: Duration = 60 s; pub const START_EXTRA_MINUTES: u32 = 60;
pub const SONIOX_USD_PER_HOUR: f64 = 0.12;
#[serde(rename_all = "camelCase")] pub struct TranscriptionSetting { pub enabled: bool, pub guest_mic: Option<String>, pub daily_cap_minutes: u32 } // Default: false, None, 240
#[serde(rename_all = "lowercase")] pub enum SessionMode { On, Off, Auto }
pub struct SessionOverride { pub day: NaiveDate, pub mode: SessionMode, pub extra_minutes: u32 }
pub struct MicUsage { pub audio_ms: u64, pub connected_ms: u64 }
pub struct UsageRow { pub day: NaiveDate, pub mics: BTreeMap<String, MicUsage> }
pub enum TranscriptionState { Off, Armed, Connecting, Streaming, Gated, Parked, Reconnecting, Stopped }
pub enum OffReason { Disabled, NoGuestMic, GuestMicMissing, NoKey, CapReached, NotPreach, OverrideOff,
                     Auth, Payment, Refused, TooManyConnections }
impl OffReason { pub fn slovak(&self) -> &'static str } // "Prepis je vypnutý", "Nie je vybraný mikrofón hosťa", "Chýba kľúč služby prepisu (Soniox)", "Denný limit prepisu dosiahnutý", …
pub struct MicTranscription { pub mic_id: String, pub state: TranscriptionState, pub reason: Option<OffReason>, pub error: Option<String>,
    pub audio_ms_today: u64, pub connected_ms_today: u64, pub cap_ms: u64, pub cost_usd_today: f64,
    pub connections: u32, pub reconnects: u32, pub latency_last_ms: Option<u32>, pub latency_p50_ms: Option<u32>, pub latency_p90_ms: Option<u32> }
pub struct TranscriptionStatus { pub key_configured: bool, pub model: String, pub setting: TranscriptionSetting, pub mode: SessionMode, pub mics: Vec<MicTranscription> }
pub struct StartInputs<'a> { pub setting: &'a TranscriptionSetting, pub published: &'a [MicConfig], pub key: bool,
    pub today: NaiveDate, pub over: Option<&'a SessionOverride>, pub preach: bool, pub connected_ms_today: u64 }
pub fn start_decision(i: &StartInputs) -> Result<String /* guest mic id */, OffReason>; // order of Decision 9
pub fn cap_ms(setting: &TranscriptionSetting, over: Option<&SessionOverride>, today: NaiveDate) -> u64;
pub fn cost_usd(connected_ms: u64) -> f64;
pub fn preach_active(layout: &str, left_preach_at: Option<Instant>, now: Instant) -> bool;
pub fn next_override(current: Option<&SessionOverride>, today: NaiveDate, mode: SessionMode, cap_reached: bool) -> SessionOverride;
pub fn validate_transcription(s: &TranscriptionSetting, published: &[MicConfig]) -> Result<(), TranscriptionValidationError>; // cap 10..=600, guestMic ∈ published or None → Slovak text
```

`ListenStatus` gains `#[serde(default)] pub transcription: TranscriptionStatus` (`status.rs`); every struct literal
of `ListenStatus` in the server is updated (grep `ListenStatus {` — the E0063 trap).

- [ ] Tests: `absent_rows_mean_off` (serde defaults), `the_decision_checks_switch_mic_key_cap_override_then_preach_in_that_order`
  (one case per `OffReason`), `an_override_from_yesterday_is_ignored`, `on_override_starts_without_preach`,
  `preach_keeps_running_sixty_seconds_after_leaving`, `a_start_after_the_cap_adds_an_hour`,
  `cost_is_twelve_cents_per_connected_hour`, `validation_refuses_an_unknown_mic_and_a_cap_outside_ten_to_six_hundred`,
  `a_status_without_transcription_still_deserialises` (old status JSON).

### Task 2.3: Context terms (`listen/context.rs`) + the English long names

- [ ] Generator: `scripts/dev/gen_bible_book_names.py` also emits `pub static BOOK_LONG_EN: [&str; 66]` into
  `names_table.rs` from `data/bible-book-names/bprp-eng-preferred.yaml` (`long`, book order of `BOOK_CODES`); run it
  and its `--check` (CI runs `--check`; the table is generated → excluded from the line count).

```rust
pub const CONTEXT_DOMAIN: &str = "Christian church sermon";
pub const CONTEXT_TOPIC: &str = "preacher reads and cites Bible books, chapters and verses";
pub const CONTEXT_MAX_CHARS: usize = 8_000;
pub const CHRISTIAN_TERMS: &[&str] = &["Holy Spirit", "Pentecost", "righteousness", "sanctification", "atonement",
    "Gethsemane", "Nazareth", "Pharisees", "Sadducees", "apostle", "epistle", "covenant", "repentance", "resurrection",
    "Messiah", "Emmanuel", "hallelujah", "hosanna", "Yahweh", "Psalm", /* … ≤ 60, reviewed in the lane */ ];
pub struct ContextInputs { pub prepared_refs: Vec<String>, pub tip_terms: Vec<String> }
pub struct ContextTerms { pub general: Vec<(String, String)>, pub terms: Vec<String> }
pub fn context_terms(i: &ContextInputs) -> ContextTerms; // priority + dedup + cap (Decision 12)
pub fn reference_term(book_index: u8, chapter: u16, verse_start: Option<u16>, verse_end: Option<u16>) -> String; // "Romans 8:37-39"
```

- [ ] Tests: `all_66_books_are_terms` (incl. Mark, John, James, Job, Ruth — the eval list missed them),
  `prepared_references_follow_the_books`, `duplicates_are_dropped_case_insensitively`,
  `the_cap_drops_curated_terms_first_then_tip_terms`, `reference_term_formats_ranges_with_english_names`.

### Task 2.4: Close the lane (post-deploy: `/listen/status` still parses, `.transcription.setting.enabled == false`).

---

## Lane C3-3 — server: the Soniox wire and its settings (≈ 300 lines)

### Task 3.1: Version check

### Task 3.2: Dependency

- [ ] Move `tokio-tungstenite = { version = "0.29", features = ["rustls-tls-webpki-roots"] }` to
  `[dependencies]` of `crates/presenter-server/Cargo.toml` (keep no duplicate in `[dev-dependencies]`); resolve with
  `cargo metadata --offline` (non-compiling), diff `Cargo.lock`: only `+` lines, list every new package + licence
  against `deny.toml`. If anything beyond `webpki-roots` is new, record it on #852 before going on.

### Task 3.3: `listen/soniox/wire.rs` + `config.rs` (new; `listen/soniox/mod.rs` declares them)

```rust
pub const DEFAULT_URL: &str = "wss://stt-rt.soniox.com/transcribe-websocket";
pub const DEFAULT_MODEL: &str = "stt-rt-v6";
pub struct SonioxConfig { pub model: String, pub context: ContextTerms, pub endpoint_delay_ms: Option<u32> }
pub fn config_message(c: &SonioxConfig) -> String;         // never an api_key field
pub enum Control { Finalize, Keepalive }
pub fn control_message(c: Control) -> String;              // {"type":"finalize"} / {"type":"keepalive"}
pub struct RawToken { text, start_ms, end_ms, confidence, is_final, language, translation_status }  // serde, all but text optional
pub struct SonioxResponse { pub tokens: Vec<RawToken>, pub final_audio_proc_ms: Option<u64>, pub total_audio_proc_ms: Option<u64>, pub finished: bool, pub error: Option<SonioxError> }
pub struct SonioxError { pub code: Option<u16>, pub kind: Option<String>, pub message: String }
pub enum ErrorClass { Auth, Payment, BadRequest, Timeout, TooLong, RateLimited, Server, Unknown }
pub fn parse_response(text: &str) -> Result<SonioxResponse, WireError>;  // unknown fields ignored
pub fn classify(e: &SonioxError) -> ErrorClass;            // code first, then error_type text
pub fn classify_close(code: u16, reason: &str) -> ErrorClass;
pub fn to_tokens(r: &SonioxResponse) -> Vec<AsrToken>;     // translation_status "translation" → is_translation
pub fn redact(s: &str, key: &str) -> String;
// config.rs
pub struct SonioxEnv { pub url: String, pub key: Option<String>, pub model: String, pub endpoint_delay_ms: Option<u32> }
impl SonioxEnv { pub fn from_env() -> Self; pub fn from_vars(get: impl Fn(&str) -> Option<String>) -> Self; }
impl std::fmt::Debug for SonioxEnv // prints key: "<set>" / "<unset>", never the value
```

- [ ] Tests: `the_config_matches_the_measured_one` (literal JSON: model, `pcm_s16le`, 16000, 1, `["en"]`, context
  general + terms, NO `api_key`, NO `enable_endpoint_detection`), `endpoint_delay_adds_detection_clamped_500_to_3000`,
  `control_messages_are_the_documented_json`, `parses_partial_final_and_marker_tokens` (fixtures copied from the eval
  log shape), `parses_an_error_response` and `classify` table (400 / 401 / 402 / 403 / 408 / 413 / 429 / 500 / 503 /
  unknown), `env_defaults_and_overrides`, `debug_never_prints_the_key`, `redact_replaces_every_occurrence`.

### Task 3.4: Close the lane (`#[allow(dead_code, …)]` on `mod soniox;` in `listen/mod.rs`).

---

## Lane C3-4 — server: the session machine, pure (≈ 560 lines)

### Task 4.1: Version check

### Task 4.2: `listen/soniox/machine.rs` (new)

```rust
pub struct MachineConfig { pub key_present: bool, pub config_json: String /* rebuilt per connection by the driver */ }
pub enum Input { Frame { pcm: MicFrame, voiced: bool, at: Instant }, Stop { at: Instant } }
pub enum Report {
    Connected { conn: u32, at: Instant },
    Response { conn: u32, resp: SonioxResponse, at: Instant },
    Closed { conn: u32, code: Option<u16>, reason: String, at: Instant },
    ConnectFailed { conn: u32, error: String, at: Instant },
}
pub enum Action {
    Connect { conn: u32, offset_ms: u64 },
    SendAudio { conn: u32, pcm: MicFrame },
    SendControl { conn: u32, control: Control },
    EndAudio { conn: u32 },               // empty text frame
    Drop { conn: u32 },                   // abort the task
    Emit(TokenBatch),
    ConnectionEnded { conn: u32 },        // → assembler.connection_ended
    State { state: TranscriptionState, error: Option<String> },
    Usage { audio_ms: u64, connected_ms: u64 },
    Onset { latency_ms: u32 },
    Finished(Option<OffReason>),
}
pub struct SonioxCore { /* phase, conns (active + draining), rings (pre-roll 1 s, replay 5 s, connect queue 5 s),
                           session_ms, last_final_end_ms, gate timers, backoff step, connection log (10 min), counters */ }
impl SonioxCore {
    pub fn new(cfg: MachineConfig, now: Instant) -> Self;
    pub fn on_input(&mut self, i: Input) -> Vec<Action>;
    pub fn on_report(&mut self, r: Report) -> Vec<Action>;
    pub fn on_tick(&mut self, now: Instant) -> Vec<Action>;
    pub fn next_deadline(&self) -> Option<Instant>;
}
pub const GATE_AFTER: Duration = 10 s; KEEPALIVE_EVERY = 5 s; PARK_AFTER = 5 min; PRE_ROLL_FRAMES = 10;
pub const REPLAY_MAX = 5 s; STALL_AFTER = 15 s; STALL_MIN_AUDIO = 10 s; BACKOFF = [1, 5, 20, 60] s;
pub const ROLLOVER_FROM = 280 min; ROLLOVER_HARD = 295 min; FIN_WAIT = 3 s; MAX_CONNECTIONS_PER_10_MIN = 20;
```

Rules: Decision 5 in full; `Response` with tokens → `Emit` (+ update `last_final_end_ms`, the onset latency on the
first token after a voiced onset); `finished: true` → `ConnectionEnded` + (if not stopping / rolling over) reconnect
without backoff; `Stop` → `finalize` + `EndAudio`, wait ≤ 3 s for `finished`, then `Finished(None)`; no key →
the first `on_input` returns `Finished(Some(NoKey))` and never `Connect`.

**Tests** (`machine/tests.rs`, hand-made `Instant`s, no runtime):

- [ ] `nothing_connects_before_a_voiced_frame`; `the_first_voiced_frame_connects_and_sends_the_pre_roll_first`.
- [ ] `frames_while_connecting_are_queued_up_to_five_seconds_and_counted_when_dropped`.
- [ ] `ten_quiet_seconds_finalize_once_then_keep_alive_every_five_seconds`; `no_frames_for_ten_seconds_also_gates`.
- [ ] `voice_after_the_gate_resumes_on_the_same_connection_with_the_pre_roll`.
- [ ] `five_gated_minutes_park_and_the_next_voice_reconnects_without_backoff`.
- [ ] `a_17_second_outage_retries_on_schedule_and_resends_from_the_last_final` (drop at t, connects at t+1, +6, +26;
  the resend starts at `last_final_end_ms`, never earlier, ≤ 5 s).
- [ ] `a_stall_while_audio_flows_reconnects`; `no_stall_while_gated`.
- [ ] `auth_payment_and_bad_request_stop_without_retry`; `rate_limit_and_server_errors_back_off`;
  `too_long_reconnects_at_once`.
- [ ] `more_than_twenty_connections_in_ten_minutes_stop`.
- [ ] `rollover_waits_for_a_gated_moment_after_280_minutes`; `rollover_is_forced_at_295_minutes_without_losing_a_frame`
  (every frame sent exactly once across the two connections, in order); `rollover_waits_at_most_three_seconds_for_fin`.
- [ ] `stop_finalizes_ends_audio_and_finishes`; `without_a_key_the_first_frame_finishes_with_no_key`.
- [ ] `usage_counts_connected_and_audio_time`; `onset_latency_is_measured_from_the_first_voiced_frame`.

### Task 4.3: Close the lane.

---

## Lane C3-5 — server: the connection, the provider task, `StreamingAsr`, the scripted mock (≈ 520 lines)

### Task 5.1: Version check

### Task 5.2: The trait (`listen/asr.rs`, new)

```rust
pub struct AsrSessionConfig { pub mic_id: String, pub context: ContextTerms }
pub enum AsrInput { Frame { pcm: MicFrame, dbfs: f32 }, Stop }
pub enum AsrEvent { Tokens(TokenBatch), ConnectionEnded(u32), State { state: TranscriptionState, error: Option<String> },
                    Usage { audio_ms: u64, connected_ms: u64 }, Onset { latency_ms: u32 }, Finished(Option<OffReason>) }
pub struct AsrSession { pub input: mpsc::Sender<AsrInput>, pub events: mpsc::Receiver<AsrEvent> }
pub trait StreamingAsr: Send + Sync { fn open(&self, cfg: AsrSessionConfig) -> AsrSession; }
pub const INPUT_CAPACITY: usize = 64; // frames; the mic task uses try_send and counts drops — never waits
```

### Task 5.3: One connection (`listen/soniox/conn.rs`, new)

```rust
#[async_trait] pub trait Connector: Send + Sync { async fn connect(&self, req: Request) -> Result<WsStream, ConnectError>; }
pub struct TlsConnector; // tokio_tungstenite::connect_async_tls_with_config(req, None, false, None)
pub fn request(env: &SonioxEnv) -> Result<Request, ConnectError>; // Authorization: Bearer, User-Agent presenter-listen/<CARGO_PKG_VERSION>
pub enum ConnCommand { Config(String), Audio(MicFrame), Control(Control), End }
pub fn spawn_connection(conn: u32, connector: Arc<dyn Connector>, req: Request, reports: mpsc::Sender<Report>) -> (mpsc::Sender<ConnCommand>, AbortHandle);
```

The task: connect with a 10 s timeout → `Connected` / `ConnectFailed`; writes commands in order (audio as BINARY
s16 little-endian bytes); reads TEXT frames → `parse_response` → `Response`; BINARY from the server → DEBUG +
ignored; close / read error → `Closed`; **never sends `Message::Ping`** (no keepalive config on the client).

### Task 5.4: The provider task (`listen/soniox/session.rs`, new)

```rust
pub struct SonioxAsr { env: Arc<SonioxEnv>, connector: Arc<dyn Connector> }
impl StreamingAsr for SonioxAsr { fn open(&self, cfg: AsrSessionConfig) -> AsrSession } // spawns run()
async fn run(core: SonioxCore, inputs: mpsc::Receiver<AsrInput>, reports, events: mpsc::Sender<AsrEvent>, …);
// loop { select! { input, report, sleep_until(core.next_deadline()) } → execute actions }
```

`Connect` builds the config JSON from `cfg.context` + `env` (`wire::config_message`) and sends it as the first
command. Logging per Decision 16 (connect, close with code / reason redacted, reconnect with the attempt and the
wait, gate / park / rollover INFO, WARNs gated).

### Task 5.5: The scripted mock + integration tests (`listen/soniox/tests/{mod,mock,session}.rs`, test-only)

`mock.rs`: a Soniox server over `tokio::io::duplex` (`tokio_tungstenite::accept_hdr_async` to read the upgrade
headers) driven by a script: `Expect::Config(|json| …)`, `Expect::Audio { ms }`, `Reply(json)`, `ReplyOnFinalize`,
`Close(code)`, `Error(code, type)`, `GoSilent` (reads but never answers), and a log of every received frame (text /
binary length / ping). A `DuplexConnector` hands the client end to `SonioxAsr`.

- [ ] `the_upgrade_carries_bearer_and_user_agent_and_the_first_frame_is_the_config`.
- [ ] `audio_goes_out_as_binary_frames_in_order`.
- [ ] `tokens_come_back_as_batches_with_the_connection_offset`.
- [ ] `finalize_after_ten_quiet_seconds_returns_fin_and_keepalives_follow` (paused clock).
- [ ] `a_dropped_connection_resends_unconfirmed_audio_once` (mock closes 1006 after a final ending at 3 000 ms; the
  second connection receives exactly the audio from 3 000 ms on).
- [ ] `a_401_stops_without_a_second_connection`.
- [ ] `a_silent_server_is_dropped_after_fifteen_seconds_of_audio` (`GoSilent`).
- [ ] `the_client_never_sends_a_ping` (over every scenario: the mock's log holds 0 Ping frames).
- [ ] `stop_ends_audio_with_an_empty_text_frame_and_waits_for_finished`.
- [ ] `without_a_key_a_session_finishes_at_once_and_never_connects`.

### Task 5.6: Close the lane.

---

## Lane C3-6 — server: transcript hub + `GET /listen/ws` (≈ 330 lines)

### Task 6.1: Version check

### Task 6.2: The hub (`listen/transcript_hub.rs`, new; owned by `ListenService`)

```rust
pub const SENTENCES_KEPT: usize = 500; pub const HUB_CAPACITY: usize = 512; pub const TAIL_MIN_INTERVAL: Duration = 100 ms;
pub struct TranscriptHub { inner: Mutex<HubState>, tx: broadcast::Sender<ListenWsMessage> }
impl TranscriptHub {
    pub fn push_sentence(&self, mic_id: &str, s: ClosedSentence) -> u64; // id = next_id++
    pub fn set_tail(&self, mic_id: &str, t: Option<TailText>, at: DateTime<Utc>);  // coalesced
    pub fn set_session(&self, m: MicTranscription);                               // only on a visible change
    pub fn subscribe(&self) -> (TranscriptSnapshot, broadcast::Receiver<ListenWsMessage>); // under the same lock
    pub fn snapshot(&self) -> TranscriptSnapshot;
}
```

### Task 6.3: `GET /listen/ws` (`router/listen_ws.rs`, new; route line in `router.rs`)

Upgrade → send `Snapshot` → forward messages; `Lagged(n)` → a fresh `Snapshot` + `resyncs += 1` (INFO); a client
message is ignored (read only for close); INFO connect / disconnect with the client IP by the `request_log` rule.

- [ ] Tests (real axum server on loopback + `tokio_tungstenite` client, the #471 pattern):
  `the_first_message_is_a_snapshot_with_the_newest_sentences`, `sentences_and_tails_follow_in_order`,
  `subscribe_and_snapshot_never_miss_or_double_a_sentence` (a publisher racing the subscriber),
  `a_lagging_reader_gets_a_fresh_snapshot`, `tails_are_coalesced_to_ten_per_second`,
  `session_messages_only_on_a_visible_change`, `only_the_newest_500_sentences_are_kept`.

### Task 6.4: Close the lane (post-deploy on SNV: a `/listen/ws` tap receives `{"type":"snapshot", …}` with empty
lists — GET-only).

---

## Lane C3-7 — server: supervisor, session task, rows, endpoints, `preach` feed, meter, status (≈ 580 lines)

### Task 7.1: Version check

### Task 7.2: Rows + endpoints (`state/listen_transcription.rs`, new; `router/listen.rs`)

- `GET` comes with `/listen/status` (`transcription`); `PUT /listen/transcription` `{enabled, guestMic, dailyCapMinutes}`
  → `validate_transcription` → 422 Slovak / 200 status; `PUT /listen/session` `{mode: "on"|"off"|"auto"}` → 200.
  Both under the listen save lock, each with one best-effort `settings_audit` row (`listen.recorder` pattern), INFO
  `presenter::listen::asr` with the actor. Startup reads the three rows before the supervisor starts (never in
  validate mode). `request_log.rs`: nothing new is quiet (writes stay logged).

### Task 7.3: The supervisor + the session task (`listen/transcription/{mod,mic}.rs`, new)

```rust
pub struct TranscriptionService { /* rows (watch), asr: Arc<dyn StreamingAsr>, hub, running: Option<MicTask>, blocked: Option<(String, OffReason, RowsVersion)>, meter */ }
impl TranscriptionService {
    pub fn spawn(listen: ListenService, state_layout: Arc<RwLock<String>>, asr: Arc<dyn StreamingAsr>, env: Arc<SonioxEnv>, …) -> Self;
    pub fn status(&self) -> TranscriptionStatus;
}
// mic.rs: one task per running guest session
async fn run_mic(mic_id, frames: broadcast::Receiver<MicFrame>, session: AsrSession, hub, meter, …);
//   frames → pcm_dbfs (core audio/level.rs) → AsrInput::Frame via try_send (drops counted, Lagged counted)
//   events → TranscriptAssembler → hub.push_sentence / set_tail; State/Usage/Onset → status + meter; Finished → flush + exit
```

Decision 9: re-evaluate on rows / mic-list watch changes and every 1 s (reads `stage_layout`, tracks the moment it
left `preach`); start = build `ContextInputs` (today's sermons' references from `state/sermons` — locate the
"today's prepared passages" read used by the study / sermon prep pages; today's active tips from
`state/interpreter_tips` `active` list) → `asr.open` → spawn `run_mic`; stop = `AsrInput::Stop`, await `Finished`
≤ 5 s, then abort.

### Task 7.4: The meter (`listen/transcription/meter.rs`, new)

Per mic per local day: `audio_ms`, `connected_ms` from `Usage` events; written to `listen.usage` every 60 s and at
session end; on a new local date the row starts at zero; latency ring of 50 onsets → p50 / p90; one INFO summary
per session.

### Task 7.5: Wire it and remove the allowances

- [ ] `ListenService::start` spawns the supervisor (never in validate mode) with `SonioxAsr { env, TlsConnector }`.
- [ ] Delete every `#[allow(dead_code, reason = "#852 C3: wired by lane C3-7")]`; prove it:
  `grep -rn 'wired by lane C3-7' crates/` → no output.

**Tests** (`listen/transcription/tests.rs`, a fake `StreamingAsr` recording opens / inputs and scripting events;
paused clock; real `ListenService` with a fake frame sender):

- [ ] `nothing_starts_without_the_switch_the_guest_mic_the_key_and_the_layout` (one case per `OffReason`, 0 opens).
- [ ] `preach_starts_the_guest_session_and_leaving_it_stops_after_sixty_seconds`.
- [ ] `an_on_override_starts_without_preach_and_off_stops_for_the_rest_of_the_day`.
- [ ] `only_the_guest_mic_frames_reach_the_session` (two mics publishing; the interpreter mic's frames never arrive).
- [ ] `the_daily_cap_stops_the_guest_session_and_a_start_adds_an_hour`.
- [ ] `usage_survives_a_restart_the_same_day` (row written, a new service reads it, cap reached stays reached).
- [ ] `a_provider_stop_blocks_the_mic_until_the_setting_changes` (Auth → no reopen on the next ticks; a PUT reopens).
- [ ] `the_context_carries_todays_prepared_references_and_tip_terms`.
- [ ] `tokens_become_hub_sentences_and_a_tail` (end-to-end through the assembler).
- [ ] `a_slow_session_never_blocks_the_chain` (input channel full → drops counted, the frame broadcast unaffected).
- [ ] router: `put_transcription_validates_and_audits`, `put_session_sets_todays_mode`,
  `status_reports_key_state_without_the_key`.

### Task 7.6: Close the lane (post-deploy on SNV, GET-only: `/listen/status | jq .transcription` → `keyConfigured`
false until C3-8 + the secret, `setting.enabled` false, `mics` empty; the journal on SNV has no
`presenter::listen::asr` WARN).

---

## Lane C3-8 — deploy: key plumbing, dev wipe, docs, rule, harness defaults (0 Rust lines)

### Task 8.1: Version check

### Task 8.2: `listen.env` in the three deploy paths + the units

- [ ] `pipeline.yml` (dev, `/etc/presenter`), `deploy-site.yml` (SNV + PRBA, `${{ env.ETC_DIR }}`), `release.yml`
  (PP): a step "Configure listening transcription" next to "Configure AI backend", same shape:

```yaml
      - name: Configure listening transcription
        env:
          LISTEN_SONIOX_API_KEY: ${{ secrets.LISTEN_SONIOX_API_KEY }}
          LISTEN_SONIOX_MODEL: ${{ vars.LISTEN_SONIOX_MODEL }}
        run: |
          set -euo pipefail
          SECRET_FILE=${{ env.ETC_DIR }}/listen.env
          if [ -n "$LISTEN_SONIOX_API_KEY" ]; then
            ssh deploy-target "sudo install -d -m 755 ${{ env.ETC_DIR }}"
            printf 'PRESENTER_LISTEN_SONIOX_API_KEY=%s\nPRESENTER_LISTEN_SONIOX_MODEL=%s\n' \
              "$LISTEN_SONIOX_API_KEY" "${LISTEN_SONIOX_MODEL:-stt-rt-v6}" \
              | ssh deploy-target "sudo sh -c 'umask 077 && cat > $SECRET_FILE'"
            echo "Listening transcription key written to $SECRET_FILE (0600 root-only), model=${LISTEN_SONIOX_MODEL:-stt-rt-v6}"
          else
            ssh deploy-target "sudo rm -f $SECRET_FILE 2>/dev/null || true"
            echo "LISTEN_SONIOX_API_KEY unset — transcription has no key; any stale $SECRET_FILE removed"
          fi
          ssh deploy-target "sudo systemctl daemon-reload"
```

  (`pipeline.yml` / `release.yml` use their literal etc dir like their AI step does.) The step runs BEFORE the
  swap / restart step and is gated like its AI sibling (the #795 `ssh_setup` rule, `deploy-workflow-ssh.md`).
- [ ] Units: `scripts/deploy/presenter.service`, `presenter-dev.service`, `presenter-prba.service` (with its
  `/etc/presenter-prba`) get `EnvironmentFile=-<etc>/listen.env` with the AI comment's wording adapted.
- [ ] `scripts/ci/check_deploy_workflows.py`: assert each of the three workflows writes `listen.env` with `umask 077`
  and removes it on an empty secret; its unit test gets one failing and one passing case.
- [ ] `pipeline.yml` redirect step: `DELETE FROM app_settings WHERE key IN ('listen.recorder','listen.transcription','listen.session','listen.usage');`
  (message text updated).

### Task 8.3: Docs, rule, router row, harness defaults

- [ ] `docs/configuration.md` + `CLAUDE.md` env table: `PRESENTER_LISTEN_SONIOX_API_KEY`, `_MODEL`, `_URL`,
  `_ENDPOINT_DELAY_MS`.
- [ ] `docs/listen-providers.md` (new; spec §14): provider Soniox, model `stt-rt-v6`, credential `LISTEN_SONIOX_API_KEY`
  → `listen.env`, measured figures (§22.2), the [unverified] list, the live gate "C7".
- [ ] `.claude/rules/listen-asr.md` (new; `paths:` `listen/soniox/**`, `listen/asr.rs`, `listen/transcription/**`,
  `listen/transcript_hub.rs`, core `listen/transcript.rs`, `listen/ws.rs`, `listen/transcription.rs`,
  `listen/context.rs`, `router/listen_ws.rs`, `tests/e2e/fake-soniox.ts`, `tests/e2e/listen-transcript.spec.ts`):
  no WS pings + keepalive, Bearer header only, measured config, gate / park / resend-from-last-final / rollover
  numbers, guest-mic-only, the cap counts connected time, the mock seam, dev hides the target. Router row in
  `CLAUDE.md`'s Playbook table.
- [ ] `tests/e2e/support.ts`: `startTestServer` sets `PRESENTER_LISTEN_SONIOX_URL=ws://127.0.0.1:9` (a dead loopback,
  like the NLT / Drive defaults) and no key, unless the spec passes its own.

### Task 8.4: Close the lane — post-deploy on SNV: `systemctl cat presenter | grep listen.env`; with the secret not
yet set, `/etc/presenter/listen.env` absent and the deploy log shows the "unset" line.

---

## Lane C3-9 — UI: "Prepis hosťa" + the interpreter's EN column + E2E against a fake Soniox (≈ 480 UI lines)

### Task 9.1: Version check

### Task 9.2: Settings section (`pages/settings/listen_transcription.rs`, new; mounted in `listen.rs`, +3 lines)

- The paid switch "Prepis hosťa (Soniox)", a `<select>` "Mikrofón hosťa" over the published mics (`prop:selected`
  per option, `stream-editor-ui.md` rule), "Denný limit (min)" number field, buttons "Spustiť teraz" / "Zastaviť" /
  "Automaticky (Kázeň)" (`PUT /listen/session`), the key warning when `keyConfigured` is false, one status line:
  state (`row_status::state_label`), reason in Slovak, `connected / cap` minutes, "≈ $0.xx dnes", latency p50 / p90.
  Existing classes only (`settings-ui.md`: `ui-button` / `ui-toggle`, `.settings__form-row`); the 2 s status poll
  of the card is reused, no second poll. `api/listen.rs`: `put_transcription`, `put_session`.

### Task 9.3: `/listen/ws` client + the EN column

- `ws/listen.rs` (new): connect, reconnect with the `ws::passive` back-off, resync on visibility (A+B §6.7) — every
  (re)connect starts with the server's snapshot, so no separate GET.
- `components/interpreter/transcript.rs` (new): `TranscriptModel` (sentences by id, ≤ 300 kept, the tail, the session
  state) + `TranscriptColumn` view (Decision 15): `data-role="transcript"`, sentences `data-role="transcript-sentence"`,
  tail `data-role="transcript-tail"` (`data-partial` span lighter), `data-role="transcript-live"` button "▼ Naživo ·
  N nové", banner `data-role="transcript-banner"`; keyed `<For>` by sentence id (`ui` skill).
- `pages/interpreter.rs`: `data-transcript="present"` once the snapshot has a session or a sentence; the tips keep
  their column (§8.2, 62 / 38).
- `styles/interpreter.css`: EN 24 px, tail lighter, older dimmed, banner; touch targets ≥ 44 px.

### Task 9.4: The fake Soniox + E2E

- `tests/e2e/fake-soniox.ts` (new): a `ws` server on port 0; records the upgrade's `authorization` + `user-agent`, the
  first text frame (config JSON), binary bytes (→ audio ms at 32 bytes/ms), control frames, and a `ping` counter;
  replays `fixtures/listen/soniox-basic.jsonl` lines `{ "atAudioMs": n, "response": {...} }` as audio arrives;
  `finalize` → the pending tokens as finals + `<fin>`; empty text → `{finished: true}` + close; `kill()` (outage) and
  `restart()`.
- `tests/e2e/listen-transcript.spec.ts` (new), one server with `PRESENTER_LISTEN_SONIOX_URL` = the fake, key
  `e2e-test-key-not-real`, a VBAN mic on `127.0.0.1:0` (`PUT /listen/mics`):
  - [ ] Settings (desktop): switch on, pick "Hosť (test)", cap 30 → `/listen/status` reflects it; the key warning is
    absent; with no key (second server) the warning shows.
  - [ ] "Spustiť teraz", then `vban_send.py --to <port> --stream E2E_GUEST --seconds 4 --amplitude 0.5` (voiced) →
    on `/ui/interpreter` (tablet 1180×820 touch) the tail shows the fixture's partial words, then the sentence turns
    final; the fake saw `Bearer e2e-test-key-not-real`, a config with model `stt-rt-v6`, `pcm_s16le`, 16000,
    `language_hints ["en"]`, `context.terms` ⊇ {"Habakkuk", "James"}, and NO `api_key`.
  - [ ] `fake.kill()` → banner "Služba prepisu nedostupná — skúšam znova" within 3 s (C-S7); `fake.restart()` + more
    audio → the banner goes and new words arrive.
  - [ ] scroll the column up → "▼ Naživo · N nové" appears with N > 0 after new sentences; tap → back at the bottom.
  - [ ] the fake's ping counter is 0; zero console errors / warnings (last assertion).

### Task 9.5: Close the lane — post-deploy on SNV (GET-only): Settings → Počúvanie shows "Prepis hosťa" with the
switch OFF; `/ui/interpreter` still shows the tips area full width (no session, `data-transcript="absent"`).

---

## Lane C3-10 — real-API acceptance on dev (manual; after C3-1 … C3-9 are deployed)

Not a PR. Results go on #852 and into `listen-asr.md` / `docs/listen-providers.md` (a docs PR if anything changed).

### Task 10.1: The key into GitHub (no owner action — the key is in the dev2 vault)

- [ ] `gh secret set LISTEN_SONIOX_API_KEY --repo zbynekdrlik/presenter < ~/.secrets/listen-soniox-api-key`
  (stdin, never argv / chat); re-run the latest `pipeline.yml` deploy-dev job; `curl -s
  http://10.77.8.134:8080/listen/status | jq .transcription.keyConfigured` → `true`. (SNV / PRBA / PP get the file
  on their next deploy; nothing runs there while the switch is off.)

### Task 10.2: Replay through the REAL Presenter path (dev only; dev's rows are wiped at every dev deploy)

```bash
DEV=http://10.77.8.134:8080
curl -s -X PUT -H 'content-type: application/json' $DEV/listen/mics -d '[{"id":"guest","name":"Hosť (test)","streamName":"ACC_GUEST","senderIp":"127.0.0.1","channel":"left","language":"en","role":"guest"}]'
curl -s -X PUT -H 'content-type: application/json' $DEV/listen/transcription -d '{"enabled":true,"guestMic":"guest","dailyCapMinutes":120}'
curl -s -X PUT -H 'content-type: application/json' $DEV/listen/session -d '{"mode":"on"}'
OUT=~/.claude/work-products/presenter/c3-acceptance/$(date +%F); mkdir -p "$OUT"
node -e 'const fs=require("fs");const o=fs.createWriteStream(process.argv[1]);const w=new WebSocket("ws://10.77.8.134:8080/listen/ws");w.onmessage=m=>o.write(JSON.stringify({at:Date.now(),msg:JSON.parse(m.data)})+"\n");w.onclose=()=>process.exit(0);' "$OUT/ws.jsonl" & TAP=$!
date +%s%3N > "$OUT/replay_start_ms"
python3 scripts/dev/listen/vban_send.py --to 127.0.0.1:6980 --stream ACC_GUEST \
  --file ~/.claude/work-products/presenter/eval-audio/en-only/LwFI3gwp7hU.16k.wav --start 180 --duration 600
sleep 20; curl -s $DEV/listen/status | jq . > "$OUT/status_after.json"; kill $TAP
```

- [ ] Score with the eval's scorer (`~/.claude/work-products/presenter/eval-audio/en-only/runs/timeline_score.py`,
  adapted to read the tap's `tail` / `sentence` messages on the replay clock) against the same three references.
  **Pass:** WER ≤ 2.4 % (eval 1.9 % + 0.5 pt), visible p50 ≤ 0.9 s / p90 ≤ 1.8 s (C-S2 as amended, §22.7), 1
  connection, 0 errors, `framesDropped` ≈ 0.
- [ ] The Shuttlesworth reference windows (`darlKC25jLs.refwin.wav`, 31 min): **≥ 37 / 38 citations** present in the
  sentences (eval 38 / 38), the "King James" passages recorded as-is for C4's corpus.

### Task 10.3: The open questions (each result → #852 + the rule's [unverified] list)

- [ ] **Gate + keepalive:** replay 30 s, 2 min nothing, 30 s → same connection (`connections` unchanged), the first
  word after the pause present (pre-roll), a `<fin>` seen in the tap's text flow as a closed sentence at the gate.
- [ ] **Park:** 6 min nothing → the connection closed; replay → `connections + 1`, no banner.
- [ ] **Bad key:** a later environment file (systemd: `EnvironmentFile=` beats `Environment=`):
  `printf 'PRESENTER_LISTEN_SONIOX_API_KEY=invalid-key-for-test\n' | sudo sh -c 'umask 077 && cat > /etc/presenter/listen-badkey.env'`
  + a named drop-in `/etc/systemd/system/presenter-dev.service.d/c3-acceptance.conf` with
  `EnvironmentFile=-/etc/presenter/listen-badkey.env` (never `systemctl edit` / `revert` — `gst-debug.conf` must
  stay) → status `stopped` / `auth`, ONE connection, no retry; record `error_code` / `error_type`; if `classify` was
  wrong → a RED test with the real text, then the fix. Remove both files, daemon-reload, restart.
- [ ] **Endpoint detection:** the same Sloan slice with `PRESENTER_LISTEN_SONIOX_ENDPOINT_DELAY_MS=1000` and `1500`
  (same drop-in mechanism): WER, visible p50 / p90, final-word latency, sentences closed by `<end>`. Keep it as the
  default only if WER does not rise and finals come sooner → a follow-up PR changing the default + the rule.
- [ ] **`context.text` for references** (voiceagent's lever): the Shuttlesworth windows again with `context.text` =
  "The preacher cites Bible references as book, chapter and verse; write them as John 10:10 or Mark 10:29-30. King
  James, Amplified, NIV, ESV and NKJV are names of Bible translations, not books." → count bare digit pairs and
  "King James" artefacts vs the run without it; WER must not rise. Adopt it (a core constant + RED test of the config
  JSON) only if both counts drop.
- [ ] **Context size:** one connection with the largest real context (66 books + a long prepared list + 50 tip terms)
  → accepted; if refused, lower `CONTEXT_MAX_CHARS` with a RED test.
- [ ] **Cost:** compare `connectedMsToday` / `audioMsToday` with the Soniox console's usage for the day (the parked
  share; whether keepalive time is billed).
- [ ] **Clean up dev:** `PUT /listen/session {"mode":"auto"}`, `PUT /listen/transcription {"enabled":false,"guestMic":null,"dailyCapMinutes":240}`,
  `PUT /listen/mics []`.

---

## After C3 (not part of this plan)

- **C4 — verse detection** (spec §7 with §22.8): the next plan. Reads the hub's closed sentences of the guest mic;
  corpus includes bare "John 10 10", "Mark 10:29 to 30", and translation names ("King James", "Amplified") that must
  never become a book; adds `LiveEvent::BibleSuggestions`, the console "Návrhy AI" section and the listening chip
  (calls `PUT /listen/session`).
- **C3-persist** — `listen_transcript` 7-day store + the restart seed (spec §12; the Gemini plan's lane C3-8 design is
  engine-neutral and reusable).
- **C5a** — the rest of the interpreter tablet (⏸, A−/A+, Bible area, numbers + names); **C6** — the SK suggestion
  (spec §22.10, decided after the EN transcript ran at a real service); **C5b** TV; **C7** hardening (language
  mismatch, the provider live gate, AssemblyAI Universal-3.6 Pro as the failover only if Soniox fails in production).

## Self-review (against the spec, 2026-11-14)

- **Spec coverage:** §22.3 engine + measured config + context (C3-2, C3-3); §22.4 protocol incl. no pings + keepalive
  + Bearer (C3-3, C3-5); §22.5 guest mic setting, lazy connect, gate, park, reconnect + resend, stop classes, storm
  guard, rollover (C3-4, C3-7); §22.6 assembly (C3-1); §22.7 C-S2 measured (C3-10); §6.4 start / stop / paid gate /
  cap / meter (C3-2, C3-7); §4 / §8.1 `/listen/ws` + snapshot (C3-6) and the EN column (C3-9); §11 + §22.12 banners'
  data (C3-7) and the EN-column banners (C3-9); §12 keys (C3-8); §14 logs + status + `docs/listen-providers.md`
  (C3-5, C3-7, C3-8); §15 tests (every lane). Out by the amended phasing: verse detection (C4), persistence
  (C3-persist), SK suggestion (C6), TV (C5b), language mismatch + live gate (C7).
- **Placeholders:** interfaces and tests are named; the code is written in the lanes (no code in this plan was
  compiled). One deliberate locate step: today's prepared references in `state/sermons` (Task 7.3).
- **Type consistency:** `AsrToken` / `TokenBatch` (C3-1) are what `wire::to_tokens` (C3-3) produces and `Action::Emit`
  (C3-4) / `AsrEvent::Tokens` (C3-5) carry; `MicTranscription` / `TranscriptionState` / `OffReason` (C3-1 types, C3-2
  logic) are the hub's `session` payload (C3-6), the supervisor's status (C3-7) and the UI's model (C3-9);
  `ContextTerms` (C3-2) feeds `SonioxConfig` (C3-3) via `AsrSessionConfig` (C3-5).
- **Review focus:** each of the five lines has its tests in the owning lane (listed above).
