---
paths:
  - "crates/presenter-server/src/listen/soniox/**"
  - "crates/presenter-server/src/listen/asr.rs"
  - "crates/presenter-server/src/listen/transcription/**"
  - "crates/presenter-server/src/listen/transcript_hub.rs"
  - "crates/presenter-server/src/listen/transcript_hub/**"
  - "crates/presenter-server/src/listen/transcript_store.rs"
  - "crates/presenter-server/src/listen/transcript_store/**"
  - "crates/presenter-persistence/src/repository/listen_transcript.rs"
  - "crates/presenter-server/src/state/listen_transcription.rs"
  - "crates/presenter-server/src/state/listen_transcription/**"
  - "crates/presenter-server/src/router/listen_ws.rs"
  - "crates/presenter-server/src/router/listen_ws/**"
  - "crates/presenter-core/src/listen/transcript.rs"
  - "crates/presenter-core/src/listen/transcript/**"
  - "crates/presenter-core/src/listen/ws.rs"
  - "crates/presenter-core/src/listen/transcription.rs"
  - "crates/presenter-core/src/listen/transcription/**"
  - "crates/presenter-core/src/listen/context.rs"
  - "crates/presenter-core/src/listen/context/**"
  - "tests/e2e/fake-soniox.ts"
  - "tests/e2e/listen-transcript.spec.ts"
  - "tests/e2e/fixtures/listen/**"
  - "crates/presenter-ui/src/pages/settings/listen_transcription.rs"
  - "crates/presenter-ui/src/components/interpreter/transcript.rs"
  - "crates/presenter-ui/src/ws/listen.rs"
---

# Guest transcription — Soniox real-time (#852 C3, spec §22)

Plan: `docs/superpowers/plans/2026-11-14-listen-c3-soniox.md`. Provider facts, measured figures and the
[unverified] list: `docs/listen-providers.md`. Env vars + the key file: `docs/configuration.md`
("Sermon listening — guest transcription").

## The rules in one place

- **Never a WebSocket Ping from the client.** Soniox does not answer pings promptly (a pinging Python
  client died at 234 s, 1011). Liveness = the response cadence while audio flows (p50 0.19 s, max
  1.19 s → the 15 s stall rule) and `{"type":"keepalive"}` every 5 s while gated (Soniox needs ≤ 20 s).
  tungstenite still answers a SERVER ping by itself.
- **The key goes only in the upgrade's `Authorization: Bearer` header** — never the deprecated
  `api_key` config field, never a log, `Debug`, status or error text (C3-3 / C3-5 below).
- **The config is the measured one** (`stt-rt-v6`, `pcm_s16le` 16 kHz mono, `language_hints: ["en"]`,
  the `context` of spec §22.3, no translation, endpoint detection OFF unless
  `PRESENTER_LISTEN_SONIOX_ENDPOINT_DELAY_MS` is set). A change to it is a measured decision (C3-10 /
  the owner), never a tweak.
- **The numbers** (`listen/soniox/machine.rs` consts, each pinned by a test): lazy connect on the first
  voiced frame (> −50 dBFS) with a 1 s pre-roll; gate after 10 s without voice → ONE `finalize`, then
  keepalives; park after 5 min gated (empty TEXT frame = end), the next voice reconnects without
  backoff; reconnect after 1, 5, 20, 60, 60 … s and resend from the end of the last FINAL (≤ 5 s
  replay); stall = 15 s without a response while ≥ 10 s of audio went out; stop without retry on
  401 / 402 / 403 / 400 and on > 20 connections in 10 min; rollover from 280 min at a gated moment,
  295 min regardless (`<fin>` wait ≤ 3 s).
- **Only the guest mic** (`listen.transcription.guestMic`) is transcribed; the interpreter mic never
  goes to the cloud in C3.
