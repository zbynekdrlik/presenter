---
paths:
  - "crates/presenter-server/src/listen/translate/**"
  - "crates/presenter-server/src/router/listen_guest.rs"
  - "crates/presenter-server/src/router/listen_guest/**"
  - "crates/presenter-server/src/ui/guest/**"
  - "crates/presenter-server/src/router/guest_host.rs"
  - "crates/presenter-server/src/router/guest_host/**"
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

# Guests' live translation — the guest hostname (#999)

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
  first_voiced_ms median/max — onset after ≥ 1.5 s quiet → first translated audio, stalls = 6 s
  without output while the preacher spoke, reconnects, dropped frames). A silent minute logs nothing.

## The hostname (`router/guest_host.rs`, ROZHODNUTÉ 27.11)

- `PRESENTER_GUEST_HOST` (per-site drop-in, never the literal host in code): a request whose `Host`
  header (lowercase, port and trailing dot dropped — `config::bare_host`) is it goes to
  `listen_guest::guest_routes()` ONLY: `/` (WASM shell), `/manifest.json`, `/sw.js`,
  `/icon-192.png`, `/icon-512.png`, `/ui-pkg/*`, `/healthz`, `/listen/guest/languages`,
  `/listen/guest/ws`; every other path 404 (`GUEST_NOT_FOUND`, the guest router's own fallback). The
  guest `/healthz` is `{status, version, channel}` only — the full one names NDI addresses and AI
  errors. That hostname is a public tunnel with NO Access app,
  so this dispatch IS the security boundary: a new guest route goes into `guest_routes()` on
  purpose, nothing else. `X-Forwarded-Host` is never read (a client could aim it at the normal
  router through the guest tunnel); the URI authority counts too (HTTP/2, absolute-form). The layer
  sits outside the routes (inside the request log), so it also covers every fallback.
- It cannot see a tunnel that REWRITES Host (`originRequest.httpHostHeader` — never set it for the
  guest hostname) or a box whose drop-in went missing while the tunnel still points at it: the
  deploy's "Verify deployment" curls `/ui/operator`, `/listen/status` … with the guest Host and fails
  unless they are 404.
- Normal hosts never serve the guest page: their `/` stays the SSR home. They carry the guest
  endpoints (an operator can try them) and the operator's QR page `/ui/guest-qr`
  (`GET /listen/guest/qr` → `https://<guest host>/` + SVG from the `qrcode` crate; `url: null` on a
  box without a guest host).
- Deploy: `deploy-site.yml` input `guest_host` → the drop-in `guest-host.conf`
  (`PRESENTER_GUEST_HOST`, host-name characters only; empty removes it). SNV passes the repository
  variable `SNV_GUEST_HOST` (`listen.newlevel.media`, set 27.11); PRBA / PP / dev have none.
- The hostname itself (DNS + tunnel ingress) is set up by airuleset's publish-app lane, not here.

## The page (`presenter-ui` `pages/guest`, WASM route `/`)

- WASM like every UI here (house stack; the AudioWorklet module `guest-player.js` is the one JS file
  the platform requires, copied by Trunk to `/ui-pkg/guest-player.js`, unit-tested by
  `guest-player.test.js` under `node --test` (`npm run test:guest-player`, the Companion Tests job).
  It has no content hash, so `wasm_ui::asset_cache_control` serves an unhashed `.js` `no-cache`
  (every fingerprinted asset stays `immutable`). The WASM route `/` is the
  guest page — the server serves the shell at `/` ONLY on the guest host. The shell does NOT install
  `ui_usage` there (that host has no `/ui/usage`).
- Phone first, ENGLISH copy (owner 27.11: the page is for foreigners), language names in their own
  language (core `GUEST_LANGUAGES`); `?lang=` keeps the pick. Play = the gesture that creates +
  resumes the `AudioContext` synchronously in the tap (iOS unlock), asked for 24 kHz (a refusing phone gets a linear
  resample, `state::guest::resample`). The worklet starts at 150 ms queued and drops back to 150 ms
  when > 1 s queued (latency first). Screen Wake Lock via `navigator.wakeLock` (Reflect; not in
  stable web-sys), re-taken on `visibilitychange`, released on Stop / refusal / `stopped`; every
  Play and Stop bumps `Listening::start`, so a slow start of an earlier Play drops what it made.
- E2E hooks: `body[data-guest-chunks]` (speech frames received), `body[data-guest-played]` (samples
  the worklet played), `[data-role=guest-state][data-state]` (`idle` / `connecting` / `waiting` /
  `live` / `reconnecting` / `stopped` / `refused`).

## Tests

- Rust: `listen/translate/test_fake.rs` = the scripted fake Gemini (duplex `Connector`, records
  headers / setups / audio count, answers 100 ms of audio + `w<n> ` + a handle per audio message;
  `Script` = close on setup, goAway after n, close after n). Session tests on the paused clock;
  router tests (`router/listen_guest/tests.rs`) on a real loopback server. Keys `test-key-not-real`.
- E2E `guest-translate.spec.ts` + `fake-gemini.ts`: `PRESENTER_GUEST_HOST=localhost` (the other
  specs use 127.0.0.1, so `http://localhost:<port>/` IS the guest host); phone, pick → Play →
  `waiting` (nothing sent) → the real VBAN sender → `live`, chunks + played samples, transcript,
  original toggle; the guest host 404s everything else; zero console errors. `support.ts` gives
  every other server a dead Gemini URL, NO key and no guest host.
- Deploy: the Gemini key rides the SAME "Configure listening transcription" step as Soniox
  (invariant 7's canonical text in `scripts/ci/check_deploy_workflows.py`; both secrets, one file).
