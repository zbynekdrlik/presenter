---
paths:
  - "crates/presenter-server/src/listen/translate/**"
  - "crates/presenter-server/src/router/listen_guest.rs"
  - "crates/presenter-server/src/router/listen_guest/**"
  - "crates/presenter-server/src/ui/guest/**"
  - "crates/presenter-server/src/main.rs"
  - "crates/presenter-core/src/listen/guest.rs"
  - "crates/presenter-ui/src/pages/guest/**"
  - "crates/presenter-ui/src/state/guest.rs"
  - "crates/presenter-ui/src/ws/guest.rs"
  - "crates/presenter-ui/src/api/guest.rs"
  - "crates/presenter-ui/guest-player.js"
  - "crates/presenter-ui/styles/guest.css"
  - "tests/e2e/guest-translate.spec.ts"
  - "tests/e2e/fake-gemini.ts"
---

# Guests' live translation — the guest listener (#999)

A guest who does not understand Slovak opens the guests' own hostname (SNV: `listen.newlevel.media`,
ROZHODNUTÉ 27.11 — never a path like `/poslech`) on their own phone, picks a language and hears the
sermon translated live, with the text on screen. Design: #999 comment "DESIGN D1";
engine: "FINDING R2" (`gemini-3.5-live-translate-preview`, 3.2–3.6 s phrase-end lag EN/DE).

## The wire (raw probes 2026-11-27, no SDK — `listen/translate/wire.rs`)

- URL `wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent`,
  the key ONLY in the `x-goog-api-key` upgrade header (`set_sensitive`; never `?key=`, never a log /
  `Debug` — `GeminiEnv` prints `<set>`).
- Setup `{"setup":{"model":"models/<id>","generationConfig":{"responseModalities":["AUDIO"],"translationConfig":{"targetLanguageCode":"en"}},"inputAudioTranscription":{},"outputAudioTranscription":{},"contextWindowCompression":{"slidingWindow":{}},"sessionResumption":{…}}}`;
  audio `{"realtimeInput":{"audio":{"data":<b64 s16le>,"mimeType":"audio/pcm;rate=16000"}}}`.