- **Cost:** the daily cap counts CONNECTED time (not voiced time); no voice = no connection.
- **The mock seam:** tests never reach Soniox — the scripted server `listen/soniox/tests/mock.rs`
  (`Connector` over `tokio::io::duplex`), a fake `StreamingAsr` for the supervisor, and in E2E
  `tests/e2e/support.ts` points `PRESENTER_LISTEN_SONIOX_URL` at the dead loopback `ws://127.0.0.1:9`
  with an EMPTY key (empty = unset); a spec that needs Soniox sets its own fake URL + a
  `e2e-test-key-not-real` key in `process.env`. Keys in tests: `test-key-not-real` (Rust) /
  `e2e-test-key-not-real` (E2E), never a real one.
- **Deploy (C3-8):** every service deploy job (`pipeline.yml`, `deploy-site.yml`, `release.yml`) runs
  "Configure listening transcription" before the start (#999: the same step also writes the Gemini
  key of the guest translation, `listen-guest-translate.md`): the `LISTEN_SONIOX_API_KEY` secret →
  0600 `<etc dir>/listen.env` (`printf` piped into ssh stdin, written under `umask 077`), an empty
  secret REMOVES the file and the deploy goes on; the units read it with `EnvironmentFile=-`.
  Invariant 7 of `scripts/ci/check_deploy_workflows.py` pins the step to ONE canonical text
  (`LISTEN_STEP_RUN`; only the etc dir varies) and keeps the secret out of every other step: edit the
  checker and the three workflows together.
- **Dev never inherits prod's transcription:** the dev deploy deletes `listen.transcription`,
  `listen.session` and `listen.usage` (with `listen.recorder`) after copying prod's DB — no switch,
  no override, a fresh meter. A new transcription settings row joins that `DELETE`.

## Per lane

