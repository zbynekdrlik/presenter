---
paths:
  - "crates/presenter-server/src/listen/translate/**"
  - "crates/presenter-server/src/router/listen_guest.rs"
  - "crates/presenter-server/src/router/listen_guest/**"
  - "crates/presenter-server/src/ui/poslech/**"
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

# Guests' live translation — `/poslech` (#999)

A guest who does not understand Slovak opens `/poslech` on their own phone, picks a language and
hears the sermon translated live, with the text on screen. Design: #999 comment "DESIGN D1";
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
  there and the old one drains `DRAIN` 4 s. Any other close / connect failure backs off 1, 2, 4 … 30 s.
- Daily cap: `DAILY_CAP_MINUTES` = 480 audio minutes SENT per local day over every language
  (`DayMeter`, in memory — a restart starts the count again). Reached → the language stops
  (`stopped/dailyCap`), new joins are refused.
- Logs on `presenter::listen::guest`: session start/stop, every state change, connection open /
  ready / close (code + reason + class), goAway, back-off WARN (power-of-two), stall WARN, and ONE
  INFO `guest translation minute summary` per language (listeners, sent_ms, output_ms,
  first_voiced_ms median/max — onset after ≥ 1.5 s quiet → first translated audio, stalls = 6 s
  without output while the preacher spoke, reconnects, dropped frames). A silent minute logs nothing.

## The page (`presenter-ui` `pages/guest`, route `/poslech`, `/poslech/qr`)

- WASM like every UI here (house stack; the AudioWorklet module `guest-player.js` is the one JS file
  the platform requires, copied by Trunk to `/ui-pkg/guest-player.js`). The shell does NOT install
  `ui_usage` on `/poslech*` (public page; the beacon would meet the login wall).
- Phone first, Slovak copy, language names in their own language (core `GUEST_LANGUAGES`); `?lang=`
  keeps the pick. Play = the gesture that creates + resumes the `AudioContext` (iOS unlock), asked
  for 24 kHz (a refusing phone gets a linear resample, `state::guest::resample`). The worklet starts
  at 150 ms queued and drops back to 150 ms when > 1 s queued (latency first). Screen Wake Lock via
  `navigator.wakeLock` (Reflect; not in stable web-sys), re-taken on `visibilitychange`.
- E2E hooks: `body[data-guest-chunks]` (speech frames received), `body[data-guest-played]` (samples
  the worklet played), `[data-role=guest-state][data-state]` (`idle` / `connecting` / `waiting` /
  `live` / `reconnecting` / `stopped` / `refused`).
- `/poslech/qr`: the server makes the QR (`qrcode` crate, SVG) of the address the request came
  through (`X-Forwarded-Host` + `-Proto`); a LAN / loopback address gets a warning. Open it through
  the public host (`https://prsnv.newlevel.media/poslech/qr`) to show it on a screen.

## Reachability (#999 L4)

Guests have no login: `ops/cloudflare/presenter_access.py` `public_paths` per site makes one
everyone-bypass Access app per path — SNV: `/poslech`, `/listen/guest`, `/ui-pkg` (the WASM
assets; the operator's bundle is public code, every API stays behind Access). `/healthz` (version)
is already bypassed. Dry run first, always (`cloudflare-access.md`).

## Tests

- Rust: `listen/translate/test_fake.rs` = the scripted fake Gemini (duplex `Connector`, records
  headers / setups / audio count, answers 100 ms of audio + `w<n> ` + a handle per audio message;
  `Script` = close on setup, goAway after n, close after n). Session tests on the paused clock;
  router tests (`router/listen_guest/tests.rs`) on a real loopback server. Keys `test-key-not-real`.
- E2E `guest-translate.spec.ts` + `fake-gemini.ts`: phone, pick → Play → `waiting` (nothing sent) →
  the real VBAN sender → `live`, chunks + played samples, transcript, original toggle, zero console
  errors. `support.ts` gives every other server a dead Gemini URL and NO key.
- Deploy: the Gemini key rides the SAME "Configure listening transcription" step as Soniox
  (invariant 7's canonical text in `scripts/ci/check_deploy_workflows.py`; both secrets, one file).