- EVERY server frame is BINARY holding JSON (parse Text and Binary alike): `setupComplete`,
  `serverContent.modelTurn.parts[].inlineData` (`audio/pcm;rate=24000`, 250 ms chunks live),
  `outputTranscription` (the guest's text, arrives with its audio), `inputTranscription` (Slovak),
  `sessionResumptionUpdate {newHandle, resumable}` ~1/s, `goAway {timeLeft}`.
- A refusal is a CLOSE after the upgrade, never an HTTP status: **1007** = bad key → stop;
  **1008** + "not found" = bad model → stop; **1008 "client inactivity"** = the 120 s idle cut →
  reconnect AT ONCE with the handle (no back-off), so the next phrase never pays a connect.
- When the input stops, the model still speaks out its backlog: pausing the send loses nothing.
- The model streams audio ALL the time, its pauses included (a silent minute: `output_ms=60000`,
  `sent_ms=0`): "any output chunk" is never a latency or stall signal — only output above −45 dBFS
  (`session::OUTPUT_VOICED_DBFS`; R2: pauses ≤ −50, the first chunk after a pause ≥ −41.8).
- No fast / latency mode exists (doc 27.11.2026: `translationConfig` = `targetLanguageCode` +
  `echoTargetLanguage` only). The doc's 100 ms input chunks are what we send; 20 ms chunks measured
  60–100 ms sooner (3.32 vs 3.38 s, 3.33 vs 3.43 s, 5 runs each) — the 100 ms frame is the whole
  listen pipeline's unit, so that is a separate change, not done.
- `systemInstruction` (setup-only) IS honoured: `config::DEFAULT_CONTEXT` (override
  `PRESENTER_LISTEN_GEMINI_CONTEXT`) took "Newlevel Church" from 0/36 to 36/36 on Slovak TTS clips and
  removed "veríme" → "in Rome" and "zbor" → "choir" (#999 comment "FINDING L6c"). It also improves the
  INPUT transcription. Keep only public facts in it (the repo is public).
- Parallel sessions on one key: past the account's concurrency quota the server closes **1011 "You
  exceeded your current quota"** right after the setup (6 at once did; 4 languages is our cap) →
  `Retry`, the back-off.

## The session (`listen/translate/session.rs`, `mod.rs`)

- ONE session per ACTIVE language shared by every listener (`GuestTranslate::join`); the first
  listener starts it, it ends `LINGER` (60 s) after the last left (a reload keeps it). ≤ 4
  languages (`MAX_LANGUAGES`; a running language takes more guests when full). Cost scales with
  languages, never listeners.
- Source = `PRESENTER_LISTEN_GUEST_MIC` (default `dante-mic-1`), its 16 kHz broadcast frames. A box
  without that mic refuses (`noSource`) — PP until #991, PRBA, CI.
- Audio goes out only while the mic is voiced: a frame > −50 dBFS opens the gate, `HANGOVER` 2 s of
  quiet closes it, the last 1 s (`PRE_ROLL_FRAMES`) is sent with every onset. A silent mic = no
  messages = no cost. Frames before a connection is ready wait in `pending` (≤ 5 s, oldest dropped);
  a send NEVER opens a connection while a retry is scheduled (the back-off would be bypassed).
- `goAway` opens the next connection with the newest handle; at its `setupComplete` the audio moves
  there and the old one drains `DRAIN` 4 s — but its speech is DROPPED once the new connection spoke
  (`Conn::spoke`; two voices never overlap; counted `overlap_dropped_ms`). Any other close / connect
  failure backs off 1, 2, 4 … 30 s. An idle cut of a connection younger than `IDLE_CUT_MIN_AGE`
  (60 s) or never ready backs off too (no connect loop). `NO_ANSWER`: a ready connection that answers
  nothing for 10 s while audio goes out (counted from the gate opening, never across a quiet stretch)
  is dropped and reopened — a half-open TCP never errors, and this bounds its unbounded queue. A
  promoted, not-yet-ready successor shows `reconnecting`.
- Daily cap: `DAILY_CAP_MINUTES` = 480 audio minutes SENT per local day over every language
  (`DayMeter`, in memory — a restart starts the count again), charged only when a frame really goes
  out (`deliver`), never for one dropped from `pending`.
- `MAX_LISTENERS` = 300 over every language (`busy`); no per-IP cap — every guest on the church Wi-Fi
  shares one public address. A linger timer carries the entry's `epoch` (bumped by every join), so a
  leave → rejoin → leave ends the session 60 s after the LAST leave. Reached → the language stops
  (`stopped/dailyCap`), new joins are refused.
- Logs on `presenter::listen::guest`: session start/stop, every state change, connection open /
  ready / close (code + reason + class), goAway, back-off WARN (power-of-two), stall WARN, and ONE
  INFO `guest translation minute summary` per language (listeners, sent_ms, output_ms,
  `first_voiced_timed` + `first_voiced_ms` median/max, stalls = 6 s without translated SPEECH while
  the preacher spoke, reconnects, dropped frames, and our own segments in µs: `to_gemini_us_*` = a live
  mic frame reaching the session → written to Gemini's socket, `to_guest_us_*` = a Gemini frame read →
  written to each guest's WebSocket). A silent minute logs nothing.
- `first_voiced_ms` (#999 L6a): EVERY onset (a voiced input frame after `ONSET_GAP` 1.5 s quiet) waits
  in `Session::onsets` for the first VOICED output chunk that follows the model's own pause
  (`OUTPUT_GAP` 750 ms of unvoiced output audio, or 750 ms with no voiced output); speech before such a
  pause is the previous phrase's backlog. 750 ms from replaying R2 (0 ms read 0.0–0.2 s backlog, 500 ms
  once 0.14 s, 750 ms gives 2.0–3.6 s). Unanswered after 20 s → given up. Tests: the fake's
  `Script::quiet` ranges answer near-silence (the model's latency), everything else speech level.

## The guest listener (`PRESENTER_GUEST_PORT`, arch review 27.11 — fail CLOSED)

- The public no-login hostname (SNV `listen.newlevel.media`, ROZHODNUTÉ 27.11) is a Cloudflare
  tunnel that points at a SECOND TCP port, never at 80: `PRESENTER_GUEST_PORT` (SNV 8099, checked
  free on presenter.lan 27.11). `main.rs::spawn_guest_listener` binds it on the main port's
  interface and serves `router::build_guest_router` = `listen_guest::guest_routes()` (+ the request
  log) ONLY: `/` (WASM shell), `/manifest.json`, `/sw.js`, `/icon-192.png`, `/icon-512.png`,
  `/ui-pkg/*`, `/healthz`, `/listen/guest/languages`, `/listen/guest/ws`; every other path 404
  (`GUEST_NOT_FOUND`). The guest `/healthz` is `{status, version, channel}` only — the full one names
  NDI addresses and AI errors. A new guest route goes into `guest_routes()` on purpose, nothing else.
- Why a port and not Host routing on 80 (the first cut, reverted): Host routing fails OPEN — a lost
  `PRESENTER_GUEST_HOST` (or a tunnel that rewrites Host) handed the public hostname the WHOLE app.
  With a port, a missing setting = no listener = the tunnel answers 502, never the app. Unset → INFO
  "no guest listener"; a bind / serve failure → ERROR, the main server goes on; never in validate
  mode.
- The main port never serves the guest page: its `/` stays the SSR home, whatever the Host. It carries
  the two guest endpoints (an operator can try them) and the operator's QR page `/ui/guest-qr`
  (`GET /listen/guest/qr` → `https://<PRESENTER_GUEST_HOST>/` + SVG from the `qrcode` crate; `url:
  null` without one). `PRESENTER_GUEST_HOST` is ONLY that printed address now.
- Deploy: `deploy-site.yml` inputs `guest_port` + `guest_host` → the drop-in `guest-host.conf`
  (`PRESENTER_GUEST_PORT` digits only, `PRESENTER_GUEST_HOST` host-name characters only; both empty
  removes it). SNV: `guest_port: "8099"` in deploy.yml + the repository variable `SNV_GUEST_HOST`
  (`listen.newlevel.media`); PRBA / PP / dev have none. "Verify deployment" curls the guest port:
  `/` = 200, `/ui/operator`, `/ui/settings`, `/settings`, `/listen/status`, `/listen/mics`,
  `/libraries/summary` = 404, and fails if the main port's `/` is the WASM shell.
- The hostname itself (DNS + tunnel ingress to the guest port) is airuleset's publish-app lane
  (airuleset#1403), not here.

## The page (`presenter-ui` `pages/guest`, WASM route `/`)

- WASM like every UI here (house stack; the AudioWorklet module `guest-player.js` is the one JS file
  the platform requires, copied by Trunk to `/ui-pkg/guest-player.js`, unit-tested by
  `guest-player.test.js` under `node --test` (`npm run test:guest-player`, the Companion Tests job).
  It has no content hash, so `wasm_ui::asset_cache_control` serves an unhashed `.js` `no-cache`
  (every fingerprinted asset stays `immutable`). The WASM route `/` is the
  guest page — the server serves the shell at `/` ONLY on the guest listener. The shell does NOT install
  `ui_usage` there (that listener has no `/ui/usage`).
- Phone first, ENGLISH copy (owner 27.11: the page is for foreigners), language names in their own
  language (core `GUEST_LANGUAGES`); `?lang=` keeps the pick. Play = the gesture that creates +
  resumes the `AudioContext` synchronously in the tap (iOS unlock), asked for 24 kHz (a refusing phone gets a linear
  resample, `state::guest::resample`). The worklet starts at 150 ms queued and drops back to 150 ms
  when > 1 s queued (latency first); while playing it cuts a SILENT chunk (≤ −45 dBFS) so at most
  `TRIM` 650 ms stay queued (#999 L6b): the session's start burst otherwise stayed queued all session
  (R2 replay: 0.48–0.70 s standing). 650 ms = the lowest TRIM with no added gap in speech on the R2
  arrivals, also with ±80 ms mobile jitter (550 ms: 3–5 gaps / 5 min, 450 ms: 24); never lower the
  150 ms start (a 250 ms chunk always exceeds it, so it costs nothing). Screen Wake Lock via `navigator.wakeLock` (Reflect; not in
  stable web-sys), re-taken on `visibilitychange`, released on Stop / refusal / `stopped`; every
  Play and Stop bumps `Listening::start`, so a slow start of an earlier Play drops what it made.
- E2E hooks: `body[data-guest-chunks]` (speech frames received), `body[data-guest-played]` (samples
  the worklet played), `[data-role=guest-state][data-state]` (`idle` / `connecting` / `waiting` /
  `live` / `reconnecting` / `stopped` / `refused`).

## Tests

- Rust: `listen/translate/test_fake.rs` = the scripted fake Gemini (duplex `Connector`, records
  headers / setups / audio count, answers 100 ms of audio + `w<n> ` + a handle per audio message;
  `Script` = close on setup, goAway after n, close after n, idle cut after a time, deaf after n,
  late speech after a goAway). Session tests on the paused clock; the session's 1 s tick starts with
  the session, so a time rule (`NO_ANSWER`, stalls) fires on the first WHOLE second past its bound —
  feed past it (the deaf test feeds 12.5 s for a 10 s rule + 1 s retry), never shorten the rule;
  router tests (`router/listen_guest/tests.rs`) on a real loopback server, incl. the guest
  listener's 404 boundary (`build_guest_router`) and the main port's `/` staying the home. Keys `test-key-not-real`.
- E2E `guest-translate.spec.ts` + `fake-gemini.ts`: the test server gets `PRESENTER_GUEST_PORT`
  (a free port) and the page is `http://127.0.0.1:<guest port>/` (a secure context, so the
  AudioWorklet exists); phone, pick → Play →
  `waiting` (nothing sent) → the real VBAN sender → `live`, chunks + played samples, transcript,
  original toggle; the guest listener 404s everything else; zero console errors. The PLAYED-samples
  proof runs in Firefox through a private null-sink PulseAudio (`startNullSink`): the runner has no
  sound server (a browser never resumes its AudioContext without one), and dev2's Chromium never
  loads an AudioWorklet module at all — `addModule` stays pending without even fetching the file,
  headless or headed, with or without a sound server (probed 27.11.2026). Never assert
  `data-guest-played` in a Chromium test. `support.ts` gives
  every other server a dead Gemini URL, NO key, no guest port and no guest host.
- Deploy: the Gemini key rides the SAME "Configure listening transcription" step as Soniox
  (invariant 7's canonical text in `scripts/ci/check_deploy_workflows.py`; both secrets, one file).