- **Transcript assembly (#852 C3-1, core `listen/transcript.rs`, pure):** one `TranscriptAssembler` per
  transcription session (`flush` restarts the connection numbering). Soniox sends a connection's FIRST
  token without a leading space, so a final of another connection than the open sentence's last one is
  a word start (decides a pending close, counts for the 20-word cap, gets one space unless it is
  punctuation); a marker closes only a sentence with no finals of a NEWER connection (a rollover's late
  finalize answer is "its last finals + `<fin>`" in one response); session ms and a sentence's end
  never move back; untimed tokens take the newest ms. The `/listen/ws` JSON shape lives once in core
  `listen/ws.rs` (camelCase, `type` tag) — server, UI and E2E share it. The test fixture
  `listen/testdata/soniox-assembly-fixture.jsonl` is hand-written (own text): never commit an eval log
  of a sermon.
- **Transcription rules + context (#852 C3-2, core `listen/transcription.rs`, `listen/context.rs`, pure):**
  ONE `start_decision` (switch → guest mic chosen → published → key → cap → today's `listen.session`
  choice → Kázeň with the 60 s stop delay) used by the supervisor AND the status; the cap counts
  connected ms (`>=` = reached), a start after it adds 60 min to today's row only. Save a
  `listen.transcription` row with `validate_transcription_change(new, stored, published)`: a mic the
  stored row names stays valid while missing (spec gap 7 — the switch must go off then). Context terms:
  books (generated `BOOK_LONG_EN` — run `gen_bible_book_names.py`, never hand-edit) → today's refs
  (`reference_term`, "Psalm 23:1") → tips → curated; dedup lowercase after collapsing whitespace; the
  8 000-char cap counts compact serde_json chars (`context_json_chars`, pinned against serde_json — core
  has serde_json only as a dev-dependency); a new wire field of `ListenStatus` is `#[serde(default)]`
  and every `ListenStatus {` literal (server `health.rs`, two presenter-ui tests) gets it.
- **Soniox wire + env (#852 C3-3, server `listen/soniox/{wire,config}.rs`, pure):** the key goes ONLY in
  the upgrade's `Authorization: Bearer` header — never an `api_key` config field (deprecated, refused
  from 2027-01-15), never in `Debug` (`SonioxEnv` prints `<set>`/`<unset>`) or a log (`wire::redact`).
  An empty TEXT frame ENDS a Soniox stream, so no message builder may ever fall back to `""`.
  `classify` reads `error_code` first, then `error_type` (docs: branch on the type, not the message);
  `classify_close` reads an `error_type` in the reason, then the close code (1001 = idle Timeout,
  1002/1003/1007/1009 = ours → BadRequest stop, 1011–1014 = Server). Any `_ENDPOINT_DELAY_MS` number,
  0 included, turns endpoint detection on (clamped 500–3000). A `Serialize` struct of an EXTERNAL
  snake_case wire needs an explicit `#[serde(rename_all = "snake_case")]`: `quality-check.sh`'s serde
  convention otherwise fails it for missing camelCase. `tokio-tungstenite` 0.29 with
  `rustls-tls-webpki-roots` is a normal dependency (adds only `webpki-roots 0.26.11`; rustls's
  provider stays `ring`). Tests use `test-key-not-real` and loopback URLs only.
- **Soniox session machine (#852 C3-4, server `listen/soniox/machine.rs` + `machine/{reports,timers,audio}.rs`,
  pure):** `SonioxCore` = `on_input` / `on_report` / `on_tick(now)` → `Vec<Action>` + `next_deadline()`; no
  tokio, no clock read (a `Response` report brings its own `received_at`). Session ms = the position in the
  audio SENT: a frame gets its ms at its FIRST send (a gated or dropped frame never gets one, a resend keeps
  them), so every connection is contiguous from its `offset_ms`. Confirmed ms = a final's end, or the audio a
  `<fin>` answered — one cover per `finalize`, popped in order (never one `Option`: a second gate before the
  first answer would confirm too much). A resend starts exactly at the confirmed ms (the first frame cut, ≤ 5 s
  back); tokens of an ended ("draining") connection are ignored — the resend recreates them. The pre-roll is
  the last SECOND of unsent frames (time, not count), drained on EVERY onset, also while no connection takes
  audio. Onset latency counts only tokens at/after the onset's session ms. A retry or an unexpected `finished`
  while nobody spoke for 10 s parks instead of connecting. The machine has no connect deadline: the driver
  reports `ConnectFailed` (with the HTTP `status` of a refused upgrade, so 401/402/403/400 stop) after its own
  timeout. Child modules hold `impl SonioxCore` blocks: their methods are `pub(super)`, the parent's helpers
  stay private (`pub(super)` there leaks the private `Live` type → `private_interfaces` under `-D warnings`).
  Tests: hand-made `Instant`s, a frame's first two samples carry its index.
- **Soniox driver (#852 C3-5, server `listen/asr.rs`, `listen/soniox/{conn,session}.rs`):** `StreamingAsr::open`
  = one task per session (`SonioxAsr`), channels in/out; the driver only executes `SonioxCore`'s actions (no
  rule of its own) and feeds it `tokio::time::Instant::now().into_std()` (the paused test clock drives it).
  One connection = one task (`conn::spawn_connection`): 10 s connect timeout → `ConnectFailed` (status of a
  refused upgrade; a URL / key that makes no request = 400 → stop, never a retry loop), audio as BINARY
  s16le, empty TEXT = end, ONE closing report, never a WS ping; its command channel is unbounded (a stuck
  socket must not stall the session task, the stall rule bounds it). The transport is the `Connector` seam
  (TCP in production, `tokio::io::duplex` in tests) and both use the ONE `conn::handshake`. Key safety:
  `Authorization` is `set_sensitive`, the `State` error text is redacted at the SOURCE (the status shows it),
  and `main.rs` caps `tungstenite::handshake` at INFO — its TRACE line prints the raw upgrade with the key.
  The reconnect WARN streak ends on EVERY state but `reconnecting` (a park between outages too, #484 reset).
  Tests (`listen/soniox/tests/`): the scripted server (`mock.rs`: `Step`s, upgrade headers, a log of every
  client frame) on `start_paused`; a tungstenite upgrade callback needs `#[allow(clippy::result_large_err)]`
  (the http error response is its fixed type).
- **Transcript hub + `GET /listen/ws` (#852 C3-6, server `listen/transcript_hub.rs`, `router/listen_ws.rs`):**
  ONE `TranscriptHub` (a cheap clone of one `Arc`) owned by `ListenService` (`transcripts()`), never a
  `LiveEvent`. Every message is sent WHILE the hub's std mutex is held, and `subscribe` takes the snapshot
  AND `tx.subscribe()` under that same lock — the only way a reader's snapshot + stream join with no gap and
  no double (never take the receiver outside the lock; the threaded race test, which catches that, checks ≥ 100
  rounds). Newest 500 sentences, ids from 1, never reused; a 512-message lag → the reader subscribes again
  (fresh snapshot, `resyncs`, INFO). Tails: ≤ 1 per 100 ms per mic (tokio `Instant`, the paused clock drives
  the tests); a coalesced tail is sent by the hub's own flush task at the window's end, which re-arms when a
  forced send restarted the window; `push_sentence` clears the mic's stored tail and its pending flush (the
  closed words never show twice) and the next `set_tail` goes out at once. An empty tail (`None` / no text)
  is sent with empty texts and left out of the snapshot. `session` only on a state / reason / error change.
  The socket: snapshot, stream, a WS Ping every `KEEPALIVE` (30 s; 1 s under `cfg(test)`) because the hub
  is quiet outside a sermon and Cloudflare cuts an idle socket at ~100 s — a const assert keeps 2 × 30 s under it; no frame from the client for two
  intervals ends it (`WsEndReason::ClientSilent`). End reasons are `/live/ws`'s `WsEndReason` +
  `close_frame_reason`; client IP via `extract_client_ip`. Logs on `presenter::listen::ws` (connect,
  disconnect with `reason` / `resyncs` / `connected_ms`, resync). The upgrade is NOT a poll: it stays out of
  `request_log.rs`'s quiet lists. Tests: a real axum server on loopback + `tokio_tungstenite`; the lag test
  runs on the CURRENT-thread runtime (600 synchronous pushes → exactly 88 missed) with `Capture`.
- **Transcription supervisor (#852 C3-7, server `listen/transcription/{mod,mic,meter}.rs`,
  `state/listen_transcription.rs`):** ONE supervisor task (`ListenService::start_transcription`, from
  `start_listen` after the three rows are read under the save lock — never in validate mode) runs core
  `start_decision` on every rows / mic-list change and a 1 s tick that also reads `AppState.stage_layout`
  (the moment it left `preach` starts the 60 s delay); it never adds a channel into `state/mod.rs`. A
  running session = ONE mic task: frames → `rms_dbfs` → `try_send` (a full input = a counted drop, a lagging
  frame channel = counted lost frames, both #484-gated WARNs — the chain never waits), events → assembler →
  hub (`push_sentence` / `set_tail`), `State` / `Usage` (session RUNNING TOTALS → the mic task adds deltas
  to the meter) / `Onset` → `Transcription`'s std-mutex view. Stop = `AsrInput::Stop` + drop the sender,
  wait ≤ 5 s for the task's `Finished` reason, else abort. Any `Finished(Some(reason))` blocks the rows the
  session STARTED under (`Running.key`) until `BlockKey` changes (day, switch, guest mic, or a NEW
  `PUT /listen/session` — `Rows.choice` counts them, so repeating "on" retries; a cap change does not); the
  block check runs again AFTER a stop (a refusal returned by that stop names the OLD rows and is lifted at
  once, so the new guest mic starts in the same decision). A session that ends by itself without a reason,
  or whose task panicked, is retried after `bind_retry_delay` (1, 2, 4 … 60 s; reset by a session that ran
  ≥ 60 s and by ANY rows change — an operator's "on" starts at once), shown as `reconnecting` meanwhile,
  WARN and the restart INFO on power-of-two ends only — never reopened on the next tick. A PUT answers only after the
  supervisor decided on its rows generation (`Transcription::decided_on`, ≤ 7 s; no wait when no
  supervisor runs). Meter counters: `connections` = connection numbers that delivered words or ended,
  `reconnects` = entries into `reconnecting` (an outage, not each retry), lost frames summed. `listen.usage` = the meter's row, written every 60 s while
  it changed and at every session end (a failure keeps it dirty, WARN gated); `listen.transcription` /
  `listen.session` are written only by the PUTs (`validate_transcription_change` → 422, one audit row each,
  audit WARN under `presenter::listen::asr`). The status shows only the CURRENT guest mic; a guest mic that
  was replaced is sent to `/listen/ws` once as `off` / `noGuestMic`. Context = today's sermon items
  (`reference_term(book_number − 1, …)`) + active tips' `source_term`, read per start (a read error = fixed
  terms only + WARN). Tests: `listen/transcription/tests.rs` (real `ListenService`, fake `StreamingAsr`
  answering a Stop / closed input with `Finished(None)`, fake store, `start_paused`, settable day clock via
  `set_recorder_env`); endpoints + the real sermon/tip context read in `state/listen_transcription/tests.rs`.
- **Transcript store (#852 C3-persist, migration `m20261121_000001_create_listen_transcript`, persistence
  `repository/listen_transcript.rs`, server `listen/transcript_store.rs`):** table `listen_transcript` (the
  sentence fields + TEXT `service_date`, index service_date + id), never an edit of the applied migration. The
  hub offers every closed sentence to ONE writer through a bounded channel (`STORE_CAPACITY` 256, `try_send`
  UNDER the hub lock — never an `.await`, never a DB call in `push_sentence`): full / closed = the row is
  dropped + counted, WARN on power-of-two drops of the streak, INFO when it stores again. The writer batches
  ≤ 64 rows per insert under `ListenService::today()` (the recorder's clock; a failed batch is lost, gated
  WARN, no retry loop) and deletes dates more than 7 days back once per local date (at start, then the first
  60 s tick of a new date; one attempt, failed or not). `transcript_store::start` (from `start_listen`, before
  the supervisor, never in validate mode) seeds the hub with today's newest 500 and sets the next id after the
  highest stored id of ANY date and every seeded id (a reader drops an id not above its last; a failed max
  read still continues after the seeded ones); the seed sends a `snapshot` message (a reader may connect
  before `start_listen` reads the store). `service_date` = the local date when the BATCH is written (a
  sentence closed just before midnight can land on the next date). Values cross the column ↔ wire boundary through the typed
  `ListenTranscriptError` (u64 > i64 refused on write, a negative stored value refused on read). No read API:
  the seeded `/listen/ws` snapshot is the reader. The `TranscriptRows` trait is the seam (tests: `FakeRows`).
- **UI + E2E (#852 C3-9, presenter-ui `pages/settings/listen_transcription.rs`, `components/interpreter/transcript.rs`,
  `ws/listen.rs`):** the "Prepis hosťa" section reads the Počúvanie card's ONE `/listen/status` poll (never a
  second poll) and calls the card's `refresh` after every PUT (a poll answered after the write must not show
  the old setting); a refused PUT toasts and restores the controls from the stored setting. Without a guest
  mic the status has no `MicTranscription`: the section derives the reason (Disabled / NoGuestMic) and the
  cap from the setting. The `/listen/ws` client is NOT `ws::passive` (that one drops a socket silent for 5 s;
  `/listen/ws` is quiet outside a sermon, only server pings the page never sees) but shares its back-off,
  `Generation` and `watch_long_hide`; its read loop checks the generation every 500 ms. The column changes its
  three signals IN PLACE through pure helpers (a tail every ~0.2 s must not copy 300 sentences); it shows only
  with text, a session not `off`, or a banner reason; `gated` / `parked` are normal pauses, never a banner
  ("no sound" needs the mic's packet age). The E2E fake replays its fixture in a LOOP over the audio since its
  (re)start, assigns token times itself, `kill()` refuses connects; after `restart()` recovery is the 1 / 5 / 20 s
  back-off (≤ 26 s). The no-key warning is pinned in `settings-listen.spec.ts` (its server has no key; a second
  server per file would fight over the mock Resolume port 8091). Payload: the lane added raw +146 kB / br +23.5 kB
  / gz +39 kB (local delta) — check `ui_size_budget.json` headroom before growing this UI.
