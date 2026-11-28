# Listening providers

The cloud services the sermon-listening feature (#852) sends audio to: which
provider, which model, where its credential lives, what was measured and what is
still unverified. Spec: `docs/superpowers/specs/2026-10-14-sermon-listening-interpreter-assist-design.md`
§14 and §22; plan: `docs/superpowers/plans/2026-11-14-listen-c3-soniox.md`.

## Guest transcript (English) — Soniox real-time

| | |
|---|---|
| Provider | Soniox, real-time WebSocket API |
| Model | `stt-rt-v6` (`PRESENTER_LISTEN_SONIOX_MODEL`; `stt-rt-v5` is removed 2026-12-15) |
| Endpoint | `wss://stt-rt.soniox.com/transcribe-websocket` (`PRESENTER_LISTEN_SONIOX_URL`) |
| Credential | GitHub secret `LISTEN_SONIOX_API_KEY` → deploy step "Configure listening transcription" → 0600 root-only `<etc dir>/listen.env` → `PRESENTER_LISTEN_SONIOX_API_KEY` (`docs/configuration.md`). Sent only as `Authorization: Bearer` on the upgrade request. |
| Code | server `listen/soniox/` (wire, machine, connection, session), `listen/asr.rs` (the `StreamingAsr` trait), `listen/transcription/` (supervisor) |
| What is sent | the guest microphone only (`listen.transcription.guestMic`), 16 kHz mono `pcm_s16le`, only while the switch is on, a session runs and the guest speaks |
| Price | $0.12 per streamed hour; the daily cap (Settings, default 240 min) counts connected time |
| Live gate | none yet — a post-deploy live gate per provider is lane **C7** (spec §14); until then the manual acceptance on dev (lane C3-10) |

### Config sent (the measured one, spec §22.3)

`audio_format: "pcm_s16le"`, `sample_rate: 16000`, `num_channels: 1`,
`language_hints: ["en"]` (since #852 T1: `["sk", "en"]` + `enable_language_identification: true`), `context.general` (domain "Christian church sermon",
topic "preacher reads and cites Bible books, chapters and verses"),
`context.terms` (the 66 English book names from the resolver table, Christian
terms, today's prepared references, today's šepkár source terms). No
translation. Endpoint detection off unless `PRESENTER_LISTEN_SONIOX_ENDPOINT_DELAY_MS`
is set.

### Measured (spec §22.2, 2026-11-14, 1.0× real-time replay on dev2)

10-minute slice of an English-only guest sermon, against three batch references
(their own disagreement 1.9–2.8 % = the noise floor):

| Engine | WER | Word visible p50 / p90 | Bible citations (38 real) | $/h |
|---|---|---|---|---|
| **Soniox stt-rt-v6 + context** | **1.9 %** | 0.76 / 1.55 s | **38 / 38** | 0.12 |
| Soniox stt-rt-v6, no context | 2.1 % | 0.72 / 1.51 s | — | 0.12 |
| AssemblyAI Universal-3.6 Pro (second) | 2.1 % | 0.65 / 1.61 s (finals late) | — | 0.45 |

Response cadence while audio flows: p50 0.19 s, p90 0.41 s, max 1.19 s (the
basis of the 15 s stall rule).

### Behaviour the client relies on

- Soniox does NOT answer WebSocket pings promptly (a pinging client died at
  234 s): the client never sends one; `{"type":"keepalive"}` while no audio
  flows.
- Lazy connect on the first voiced frame; 10 s of silence → `finalize` once,
  then keepalives; 5 min gated → the connection ends.
- 401 / 402 / 403 / 400 stop without retry; more than 20 connections in 10 min
  stop; ≤ 300 min per stream, the client rolls over from 280 min.

### Unverified

- **[unverified]** that a kept-alive stream with no audio is not billed — the
  client bounds it anyway (5 min gated → disconnect, the cap counts connected
  time). Checked in the C3-10 acceptance from Soniox's usage page.
- **[unverified]** that a kept-alive stream resumes with its context after the
  silence gate — checked in C3-10; the fallback is a reconnect with the same
  context.
- **[unverified]** the endpoint-detection setting: off ships; C3-10 measures it on
  (`max_endpoint_delay_ms` 1000–1500) and keeps it only if finals come sooner
  without hurting WER.
- **[unverified]** end-to-end latency on the Presenter path (Dante/VBAN → chain →
  Soniox → `/listen/ws`), target p50 ≤ 0.9 s / p90 ≤ 1.8 s (spec §22.7) — C3-10.
