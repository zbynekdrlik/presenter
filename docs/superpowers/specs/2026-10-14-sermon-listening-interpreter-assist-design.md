# APPROVED by the owner 2026-10-14 (#852) — AMENDED 2026-10-15 by the owner's decision on the evaluation (§13, §19, §20) — AMENDED 2026-11-14 by the measured evaluation (§22: Soniox stt-rt-v6 for the guest EN transcript)

# Sermon listening, verse suggestions and interpreter assist (#850 part C + #852) — Design

**Issues:** #850 part C (Presenter listens to the sermon and suggests Bible verses), #852 (interpreter assist:
live transcript of the English guest, suggested Slovak translation, šepkár tips).
**Date:** 2026-10-14. **Status:** written spec, APPROVED by the owner 2026-10-14 (#852). Built from the 2026-10-14 draft
(`850c-852-design-draft.md`) plus the owner's decisions of 2026-10-14 (#852).
**Presenter baseline:** `dev` @ 9c726944 (v0.4.318). **Songplayer baseline:** `dev` @ dd551b7d (v0.81.0).
**Builds on:** the approved A + B spec `docs/superpowers/specs/2026-10-13-bible-verses-any-device-design.md`
(the resolver §5, phone rules §6, the console reserved sections §7.4, the show log §8).

**Source tags**

- **[SP]** a fact songplayer verified in production or against the live API (path given). Reused, not re-researched.
- **[GAP]** songplayer does not cover it; a small targeted lookup, source URL in §21.
- **[TICKET]** earlier research already on #850 / #852 (cited, not redone).
- **[unverified]** an assumption; each one says how it gets checked.

---

## 1. Goal and success criteria

During a sermon with an English-speaking guest and a Slovak interpreter, Presenter:

- shows the **interpreter** what the guest is saying, word by word, with a suggested Slovak translation, the
  **Slovak Bible text** of any verse the guest cites, and **tips from a šepkár** (prompter) helping with a word;
- offers the **operator** cards for the verses about to be read, from both microphones, one tap to preview.

| # | Criterion | Measured by |
|---|---|---|
| T-S1 | A tip sent on the šepkár page appears on the interpreter page **within 1 s**, newest on top, with sender and time; the interpreter can dismiss it (and undo) | Playwright `interpreter-tips.spec.ts` (two pages) |
| T-S2 | Tip validation and limits hold: length caps, at least a pair or a note, 5 tips per minute per sender, duplicate within 2 min not shown twice | router tests + the spec |
| T-S3 | The šepkár page works on a phone (390×844 touch) and a laptop: no element past the edge, inputs ≥ 16 px, controls ≥ 44 px | Playwright phone profile (A+B helpers) |
| C-S1 | Both VBAN microphones are received on prod for a whole 2 h service. Every frame-counter gap is counted and logged; a stopped sender shows "žiadny zvuk" within 3 s | `/listen/status` before/after + journal; box acceptance (§5.8) |
| C-S2 | Guest speech → English word on the interpreter display: p90 ≤ 1.0 s | token `end_ms` vs display arrival, replay + live (§13) |
| C-S3 | End of an English sentence → its Slovak suggestion: p90 ≤ 2.0 s (hard ceiling 3 s, CAI research [TICKET #852 F1]) | same harness |
| C-S4 | A cited reference (English on the guest mic or Slovak on the interpreter mic) → the Slovak Bible text in the Bible area AND an operator card ≤ 1.5 s after the reference's last word | replay test with timestamps |
| C-S5 | On the replayed guest sermons (§13): ≥ 90 % of the references read aloud become a card (both mics fused), ≤ 2 false cards per hour | bake-off scorer (§13) |
| C-S6 | Verse text is ALWAYS from the database, never machine translation. Nothing goes on the wall without the operator (tap = preview, ZOBRAZIŤ = show) | E2E: a suggestion produces 0 `POST /bible/trigger-slide` until ZOBRAZIŤ |
| C-S7 | Internet / provider outage → banner on the interpreter display + chip on the console within 3 s; the manual console and šepkár tips keep working (they do not depend on the cloud) | E2E with the mock provider killed |
| C-S8 | No microphone configured = no audio subsystem runs and nothing is billed (PP). A daily cap stops streaming | unit test + `/listen/status` |
| C-S9 | Both interpreter layouts: tablet (scroll/pause, longer history, dismiss controls) and TV at 2–3 m (big type, last 2–3 sentences, no controls); no overflow at their target sizes | Playwright tablet 1180×820 touch + TV 1920×1080 |
| C-S10 | Zero browser console errors/warnings in every new spec | each spec's final assertion |

## 2. Binding owner decisions

From #850 / #852 (ROZHODNUTÉ):

- **Audio = two separate microphones over VBAN**, quality first (#850, 2026-10-13).
- **Guest speaks ENGLISH on one mic, interpreter speaks SLOVAK on the other; per-microphone language config**
  (#850, 2026-10-13).
- **The same guest transcript feeds the interpreter assist; best existing models for transcription and
  translation** (#852). Cloud processing is therefore approved (the N100 prod box cannot run them locally).
- **Reuse songplayer first**; vbanvst and audiomatrix are dead experiments, never a source (2026-10-13).
- **`/ui/interpreter` gets BOTH layouts — stage TV at 2–3 m and desk/booth tablet — the TABLET layout first**
  (#852, 2026-10-14: "3 - no prioritne skusime tablet").
- **Šepkár tips** (#852, 2026-10-14: "nech je moznost posielat sepkarske tipy, cize niekto na
  notebooku/tel/tablete si otvori sepkarsku stranku a bude vediet poslat radu ako prelozit niektore slovicko"):
  a separate page on any device, behind the same Cloudflare Access (LAN direct), sends the interpreter a short tip;
  tips show in their own area, newest on top, with sender and time, dismissable.
- **The AI/LLM model is always the owner's choice** (project memory; prod today `google/gemini-3.8-flash`).
- From A + B: the resolver in `presenter-core` is shared by every entry point; console cards only preview;
  Cloudflare Access protects both public hosts (720 h sessions, church-IP bypass).

## 3. What is reused from songplayer

Songplayer has **no VBAN receiver** (only the sender's `UdpSocket::bind(("0.0.0.0", 0))`, `vban_out.rs:888`)
and **no real-time ASR** (its transcription is batch; its only live AI session is Gemini Live Translate on a
file). Those two parts are new, designed from what songplayer verified. The šepkár tips have no songplayer
counterpart (plain Presenter CRUD + live event).

| Songplayer source | What it gives | Reused as |
|---|---|---|
| `crates/sp-server/src/playback/vban_packet.rs` | 28-byte LE header, `VBAN_MAGIC`, `VBAN_STREAM_NAME_LEN`, `VBAN_DATA_MAX` = 1436, `sr_index_of` (48k=3, 96k=4, 192k=5, 44.1k=16, 88.2k=17), format bits INT16 0x01 / INT24 0x02 / FLOAT32 0x04, `f32_to_int24` / `int24_le` / `f32_to_int16` / `clean_f32`, `stream_name_bytes`, `VbanEncoder` (counter +1 per packet, wraps at `u32::MAX`) | **CODE.** Ported into `presenter-core/src/audio/vban.rs` with the DECODE direction added; the encoder ported **test-only** as the packet generator |
| `vban_packet_tests.rs` (`parse_packet`, `ramp_block`), `vban_packet_tests_format.rs` (15 rate × format rows) | an independent decoder used as a test oracle; geometry pins | **CODE (tests).** `parse_packet`'s sign extension becomes production decode (without its `assert!`s); the 15 rows become round-trip cases |
| `vban_rate.rs` (rubato `=5.0.1`, `Fft` + `FixedSync`) | a converter whose output length is exact per block | **CODE pattern** for 48 → 16 kHz |
| `.claude/rules/vban-out.md` | spec rev. 13 verified; port 6980; "the RECEIVE thread never waits"; Voicemeeter receivers keep ≥ 6 × 256 samples and senders burst 2–4 packets; box acceptance by tcpdump; FOH = VB-Matrix on fohabl over VBAN-TEXT, switch with `Mute` only, read back every point | **KNOWLEDGE** (§5, §20) |
| `vban_stall.rs`, `stat_window.rs` | rolling worst + rate-limited WARN | **KNOWLEDGE** (Presenter keeps its #484 log gate) |
| `.claude/rules/dabing.md` round H + `scripts/dub_live_session.py` (`ContinuousSession`, `live_config`) | the production Live Translate session: `gemini-3.5-live-translate-preview`; 16 kHz mono s16le 100 ms frames in, 24 kHz PCM out; `translation_config{target_language_code: sk, echo_target_language: False}`; input + output transcription; compression (trigger 25 000 / target 8 000) + resumption; GoAway = open the next connection with the ACTIVE handle while still feeding the old one; ≤ 50 connections; 30 s timeouts; `google-genai==2.24.0` traps | **CODE in the bake-off** (unchanged, §13); **KNOWLEDGE** for a Rust port only if Live Translate wins (§8.6) |
| `eval/dubbing/live_translate_continuous_probe.py` | replays a file at 1.0× through Live Translate; writes `input_text.txt`, `output_text.txt`, `events.jsonl` with arrival times, `summary.json` | **CODE (bake-off, unchanged)** |
| `crates/sp-server/src/dabing/subtitles.rs` (`group_fragments`, `ends_sentence`, 20-word cap, "a pause never closes a line", `split_sentences_inside`, `assign_en_sentences`) | sentence assembly + EN ↔ SK pairing by content-interval overlap; measured on a 36-min talk (95 % of SK lines end a sentence, EN coverage 0.93 vs 83 % / 0.88 with a 1.5 s pause rule) | **CODE (port of the pure rules)** (§6.5, §8.6) |
| `.claude/rules/dubbing-eval.md` | Live Translate ~$0.037/min (in $0.0053 + out $0.0315); `gemini-3.8-live` + translate instruction stops after ~1.4 s → only the dedicated model works; −50 dBFS speech line; Soniox needs a non-default `User-Agent` (Cloudflare 1010) | **KNOWLEDGE** |
| `.claude/rules/lyrics-eval-backends.md` | live-verified shapes: **Soniox** `stt-async-v5` (Bearer, `translation:{type:"one_way", target_language:"sk"}`, translated tokens without timestamps, `translation_status`, sub-word tokens joined on leading spaces); **AssemblyAI** `universal-3-5-pro`; **Gemini 3.5 Transcribe** (`gemini-3.5-transcribe`, `mode` mandatory, 100 ms word grid, ≈ $0.005/min, 30 min per file); scoring discipline | **KNOWLEDGE**; **CODE**: `eval/lyrics/backends/gemini_3_5_transcribe.py` drafts the bake-off reference transcripts |
| `.claude/rules/paid-ai.md` | one paid-AI switch per node, read where the paid call becomes certain; held, never failed | **KNOWLEDGE** (§6.4) |
| `.claude/rules/ai-proxy.md` | refusals are a prompt problem; retry 5 / 20 / 60 s honouring `Retry-After` | **KNOWLEDGE** |
| `docs/ai-providers.md` | provider / model / credential / live-gate table; songplayer's Soniox, AssemblyAI, ElevenLabs keys are slated for revocation | **KNOWLEDGE**; Presenter keeps the same table and gets its **own** keys |

**Not reused:** sender-side scheduling (MMCSS, pacing, `SlewRemainder`, send latency L) — a receiver has no
playout clock.

## 4. Architecture

Everything runs inside `presenter-server` on the prod box (10.77.9.205, Intel N100, iGPU only). Transcription and
translation are cloud calls [TICKET #850 F4]. Šepkár tips are local (no cloud, no audio).

```
FOH PC (VB-Matrix / Voicemeeter)                      any laptop / phone / tablet
  ├─ VBAN "GUEST_EN"  ──┐ UDP 6980                      /ui/sepkar ── POST /interpreter/tips
  └─ VBAN "INTERP_SK" ──┤                                                  │
                        ▼                                                  ▼
presenter-server                                         interpreter_tips table ── LiveEvent::InterpreterTips
  listen/vban_rx → mic[guest], mic[interp] (48→16 kHz, level, recorder)              │ (main live hub)
        │                │                                                           │
   asr session      asr session       one session per microphone (Soniox first)      │
   (en + EN→SK)     (sk)              the guest session = THE shared English transcript
        │                │                                                           │
   transcript ◀─────────┘  sentence assembly (songplayer rules)                       │
        ├─▶ detect (EN+SK spoken refs → resolver) ─▶ fuse ─▶ LiveEvent::BibleSuggestions ─▶ console "Návrhy AI"
        │                                         └─▶ Bible area (SK text from the DB)
        ├─▶ translate (SK suggestion per finished EN sentence; engine per bake-off)
        └─▶ listen hub ─▶ GET /listen/ws ─────────────────────────▶ /ui/interpreter ◀─────┘
                                                               (tablet layout first, then TV)
```

- **One shared English transcription.** The guest-mic session is the only transcription of that microphone; its
  events feed the interpreter display, the detector and the translation layer. No consumer starts its own
  transcription (songplayer D3 made the same call [SP `dabing.md`]).
- **Transports.** Transcript traffic (several partials per second) goes over a dedicated `/listen/ws`, so it never
  floods stage displays and operators. Low-rate state (suggestions, tips) goes through the main live hub as
  `LiveEvent`s that each carry the FULL current list, so the operator WS's latest-value coalescing (A+B §4) can
  never lose an item.

## 5. Audio ingest — the VBAN receiver

### 5.1 Where and how it listens

- One UDP socket, `0.0.0.0:6980` (VBAN default [SP]), configurable, in a tokio task; bound only when at least one
  mic is configured.
- The receive loop never waits on anything downstream (spec p. 11). Decoded blocks go into a bounded per-mic
  channel; on overflow the OLDEST block is dropped and counted (songplayer's queue rule [SP]).
- CPU: 2 streams × ~190 packets/s + two converters — negligible on the N100.

### 5.2 Decode (`presenter-core/src/audio/vban.rs`, pure, host-tested)

Ported from songplayer's encoder, decode added. Checks from the spec's reference receive algorithm
[GAP: spec rev. 13 p. 10–11]:

1. Magic `VBAN`, length ≥ 28.
2. Sub-protocol `format_SR & 0xE0` must be AUDIO (0x00). SERIAL 0x20 / TXT 0x40 / SERVICE 0x60 / USER 0xE0 →
   counted as `ignored_non_audio` (VB-Matrix also speaks VBAN-TEXT).
3. Codec `format_bit & 0xF0` must be PCM (0x00); VBCA / VBCV / USER → `unsupported_codec`.
4. Reserved bit `format_bit & 0x08` must be 0, else `corrupt`.
5. SR index `format_SR & 0x1F` < 21, from the full list `{6000, 12000, 24000, 48000, 96000, 192000, 384000, 8000,
   16000, 32000, 64000, 128000, 256000, 512000, 11025, 22050, 44100, 88200, 176400, 352800, 705600}` (songplayer
   maps 5 of them, same indices) [GAP].
6. Data type `format_bit & 0x07`: INT16 (2 B), INT24 (3 B), INT32 (4 B), FLOAT32 (4 B) accepted; BYTE8, FLOAT64,
   12/10-bit → `unsupported_type`.
7. `(nbs + 1) × (nbc + 1) × sample_size == packet_len − 28`, else `corrupt`.
8. Output `VbanPacket { stream_name, counter, rate_hz, channels, samples: Vec<f32> }` (interleaved, ±1.0).

Every reject is a typed error counted per sender; nothing panics on network data.

### 5.3 Stream selection, ordering, gaps

- A mic is configured by **stream name** (exact, ≤ 16 ASCII) and optionally the **sender IP**. Unmatched streams
  are listed in `/listen/status` as "seen, not used" (helps the first setup).
- Counter = expected → push. Ahead by packets totalling ≤ 1 s → insert silence, count `lost_packets`. Behind
  (late/duplicate) → drop, count `late_packets`. A jump > 1 s either way (sender restart) → reset, count `resyncs`.
- A rate/channel/format change → reset the converter, log INFO.
- No jitter buffer: ASR has no playout clock; Voicemeeter's bursts of 2–4 packets are normal [SP].

### 5.4 Per-mic chain

- **Channel:** `left | right | mix`.
- **Resample** → **16 kHz mono s16le** (accepted by Soniox and Gemini Live [SP, GAP]); rubato `Fft` + `FixedSync`
  (songplayer pattern); 100 ms frames out.
- **Level:** RMS dBFS per 100 ms; voiced above −50 dBFS (songplayer's `VOICED_DBFS` [SP]).
- **Health:** packets/s, `lost_packets`, `late_packets`, `resyncs`, last-packet age, level, current format.

### 5.5 Recorder (debugging)

- Off by default. A Settings toggle writes each mic's 16 kHz mono stream to
  `/opt/presenter/recordings/<date>/<mic>.wav` (exactly what the ASR gets). ~115 MB per mic-hour. Deleted by the
  server after 30 days, or right after the bake-off report.
- Not an evaluation input (§13); used for debugging a real service.

### 5.6 Configuration

`app_settings` key `listen.mics` (JSON list), edited in Settings → "Počúvanie":

```json
[{ "id": "guest", "name": "Hosť", "streamName": "GUEST_EN", "senderIp": null, "channel": "left",
   "language": "en", "role": "guest" },
 { "id": "interp", "name": "Tlmočník", "streamName": "INTERP_SK", "senderIp": null, "channel": "left",
   "language": "sk", "role": "interpreter" }]
```

- `language`: `en | sk | off` (owner: per-mic). `role`: `guest | interpreter | preacher`. A Slovak service without
  a guest = one mic `sk`/`preacher`, the other `off`.
- An empty list = audio subsystem idle, no socket bound (PP).

### 5.7 Sender side (owner action, §20)

- VB-Matrix on fohabl (the FOH PC songplayer already sends to) or Voicemeeter sends two streams to
  10.77.9.205:6980, 48 kHz, INT16, mono or stereo, names `GUEST_EN` / `INTERP_SK`.
- Tap point **after the channel mute, before the fader** if the mixer allows it — a muted mic sends silence
  (privacy), fader rides do not change what the ASR hears.
- If Claude does the VB-Matrix routing: owner's go first, `Mute` only, record prior values, read back every point
  [SP `vban-out.md`].

### 5.8 Box acceptance

songplayer's method [SP]: a 10-min capture on the prod box (`tcpdump udp port 6980`): 0 counter gaps, inter-arrival
p99 < 15 ms; `/listen/status` read before and after (counters diffed); level moves when someone speaks into each
mic, silence when muted.

## 6. Transcription — one session per microphone

### 6.1 Provider trait

```rust
// presenter-server/src/listen/asr/mod.rs
#[async_trait]
pub trait StreamingAsr: Send + Sync {
    async fn open(&self, cfg: &AsrSessionConfig) -> anyhow::Result<Box<dyn AsrStream>>;
}
#[async_trait]
pub trait AsrStream: Send {
    async fn send_frame(&mut self, pcm_16k_s16le: &[u8]) -> anyhow::Result<()>; // 100 ms
    async fn next_event(&mut self) -> Option<AsrEvent>;
}
pub enum AsrEvent { Tokens { tokens: Vec<AsrToken> }, Finished, Error { kind: AsrErrorKind, message: String } }
pub struct AsrToken { pub text: String, pub start_ms: Option<u32>, pub end_ms: Option<u32>,
    pub is_final: bool, pub confidence: f32, pub language: Option<String>, pub is_translation: bool }
```

Implementations: `live_translate.rs` (first — songplayer's production engine, §6.2), `mock.rs` (replays a recorded
JSONL, behind the existing `mock-integrations` feature, used by E2E), `soniox.rs` only if the replay evaluation (§13)
measures that a second engine is needed.

### 6.2 The first implementation is Gemini Live Translate (amended 2026-10-15, owner decision)

> **SUPERSEDED 2026-11-14 by §22.3** — the measured evaluation replaced Gemini Live Translate with **Soniox
> stt-rt-v6** for the guest's English transcript. §6.2 and §6.3 are kept below as history.

songplayer already selected and runs it in production (dubbing, round H), so it is reused, not re-evaluated:

- `gemini-3.5-live-translate-preview`, ONE continuous session per mic: 16 kHz mono s16le 100 ms frames in,
  `translation_config{target_language_code: "sk", echo_target_language: false}`, **input transcription (EN) + output
  transcription (SK)** — both consumers (verse suggestions, interpreter display) get their text from one session;
  `context_window_compression` (trigger 25 000 / sliding target 8 000 tokens) + `session_resumption`; on GoAway the
  next connection opens with the ACTIVE connection's latest handle while the old one still drains (overlap, never a
  pause) — songplayer `dabing.md` round H, `scripts/dub_live_session.py::ContinuousSession`.
- Measured in songplayer: first voiced SK output ≈ 3.1 s, EN input transcription ≈ 4–5 s after the speech onset,
  0 reconnect failures over 25 min, ≈ $0.037/min.
- Audio output is not needed by Presenter (text only); whether the model can run with text-only output, or the audio
  is received and dropped, is checked in C3 against the real API (songplayer: output modality AUDIO).
- The replay evaluation (§13) measures the EN latency on our guest sermons; Soniox real-time (§6.3) is added as a
  second engine only if that latency is unusable for the interpreter.

### 6.3 Soniox session config — fallback engine only (§6.2) [GAP: Soniox real-time docs]

- `wss://stt-rt.soniox.com/transcribe-websocket`, key in the connection's `Authorization` header (`api_key` in the
  config is deprecated), `User-Agent: presenter-listen/<version>` [SP: Cloudflare 1010].
- First text frame:

```json
{ "model": "stt-rt-v5", "audio_format": "pcm_s16le", "sample_rate": 16000, "num_channels": 1,
  "language_hints": ["en"], "enable_language_identification": true, "enable_endpoint_detection": true,
  "context": "<Bible book names EN/SK, the guest's name, today's prepared references (#851), today's šepkár pairs>",
  "translation": { "type": "one_way", "target_language": "sk" } }
```

  Interpreter mic: `language_hints: ["sk"]`, no `translation`.
- Audio: binary frames of 100 ms. End: an empty TEXT frame (an empty binary frame is not an end).
- Keepalive at least every 40 s while no audio flows (silence gate). Max 300 min per stream; a
  `max_duration_reached` (413) close → reconnect.
- Responses: `tokens[]` with `text`, `start_ms`/`end_ms` (absent on translation tokens), `confidence`,
  `is_final`, `language`, `translation_status` (`original`/`none` spoken, `translation` translated). Final tokens
  come once; non-final tokens are replaced by each response. Errors: branch on `error_type`; codes 400/401/402/403/
  408/413/429/500/503; the server closes right after an error.
- Sub-word tokens are joined on leading spaces [SP].

### 6.4 Session lifecycle (`listen/session.rs`)

- **Start** when the stage layout switches to `preach` (Kázeň) — it brackets the sermon in the prod logs
  [TICKET #850 F2] — or by the console toggle. **Stop** 60 s after leaving `preach`, or by the toggle (the toggle
  overrides the automation for the rest of the day).
- **Paid gate:** the start reads one switch (`listen.enabled` + a configured mic + a key), where the paid call
  becomes certain [SP `paid-ai.md`]. While off, the status says why ("vypnuté", "chýba kľúč", "žiadny mikrofón").
- **Silence gate:** after 10 s below −50 dBFS stop sending audio, send keepalives; the first voiced frame resumes
  with a 1 s pre-roll. **[unverified]** that a kept-alive Soniox stream resumes with its context — checked in C3
  against the real API; fallback = reconnect on resume.
- **Reconnect** on close / 1001 / 408 / 413 / 429 / 5xx after 1, 5, 20, 60 s, then every 60 s, honouring
  `Retry-After` [SP `ai-proxy.md`]; the last ≤ 5 s of audio is resent. 401/402/403 → stop, status
  `error{auth|payment}`, no retry loop.
- **Daily cap** 4 h of streamed audio per mic (configurable); **cost meter** (seconds streamed per mic per day) in
  `/listen/status` and one INFO summary per session.

### 6.5 Transcript assembly (`presenter-core/src/listen/transcript.rs`, pure)

- Final tokens build **sentences** with songplayer's rules [SP `subtitles.rs`]: close at `. ! ? …` (also inside a
  token), at 20 words, or at a Soniox endpoint; **a pause never closes a sentence**.
- Non-final tokens are the "live tail" of the current sentence.
- Each sentence keeps `start_ms`/`end_ms`, its token language, and its translation tokens (Soniox emits them right
  after the spoken tokens they translate [GAP]).

## 7. Consumer 1 — verse suggestions for the operator

### 7.1 Spoken-reference detection (`presenter-core/src/listen/detect/`, pure)

The A resolver already knows Slovak, Czech and **English** book names (T2, BPRP `eng`) and validates against a
versification. The detector turns spoken text into something the resolver reads:

- **Number normalisers** chosen per token language (not per mic config, so a wrong-language mic still detects):
  - English: "three sixteen", "chapter three verse sixteen", "verses sixteen through eighteen", "first Corinthians"
    → `1 Corinthians`, "Psalm one hundred nineteen".
  - Slovak: ordinals 1–176 in all cases and genders, pre-generated ("tretia kapitola, šestnásty verš", "v prvom
    liste Korinťanom, v trinástej kapitole"), "až" ranges.
  - ASR-formatted digits ("John 3:16", "Ján 3,16") pass straight through.
- **Triggers:** a book name followed by numbers within ~8 words; "turn with me to …", "the book of …", "otvorte si
  …", "čítame z …".
- **State per mic** (2 min): last book + chapter, for "verse five" / "piaty verš" / "v tej istej kapitole" /
  "keep reading" / "čítame ďalej" (lower confidence).
- **Resolver:** guest references against the **eng-kjv structure** (his numbering); interpreter references against
  the **main (Slovak) translation**; impossible candidates drop out.

### 7.2 Fusion of the two microphones (`fuse.rs`)

- Same book + chapter + overlapping verses from both mics within 20 s → ONE card, confidence high, tagged "EN + SK"
  (the interpreter repeating the guest's reference is the strongest signal).
- One mic → medium; a relative reference → low. Cards expire after 3 min; a passage already on the wall or in
  "Táto bohoslužba" is marked, not repeated.
- Prepared references from #851 rank first and seed the ASR `context`.

### 7.3 Versification between English and Slovak

No mapping table exists in the code (A+B §4); the guest cites KJV-style numbers, SEB may differ (Mal 4:5 KJV =
SEB 3:23; 154 chapters differ in verse count).

- v1: fetch the Slovak text by the SAME code/chapter/verse. When the chapter's verse count differs between eng-kjv
  and the Slovak translation, or the chapter does not exist there, show "číslovanie sa môže líšiť" and the English
  (KJV) text next to it.
- A real mapping table is a later improvement (risk §17).

### 7.4 Into the console (A+B §7.4 contract)

- `LiveEvent::BibleSuggestions { items: Vec<SuggestionDto> }`, the FULL current list. The new variant needs a
  no-op arm in `companion/variables.rs` (`live-events.md`).
- Mapped to `ConsoleCandidate { origin: Suggested { confidence }, label, book_code, chapter, verse_start,
  verse_end, detail: "počul som: „…turn with me to John three sixteen…“ (hosť)" }`.
- Section "Návrhy AI" (hidden while empty). A tap **previews** in the console's main translation; only ZOBRAZIŤ
  sends to the wall. Auto-show stays OFF.
- A listening chip next to the on-air strip ("Počúvam ● hosť EN ● tlmočník SK"); tap → on/off sheet with
  per-mic health.

## 8. Consumer 2 — the interpreter display `/ui/interpreter`

### 8.1 Page

- New top-level page in the WASM app (route next to `/ui/camera`, `router.rs:169`). It is NOT an operator stage
  layout: like `camera-crew` (internal-only, `stage_display.rs`) it does not follow the global layout, so the
  preacher's `preach` timer keeps working.
- **Two layouts, one component tree:** `?layout=tablet` (default, built FIRST) and `?layout=tv` (the Android stage
  launcher URL or a TV browser). The parameter is explicit so a TV never guesses.
- Data: `/listen/ws` (snapshot of the session so far + events) and the main live hub (`BibleSlide` = what is on the
  wall, `InterpreterTips`). Reconnect + visibility resync as in A+B §6.7, so a woken tablet catches up.
- Areas render only when they have data: before audio exists (units S1a/S1b ship first, §16) the tablet shows the
  tips area full-width.

### 8.2 Tablet layout (desk or booth, built first)

Targets: landscape 1180×820 / 1280×800 and portrait 820×1180, touch.

```
┌────────────────────────────────────────────────────────────┬──────────────────────────────┐
│ PREPIS  [⏸ Pauza]                                    ▲     │ TIPY OD ŠEPKÁRA              │
│  But God demonstrates his own love for us …        dim     │ ┌──────────────────────────┐ │
│   ↳ Ale Boh dokazuje svoju lásku k nám …   strojový  dim   │ │ atonement → zmierenie  ✕ │ │
│  Turn with me to Romans eight, verse thirty-seven, │       │ │ Peter · 19:42            │ │
│   ↳ Biblia: Rimanom 8,37                                   │ └──────────────────────────┘ │
│  and let's read it together ▌                  live tail   │ ┌──────────────────────────┐ │
│                                  [▼ Naživo · 3 nové]       │ │ „sanctification“ =       │ │
│                                                            │ │ posvätenie  Jana · 19:38✕│ │
│                                                            ├──────────────────────────────┤
│                                                            │ BIBLIA  Rimanom 8,37 (SEB)   │
│                                                            │ Ale v tomto všetkom slávne   │
│                                                            │ víťazíme skrze toho, ktorý   │
│                                                            │ si nás zamiloval.            │
│                                                            │ ČÍSLA A MENÁ  8 · 37 · Paul  │
├────────────────────────────────────────────────────────────┴──────────────────────────────┤
│ ● hosť EN −18 dB  ● prepis OK 0,6 s          [EN] [EN+SK] [SK]   A− A+            19:42  │
└───────────────────────────────────────────────────────────────────────────────────────────┘
```

- **Transcript (left, ~62 %):** each EN sentence with its SK suggestion directly under it (interleaved pairs read
  better on a tablet than two separate zones); newest at the bottom; final words normal, the live tail lighter;
  older sentences dimmed. A cited verse's reading rows show "↳ Biblia: <reference>" instead of machine translation.
- **History:** the whole session since listening started; the newest 300 sentences stay in the DOM.
- **Scroll and pause (owner):** auto-follow the newest line; scrolling up pauses auto-follow and shows
  "▼ Naživo · N nové"; the ⏸ Pauza button freezes the view while events keep arriving; tap ▶ or "Naživo" to jump
  back. Pausing never stops listening.
- **Right column (~38 %):** Tips (top, §9.6), Bible area (below, §8.4), numbers and names (bottom).
- Portrait: tips as a collapsible strip on top, transcript in the middle, Bible area as a bottom panel.
- **Type:** EN 24 px, SK 22 px, tips 22 px, Bible 22–28 px auto-fit; A−/A+ scales all (per device, localStorage
  in try/catch). EN / EN+SK / SK toggle per device.
- Every control ≥ 44 px; inputs none (the interpreter only reads and dismisses).

### 8.3 TV layout (stage, read from 2–3 m)

Target 1920×1080 (and 3840×2160 scaled), no touch, **no controls**.

```
┌──────────────────────────────────────────────────────┬────────────────────────────┐
│ TIP  atonement → zmierenie            (Peter 19:42)  │ BIBLIA  Rimanom 8,37 (SEB) │
├──────────────────────────────────────────────────────┤ Ale v tomto všetkom slávne │
│ …that is why Paul says we are more than        dim   │ víťazíme skrze toho,       │
│ conquerors. Turn with me to Romans eight, verse      │ ktorý si nás zamiloval.    │
│ thirty-seven ▌                                       │                            │
│ ↳ …preto Pavol hovorí, že sme viac než víťazi.       │                            │
└──────────────────────────────────────────────────────┴────────────────────────────┘
```

- Last 2–3 sentences only (EN ~52 px, SK ~44 px), the Bible area ~40 px auto-fit, the newest 2 active tips in a
  strip at the top. No scroll, no buttons. Tips leave the TV when dismissed on a tablet or after 5 min
  (TV-side filter; the server state is shared).
- Same status dot + clock, small, bottom right (the stage StatusBar look).

### 8.4 Bible area

- Reference (large) + the Slovak verse text **from the database**, auto-fit, up to the cited range.
- **Translation = the same as the wall's main translation** (decided default §18).
- Source priority: the verse **on the wall** (`BibleSlide`, operator-confirmed) > the latest detected reference.
  It stays until replaced or 5 min pass. Numbering warning per §7.3.

### 8.5 Latency budget

| Step | Budget | Basis |
|---|---|---|
| VBAN packetising + LAN | ≤ 30 ms | ≤ 256 samples/packet, bursts of 2–4 [SP] |
| 100 ms framing + resampler delay | ≤ 120 ms | |
| Cloud ASR partial | ≤ 500 ms | measured in the bake-off |
| Fan-out + WS + render | ≤ 50 ms | |
| **Speech → EN word** | **≤ 1.0 s p90** | C-S2 |
| Sentence end → final (endpoint detection) | ≤ 500 ms | Soniox finalises at once on an endpoint [GAP] |
| Translation | 0–1.5 s | built-in arrives with the finals; LLM measured |
| **Sentence end → SK suggestion** | **≤ 2.0 s p90** (ceiling 3 s) | CAI: ≤ 2 s target, ≤ 3 s tolerable, > 4–5 s hurts [TICKET #852 F1] |
| **Reference → Bible area / card** | **≤ 1.5 s** | final + resolver < 1 ms + DB ~1 ms |
| **Šepkár tip → interpreter** | **≤ 1 s** (typically < 200 ms) | POST + one live event, no cloud |

songplayer's Live Translate figures [SP `dabing.md`]: first SK output ~3.1 s after the onset; SK placement latency
clamped 1–6 s; the **English input transcription arrives per phrase ~4–5 s late** — so Live Translate never feeds
the EN column.

### 8.6 The Slovak suggestion — three candidates, decided by the bake-off

| Engine | How | For | Against |
|---|---|---|---|
| **A. Soniox built-in translation** (default at C3) | translation tokens in the guest-mic stream, paired to EN sentences by order | no extra session, latency or cost | no glossary/instructions documented; quality unknown on sermons |
| **B. LLM per finished sentence** | `ai/client.rs` `call_chat_completions_with_options` (OpenRouter): previous 3–5 EN sentences + the SK already shown + a glossary (curated + today's šepkár pairs) + a neutral technical instruction [SP: refusals are a prompt problem]; at most one revision per sentence | glossary, context, owner's model | +0.5–1.5 s; token cost |
| **C. Gemini Live Translate** (songplayer's engine) | second session on the guest mic, its `output_audio_transcription` = SK text, paired by content-interval overlap (`assign_en_sentences`) [SP] | proven on 36-min talks, interpretation-style output | preview; no instructions/glossary [GAP]; audio output paid though unused ($0.0315/min [SP]); ~3 s behind; text-only output undocumented [GAP] |

- A detected verse always replaces machine translation with the database text (§8.4).
- If C wins: port `ContinuousSession`'s connection logic to Rust (GoAway overlap, handle from the active connection
  only, compression 25 000/8 000, 30 s timeouts, ≤ 50 connections) with songplayer's fake-server scenarios as tests;
  the file-specific parts (pacing anchor, drain to the end, placement) drop out. Decided in C6.

## 9. Šepkár tips

A person who knows the topic (a pastor, a bilingual member) helps the interpreter live: "atonement → zmierenie".

### 9.1 Who and where

- Page **`/ui/sepkar`**, opened on any laptop, phone or tablet. Through the public host it sits behind the same
  Cloudflare Access app as the rest of Presenter (church-IP bypass, 720 h sessions); on the LAN it opens directly,
  like every Presenter page. No separate login (same trust as the operator).
- Independent of audio and ASR: it works with listening off, on any service, and ships before the audio units.

### 9.2 Data model

New table `interpreter_tips` (incremental migration `m20261014_000001_create_interpreter_tips.rs`, repository
`presenter-persistence/src/repository/interpreter_tips.rs`):

| Column | Type | Rule |
|---|---|---|
| `id` | INTEGER PK autoincrement | |
| `service_date` | TEXT `YYYY-MM-DD` | the server's local date at creation (the same "service = day" notion as A+B's "Táto bohoslužba") |
| `source_term` | TEXT NULL | the word/phrase as heard, 1–60 chars |
| `target_term` | TEXT NULL | its suggested translation, 1–60 chars; both terms or neither |
| `note` | TEXT NULL | free tip, 1–200 chars |
| `sender_name` | TEXT NOT NULL | 1–40 chars, typed on the page, remembered per device |
| `sender_ip` | TEXT NOT NULL | client IP by the request-log rule (`router/request_log.rs::request_fields`: X-Forwarded-For only from a loopback peer) — audit and rate limit, never shown |
| `user_agent` | TEXT NULL | audit only, ≤ 200 chars |
| `created_at` | TIMESTAMP (UTC) NOT NULL | |
| `dismissed_at` | TIMESTAMP NULL | set by the interpreter; cleared by undo |

- At least one of (pair, note). All text is trimmed, control and invisible format characters (zero-width, bidi overrides) replaced by spaces, rendered as plain text (no HTML,
  no markdown — Leptos escapes).
- Site-local: excluded from the SNV ↔ PP library sync.

### 9.3 Which tips are "active"

`dismissed_at IS NULL AND service_date = today AND created_at > now − 4 h`. The 4 h window separates the morning
and evening services of one day; the date check empties the list at midnight. The interpreter can also clear all
at once ("Skryť všetky", with a confirm).

### 9.4 API

| Method + path | Body / query | Answer |
|---|---|---|
| `GET /interpreter/tips` | `?scope=active` (default) or `?scope=today` | 200 `{ serviceDate, tips: [TipDto] }`, newest first; `today` includes dismissed tips with `dismissedAt` (for the šepkár page) |
| `POST /interpreter/tips` | `{ senderName, sourceTerm?, targetTerm?, note? }` | 201 `TipDto`; 200 `TipDto` + `duplicate: true` when the same pair (with its note) or the same note alone was sent in the last 2 min (any sender; a resend with a changed note is a new tip); 422 with a Slovak `error` per rule; 429 with `Retry-After` + "Pomaly — najviac 5 tipov za minútu" |
| `POST /interpreter/tips/{id}/dismiss` | — | 200 `TipDto` (idempotent); 404 unknown id |
| `POST /interpreter/tips/{id}/restore` | — | 200 (the interpreter's 5 s undo); 404 unknown; 409 when it is no longer today's |
| `POST /interpreter/tips/dismiss-all` | — | 200 `{ dismissed: n }` |

```json
{ "id": 17, "serviceDate": "2026-10-18", "sourceTerm": "atonement", "targetTerm": "zmierenie",
  "note": null, "senderName": "Peter", "createdAt": "2026-10-18T17:42:05Z", "dismissedAt": null }
```

- Router file `router/interpreter_tips.rs` (new), state in `state/interpreter_tips.rs` (new); nothing grows
  `router.rs` beyond the route lines.
- Errors are typed (`RepositoryError` → 404/409/422, `repository-error-pattern.md`).

### 9.5 Live event

`LiveEvent::InterpreterTips { service_date: String, tips: Vec<TipDto> }` = the FULL active list, newest first,
capped at 20. Published after every create, dismiss, restore, dismiss-all, and at the date/4 h boundary (a
1-minute tick only when the list is non-empty). Full-list semantics make the main hub's latest-value coalescing
safe. The new variant gets a no-op arm in `companion/variables.rs` (`live-events.md`). Reconnect resync = `GET
/interpreter/tips` (the A+B §6.7 resync list gets this entry for the interpreter page).

### 9.6 The tips area on the interpreter display

- **Tablet:** right column, top. Cards newest on top: the pair large ("atonement → zmierenie"), the note smaller,
  "Peter · 19:42" below. A new card is highlighted for 2 s (no sound). ✕ (≥ 44 px) or a swipe left dismisses;
  a toast "Skryté — Späť" offers undo for 5 s. The newest 6 are visible, older active ones scroll. "Skryť všetky"
  at the area's header.
- **TV:** the newest 2 active tips in a top strip, no controls (§8.3).
- Before listening exists, the tips area is the whole tablet page.

### 9.7 The šepkár page `/ui/sepkar`

Phone-first, the same on a laptop (wider form, list to the right).

```
┌──────────────────────────────┐
│ Šepkár pre tlmočníka      ●  │
│ Tvoje meno  [Peter        ]  │
│ Slovo (EN)  [atonement    ]  │
│ Preklad     [zmierenie    ]  │
│ Poznámka    [             ]  │
│             [   0 / 200   ]  │
│ [   POSLAŤ TLMOČNÍKOVI   ]   │
│ ── Dnes ───────────────────  │
│ atonement → zmierenie        │
│ Peter · 19:42 · na obrazovke │
│ „sanctification“ = posvätenie│
│ Jana · 19:38 · skryté        │
└──────────────────────────────┘
```

- Fields: "Tvoje meno" (required once, kept in localStorage in try/catch), "Slovo" → "Preklad" (optional pair),
  "Poznámka" (optional, counter). Enter in "Preklad" or "Poznámka" sends; the button is ≥ 48 px.
- After a send: toast "Odoslané 19:42", the three tip fields clear, focus returns to "Slovo". A duplicate shows
  "Tento tip už tlmočník má".
- "Dnes" list: today's tips newest first with state "na obrazovke" / "skryté"; it refetches `GET
  /interpreter/tips?scope=today` on each `InterpreterTips` event.
- Errors inline (422 text from the server), never silent; 429 shows the wait time.
- Phone rules from A+B B1: inputs ≥ 16 px, `enterkeyhint`, no element past the edge, controls ≥ 44 px. If this unit
  ships before B1b, the page's own CSS applies the 16 px rule.

### 9.8 Limits

| Limit | Value | Why |
|---|---|---|
| Pair terms | 1–60 chars each, both or neither | a word or short phrase |
| Note | ≤ 200 chars | a tip, not a paragraph; fits a tablet card |
| Sender name | 1–40 chars | |
| Rate per sender | 5 tips per 60 s per (sender IP, sender name) | church devices through the tunnel share one public IP [TICKET #850 F2], so the name is part of the key |
| Rate overall | 30 tips per 60 s | keeps the interpreter's screen calm |
| Duplicate window | 2 min, same normalised pair + note, or note alone (a changed note is a new tip) | double taps, two šepkárs sending the same word |

Rate state lives in memory (token buckets, tokio clock, reset on restart).

### 9.9 Retention

- Rows older than 30 days are deleted at startup and once a day (a background task with an injected clock).
- The interpreter sees only active tips (§9.3); the šepkár page sees today's.
- The stored pairs feed the C6 glossary (engine B) and the ASR `context` of the same day (§6.3), so a word a šepkár
  had to explain once is recognised and translated better the next time.

## 10. Cost and latency per service hour

| Setup | Components | ≈ $/h | Source |
|---|---|---|---|
| **Soniox only** (both mics, built-in EN→SK) | 2 × `stt-rt-v5` $0.12 + translation tokens (~15 k × $4/1 M ≈ $0.06) + context per (re)start | **≈ $0.30** | [GAP] Soniox pricing; token estimate ours |
| Soniox + **LLM** per sentence | $0.24 + ~0.9 M input + ~40 k output tokens/h at the owner's model price | $0.24 + model-dependent | token volumes ours |
| Soniox + **Gemini Live Translate** for SK | $0.24 + $0.037/min × 60 | **≈ $2.46** | [SP] `dubbing-eval.md` |
| Šepkár tips | local | $0 | |
| Bake-off reference drafts (offline) | Gemini 3.5 Transcribe batch | ≈ $0.30 per hour of recording | [SP] `lyrics-eval-backends.md` |

With the auto start/stop on `preach` and the silence gate, a service streams ~1–1.5 h → Soniox-only ≈ $0.30–0.45 per
service; with Live Translate ≈ $2.5–3.7. Upstream bandwidth 2 × 256 kbit/s.

## 11. Failure modes

| Failure | Detected by | Interpreter sees | Operator / šepkár sees | System does |
|---|---|---|---|---|
| VBAN sender off / mic not routed | last packet > 3 s ago | banner "Nie je zvuk z mikrofónu hosťa (VBAN)" | chip red "hosť: žiadny zvuk" | keepalive; resumes on packets |
| Packet loss / sender restart | counter gaps / resync | nothing (silence inserted) | Settings health counters | silence for ≤ 1 s gaps; reset on big jumps; rate-limited WARN |
| Venue internet down (e.g. the 17 s tunnel drop on 2026-10-12 [TICKET #850 F2]) | WS close / connect failure | after 3 s banner "Bez internetu — prepis pozastavený", text dimmed; **tips keep working on the LAN** | chip orange | reconnect 1/5/20/60 s; resend last ≤ 5 s; manual console unaffected |
| Provider outage / 5xx / 429 | `error_type` / close code | banner "Služba prepisu nedostupná — skúšam znova" | chip orange + reason | backoff, `Retry-After` |
| Auth / payment (401/402/403) | `error_type` | banner "Prepis vypnutý — problém s účtom služby" | chip red + reason in Settings | stop, no retry loop |
| Translation engine fails (LLM) | error / 3 s timeout | EN keeps flowing; SK row "preklad nedostupný" | — | next sentence retried; built-in tokens as fallback if enabled |
| Wrong language on a mic | ≥ 70 % of final tokens over 30 s tagged ≠ configured | banner "Mikrofón hosťa: počujem slovenčinu, nie angličtinu"; SK suggestion suppressed for Slovak tokens | chip "hosť: iný jazyk" | no auto-switch (config is binding); detection keeps working |
| Guest bleeding into the interpreter mic | English tokens on the SK mic | — | — | ignored by the SK normaliser |
| Daily cap reached | cost meter | banner "Denný limit prepisu dosiahnutý" | chip grey | stop; the operator may restart (raises the cap for the day) |
| Wrong suggestion / wrong verse | — | reference shown large; the interpreter judges | a card only | expires after 3 min |
| Šepkár floods tips | rate limits | calm (≤ 30/min overall, ≤ 5/min per sender) | 429 message with the wait | tips beyond the limit refused |
| Šepkár page offline / server restart | WS state | — | connection dot red; a failed send stays in the fields with the error | the page resyncs on reconnect |
| Interpreter dismisses by mistake | — | 5 s undo toast | tip shows "skryté" | restore endpoint |

## 12. Privacy, retention, control

- Audio leaves the building only to the chosen ASR/translation provider, only while listening runs (cloud approved,
  §2).
- Audio is stored only with the recorder on (§5.5, per-day switch).
- Final sentences + suggestions go to `listen_transcript` / `listen_suggestions` (incremental migration), deleted
  after 7 days; tips after 30 days (§9.9).
- No key in the repo: `/etc/presenter/listen.env` (0600) written by the deploy workflows from GitHub secrets, the
  `ai.env` pattern (#761). The repo is public: no e-mails, IPs or keys committed.

## 13. Evaluation — published guest sermons replayed through Presenter's VBAN inputs (owner decision 2026-10-15)

**No live-service recordings, no consent question, no waiting for a guest.** The owner (2026-10-15): the engine
selection was already worked through for months in songplayer — reuse it; a guest preaches only exceptionally, so an
evaluation that needs 3–5 recorded guest services would stall the work; the church's YouTube channel has plenty of
guest services. "Bake-off" elsewhere in this spec means THIS evaluation.

**Inputs** (downloaded with `yt-dlp` on dev2 — no cookies needed on 2026-10-15 — kept under
`~/.claude/work-products/presenter/eval-audio/<youtube id>/`, NEVER committed: published church video, not ours to
redistribute):

- NEW LEVEL CHURCH (`UCrF2k6feQUQASgbB8i1ybAw`) "Bohoslužba s hosťom" — English guest + Slovak interpreter
  alternating on ONE track: Marty Sloan `vvlEx3pIa44` / `-I-cO-U06RA`, Nicolas Robinson `9PDaTGxvYxY`, Dr. Debbie
  Rich `xOkLny747Y4`, Todd Holmes `pGdCwFZZZwE` / `Bv8-cpCnrsk`, Ben Kroeske `qtTNpq34uFY`, John Huizing
  `jkw5aq8FxW0`, Dr. Peter Gammons `5dauIYkCrhg`. The sermon is located by a pause map (100 ms frames below −40 dBFS
  per minute: worship ≈ 0 %, sermon with interpretation 15–35 %); e.g. `vvlEx3pIa44` worship 14–48 min, sermon
  ≈ 50–135 min.
- songplayer's dubbing sample `Dhp-qrZDK1g` 02:00–04:00 (English only), with songplayer's Gemini 3.5 Transcribe
  transcript (word timings) and its EN/SK line pairs (`seg_spec.json`) — the reference text for the EN transcript.

**Replay.** `scripts/dev/listen/vban_send.py --file <audio> --start S --duration D --stream <mic stream>` decodes with
ffmpeg and streams INT16 VBAN to the dev server at 1.0× on ONE wall-clock anchor (songplayer's pacer pattern,
injectable clock). The whole Presenter path runs on real sermon audio: VBAN receive → per-mic chain → 16 kHz frames →
(C3) transcription → (C4) suggestions → (C5) interpreter display. The mixed track goes to the guest mic, so the
interleaved Slovak interpretation is part of the test (language per segment, interpreter speech not mistaken for the
guest's English).

**Engine (reuse, not research).** songplayer's production EN→SK path is Gemini Live Translate
(`gemini-3.5-live-translate-preview`): one continuous session, 16 kHz mono s16le 100 ms frames, input (EN) + output
(SK) transcription, first voiced SK output ≈ 3.1 s, the EN input transcription ≈ 4–5 s after the onset (songplayer
`dabing.md` round H, `dubbing-eval.md`). C3 starts with it; songplayer's probe
`eval/dubbing/live_translate_continuous_probe.py` is the reference tool. A second engine (Soniox real-time) is added
only if the replay MEASURES an EN latency the interpreter cannot use — measured, not re-researched.

**Metrics (from the replays, every denominator stated):** EN transcript arrival − input position (p50/p90), SK output
arrival − input position, the EN transcript of `Dhp-qrZDK1g` against songplayer's reference text (WER), what the
engine does with the interpreter's Slovak on the guest track, reference detection (C4) on the replayed sermons
(references labelled from the transcripts) and false cards per hour, cost per hour. Interpreter ratings come from real
use later; they are not a precondition.

**Where:** dev2, Python under `scripts/dev/listen-eval/` + the replay tool, against the dev server; keys via
`airuleset.py secret exec`, never on argv.

## 14. Observability

- INFO `presenter::listen::mic` on state changes; `presenter::listen::asr` on connect/close/error + one session
  summary (`seconds_streamed`, `reconnects`, `tokens_final`, `cost_estimate`); `presenter::listen::suggest` per card.
- INFO `presenter::interpreter::tip` per create/dismiss/restore (`id`, `sender_name`, `client_ip`, `chars`,
  `duplicate`); one WARN per 60 s per key while rate-limited (`suppressed` count).
- WARNs through the #484 power-of-two gate (`log-flood-backoff.md`).
- `GET /listen/status`: per-mic health, ASR state + reason, language mismatch, cost meter, last latency figures.
- Post-deploy live gate per listen provider; `docs/listen-providers.md` = provider, model, credential, gate
  (songplayer's `docs/ai-providers.md` shape [SP]).

## 15. Testing

**Šepkár (units S1a/S1b)**

- Repository: insert / active list (date + 4 h window) / today list / dismiss / restore / dismiss-all / 30-day
  retention, with an injected clock.
- Router: 201 valid pair, valid note, both; 422 for empty, half pair, > 60 / > 200 / > 40 chars, control
  or invisible characters only; 429 on the 6th tip within 60 s per (IP, name) with a paused tokio clock, and the overall 30/min;
  duplicate within 2 min → 200 `duplicate: true` and no new row; dismiss idempotent; restore 404 / 409; each write
  publishes `InterpreterTips` with the full active list; the date / 4 h boundary empties it.
- Companion: the new `LiveEvent` variant leaves Companion variables unchanged.
- Playwright `interpreter-tips.spec.ts` (two pages, one server):
  - šepkár on `PHONE_390` (touch), interpreter on a tablet profile 1180×820 touch;
  - send "atonement" → "zmierenie" as "Peter" → the card is visible on the interpreter **within 1000 ms of the
    click**, with "Peter" and the time; a second tip lands ABOVE it;
  - ✕ hides it on the interpreter, the šepkár list shows "skryté"; "Späť" within 5 s restores it;
  - reload the interpreter → the active tips come back (resync);
  - 201-char note → the inline 422 message; the 6th tip in a minute → the 429 message;
  - šepkár inputs ≥ 16 px, no horizontal overflow, controls ≥ 44 px (A+B helpers);
  - TV layout shows the newest 2 tips, no buttons;
  - zero console errors/warnings.

**Audio and listening**

- Decoder: round-trip against the ported encoder for the 15 geometry rows + mono and stereo INT16 + INT32; rejects
  (magic, non-audio sub-protocol, VBCA, reserved bit, size mismatch, SR index ≥ 21, BYTE8/FLOAT64); counter wrap.
- Receiver: real `UdpSocket` loopback (songplayer's pattern): selection by name + IP, gap → silence, late drop,
  resync, drop-oldest on overflow.
- Resampler: a 1 kHz tone keeps its frequency, exact block length, constant delay.
- Soniox client: a fake WS server replaying recorded frames: config first, empty text frame at the end, keepalive in
  the silence gate, reconnect on 1001/413/503 with backoff, no retry on 401/402, branching on `error_type`.
- Transcript rules: songplayer's H6 tests ported + fixtures from our recordings.
- Detectors: a committed spoken-reference corpus (≥ 80 EN + ≥ 80 SK phrases), fusion window and expiry with an
  injected clock.
- Server integration: VBAN over loopback → decode → resample → `mock` ASR records the frames.
- Playwright (mock provider replaying a JSONL): tablet — EN words, SK suggestion under its sentence, the SEB text of a
  cited verse in the Bible area (asserted against the DB), scroll-up pauses auto-follow and "Naživo" resumes, ⏸
  freezes; TV — last 2–3 sentences, no controls, no overflow at 1920×1080; console — a "Návrhy AI" card, tap
  previews, 0 `POST /bible/trigger-slide` until ZOBRAZIŤ; outage banner when the mock dies; zero console errors.
- Mutation: `/mutation-sweep` after the decoder, detector and tips units merge; survivors there are that unit's work.

## 16. Phasing into PR units

Each PR: version bump first, ≤ 600 LoC (fixtures/generated tables excluded), CI all green, auto-merge, deploy,
post-deploy verification. Playbook rules land with their PRs (`.claude/rules/interpreter-tips.md`,
`listen-vban.md`, `listen-asr.md`, `interpreter-display.md`).

**The šepkár units need no audio, no ASR and no keys, so they ship FIRST — before C1a — and give the interpreter a
usable tablet page right away.**

| # | Unit | Content | ≈ LoC | Depends on |
|---|---|---|---|---|
| **S1a** | **Šepkár server** | migration + repository, `state/interpreter_tips.rs`, `router/interpreter_tips.rs` (5 endpoints), validation, rate limits, duplicate window, `LiveEvent::InterpreterTips` + Companion no-op arm, retention task, INFO lines | 420 + 300 | — |
| **S1b** | **Šepkár page + interpreter tablet shell** | `/ui/sepkar`, `/ui/interpreter?layout=tablet` with the tips area (dismiss, swipe, undo, dismiss-all, highlight), resync, `interpreter-tips.spec.ts` | 520 + 250 | S1a |
| — | **Milestone M-T:** šepkár tips usable at the next guest service (T-S1–T-S3) | | | |
| C1a | **VBAN ingest core** | `presenter-core/src/audio/vban.rs` (ported codec + decode), `listen/vban_rx.rs`, per-mic chain + resampler, `listen.mics` config, `GET /listen/status` | 450 + 250 | — |
| C1b | **Settings + recorder** | Settings "Počúvanie" card (shared `ListCard`, health), recorder toggle + retention | 350 + 150 | C1a |
| — | **Owner action:** two VBAN streams → box acceptance (§5.8) (no recordings needed, §13) | | | C1b |
| C2 | **Replay evaluation** (scripts only, alongside C3) | `vban_send.py --file` replay, downloads + sermon map of the §13 guest services, songplayer probe run, metrics + report | scripts | Gemini key (for the engine runs) |
| C3 | **Transcription** | provider trait, Soniox client (`tokio-tungstenite` 0.29 — today a dev-dependency + axum's `ws` — moves to `[dependencies]` with a rustls TLS feature), mock provider, session lifecycle (auto start/stop, paid gate, silence gate, reconnect, cap, cost meter), transcript assembly, `/listen/ws` + snapshot, retention tables | 550 + 250 | C1a, Soniox key |
| C4 | **Verse suggestions** | detectors EN/SK + state + fusion (core), `LiveEvent::BibleSuggestions`, console "Návrhy AI" + listening chip | 500 + 250 | C3, A1a/A1b (resolver), B2d (sections) |
| C5a | **Interpreter tablet: transcript + Bible** | in the S1b shell: EN history, scroll/pause + "Naživo", Bible area (detected / on-air, DB text, numbering warning), numbers/names, status bar, A−/A+ | 500 + 200 | C3, C4, S1b |
| — | **Milestone M-C1:** interpreter tablet shows the guest live + verse texts + tips; operator gets cards | | | |
| C6 | **Slovak suggestion** | the bake-off winner (A pairing / B LLM with glossary incl. tip pairs / C Live Translate port), SK under each EN sentence, "↳ Biblia" substitution, EN / EN+SK / SK toggle | 300–550 + 200 | C2 verdict, C5a |
| C5b | **Interpreter TV layout** | `?layout=tv`: last 2–3 sentences, big type, Bible area, tip strip (newest 2, 5-min TV filter), no controls | 300 + 150 | C6 |
| C7 | **Hardening** | language-mismatch detection + banners, provider failover if the bake-off keeps a second engine, provider live gate, `docs/listen-providers.md`, cost summary | 300 + 150 | C6 |

**Conditional, decided by the bake-off data (filed as tickets only if §13's rule says so):** quote matching without
a spoken reference; an LLM for unclear relative references; marking the EN sentences already interpreted (from the
SK mic).

## 17. Risks

| Risk | Mitigation |
|---|---|
| Soniox English or its built-in Slovak not good enough on sermon audio | provider trait from day one; two alternative SK engines + an EN fallback in the bake-off |
| Gemini Live Translate is a preview with no instructions; text-only output undocumented | only a candidate; songplayer's upgrade path = model id as a setting [SP] |
| EN ↔ SK versification shows the wrong Slovak verse | numbering warning + English text side by side; a mapping table later |
| Keepalive in the silence gate may lose stream context **[unverified]** | checked in C3; fallback reconnect + context resend |
| FOH routing on fohabl touches critical production | owner's go; `Mute`-only, read-back discipline [SP] |
| Transcript partials flooding operator/stage sockets | separate `/listen/ws`; suggestions and tips as full-list events |
| Cost runaway | auto stop off `preach`, silence gate, daily cap, cost meter |
| Šepkár spam or a wrong tip distracting the interpreter | Access + LAN only; rate limits; one-tap dismiss + dismiss-all; plain text only |
| Tip latency on a busy server | one POST + one event, no cloud; the 1 s assertion is a real bound, not load-sensitive timing |
| Near-cap files (`router.rs`, `live.rs` match sites) | logic in new modules; size/fn gates on every touched file |
| `tokio-tungstenite`'s TLS feature pulls new crates **[unverified]** | rustls feature matching reqwest's; check the lockfile diff in C3 |

## 18. Decided defaults (the owner may override any)

1. Listening starts when the stage layout switches to **Kázeň (`preach`)** and stops 60 s after leaving it; the
   console toggle overrides for the rest of the day.
2. Suggestions **never** go on the wall by themselves; a tap previews, ZOBRAZIŤ shows.
3. **A cited verse on the interpreter display uses the same translation as the wall** (today SEB).
4. Cloud transcription/translation is approved by the owner's request for the best existing models.
5. Transcripts + suggestions kept 7 days; tips 30 days; audio only with the recorder on.
6. `/ui/interpreter` is its own page (not in the operator's layout picker); tablet layout by default, TV via
   `?layout=tv`.
7. The mic feed is taken after the mute (a muted mic sends silence).
8. Daily cap 4 h of streamed audio per mic; silence gate after 10 s below −50 dBFS.
9. No mic configured = no audio subsystem runs (PP).
10. Šepkár: anyone who can open Presenter may send tips (no extra login); the name is typed by the sender; a tip
    cannot be edited (send a new one); active tips = today's, last 4 h, not dismissed.
11. Marking "already interpreted" sentences is not in v1.

## 19. Owner question — withdrawn (2026-10-15)

The recording-consent question is withdrawn: the evaluation replays published guest sermons (§13). The C1b recorder
stays a debugging tool (off by default, per-day switch).

## 20. Owner actions (not questions)

- **VBAN streams:** on the FOH PC (VB-Matrix on fohabl, or Voicemeeter) send the guest mic and the interpreter mic
  as two VBAN streams to `10.77.9.205:6980`, 48 kHz, 16-bit, names `GUEST_EN` / `INTERP_SK`, taken after the channel
  mute. Claude can do the VB-Matrix routing instead, with the owner's go, following songplayer's FOH rule.
- **Key for Presenter (its own, not songplayer's):** a Gemini API key for Live Translate (C3 + the replay
  evaluation), through the one-time secret-request page; stored as a GitHub secret for the deploy and in the dev2
  vault for the evaluation. A Soniox key only if the replay measures a need for a second engine (§13).
- None needed for the šepkár units.

## 21. Sources

**Songplayer** (`zbynekdrlik/songplayer` @ dd551b7d, read-only clone):

- `.claude/rules/vban-out.md`, `.claude/rules/audio-outputs.md`
- `crates/sp-server/src/playback/vban_packet.rs`, `vban_packet_tests.rs`, `vban_packet_tests_format.rs`,
  `vban_rate.rs`, `vban_out.rs` (sender socket only)
- `.claude/rules/dabing.md`, `scripts/dub_live_session.py`
- `.claude/rules/dubbing-eval.md`, `eval/dubbing/README.md`, `eval/dubbing/live_translate_continuous_probe.py`
- `.claude/rules/lyrics-eval-backends.md`, `eval/lyrics/reports/` (lyrics only)
- `.claude/rules/ai-proxy.md`, `.claude/rules/paid-ai.md`, `docs/ai-providers.md`
- `docs/superpowers/specs/2026-09-17-dubbing-feature-design.md`

**Gaps researched (small targeted lookups):**

- VBAN receive side (SR list, sub-protocol / data type / codec values, reference receive algorithm, stack sizes):
  https://vb-audio.com/Voicemeeter/VBANProtocol_Specifications.pdf (rev. 13, Sep 2025)
- Soniox real-time model + translation: https://soniox.com/docs/stt/rt/real-time-translation
- Soniox WebSocket API: https://soniox.com/docs/stt/api-reference/websocket-api
- Soniox pricing: https://soniox.com/pricing
- Gemini Live Translate limits: https://ai.google.dev/gemini-api/docs/live-api/live-translate

**Presenter code read for integration points:** `crates/presenter-core/src/live.rs` (LiveEvent),
`crates/presenter-core/src/stage_display.rs` (layouts, camera-crew precedent),
`crates/presenter-server/src/router.rs` (`/ui/*` routes), `router/request_log.rs` (client IP rule),
`ai/client.rs` (`call_chat_completions_with_options`), `crates/presenter-server/Cargo.toml` (dependencies).

**Earlier ticket research (cited, not redone):** #850 Findings 2–4, #852 Finding 1.

---

## 22. AMENDED by the evaluation — 2026-11-14 (#852)

**Status:** amendment written 2026-11-14 from the measured replay evaluation and the owner's decisions of 2026-11-13/14
on #852 (comments of those dates are the evidence). It **replaces §6.2 and §6.3**, adjusts §1 C-S2, §6.4, §6.5,
§8.5, §8.6, §10, §11, §13, §16 and §20 as listed below, and **keeps everything else** (šepkár, VBAN/Dante ingest,
interpreter layouts, verse suggestions §7, privacy §12). Implementation plan: `docs/superpowers/plans/2026-11-14-listen-c3-soniox.md`.

### 22.1 Owner decisions (2026-11-13 / 14, ROZHODNUTÉ on #852)

1. **Text, not dubbing.** The interpreter needs TEXT; Gemini Live Translate is an audio→audio dubbing engine whose EN
   input transcription arrives per phrase, 3–5 s late (already noted in §8.5). Owner priorities: high quality,
   Christian context, and fast + correct capture of cited Bible references (C-S4 / C-S5 / C-S6).
2. **EN → EN first, translation later.** The interpreter's most common problem is a missed word or a missed verse,
   not the translation. The guest's live English transcript ships first; the Slovak suggestion (§8.6) is decided
   after it.
3. **Only the guest's microphone is transcribed** — never a mix of guest + interpreter, and the interpreter mic is not
   sent to the cloud in this phase.
4. **Tests run on English-only sermons** (no manual removal of the interpreter from mixed tracks): Marty Sloan
   `UvP1l1RjmQY`, `LwFI3gwp7hU`; Jonathan Shuttlesworth `darlKC25jLs`; Ben Kroeske `8STuemX2aAU` (kept under
   `~/.claude/work-products/presenter/eval-audio/en-only/` on dev2, never committed). The Gemini Live Translate run on
   the mixed track (`vvlEx3pIa44`) is not valid for the real guest mic.
5. ElevenLabs not tested (no new account). Keys received: Gemini, AssemblyAI, **Soniox** (dev1 + dev2 vault
   `~/.secrets/listen-soniox-api-key`).

### 22.2 What was measured (1.0× real-time streaming replay, dev2)

Accuracy on the 10-min Sloan slice (`LwFI3gwp7hU` 180–780 s) against three independent batch references (AssemblyAI
universal-3-5-pro, Soniox stt-async-v6, Gemini 3.6 Flash; the references disagree among themselves 1.9–2.8 % = the
noise floor). "Visible" = a word's end in the audio → that word first visible in the engine's text (partial or
final), one engine-neutral scorer (`timeline_score.py`).

| Engine (streaming) | WER | Visible p50 / p90 | Citations (Shuttlesworth, 38 real) | $/h | Verdict |
|---|---|---|---|---|---|
| **Soniox stt-rt-v6 + context** (domain + Bible books + terms) | **1.9 %** | 0.76 / 1.55 s | **38 / 38** (incl. James 5:16) | **0.12** | **chosen** |
| Soniox stt-rt-v6 (no context) | 2.1 % | 0.72 / 1.51 s | — | 0.12 | |
| AssemblyAI Universal-3.6 Pro | 2.1 % | 0.65 / 1.61 s (partials only every ~1.3 s; finals at turn end, p90 15–41 s) | — | 0.45 | second |
| AssemblyAI universal-streaming-english + format + keyterms | 3.5 % | 0.60 / 1.38 s | 37 / 38 (misses James 5:16) | 0.15 | |
| Gemini 3.5 Transcribe Live | 23.5 % (drops whole phrases, e.g. "Matthew 5:13-16" → "514") | 0.50 / 1.31 s | — | ~0.54 | **out** |
| Gemini Live Translate (EN input transcription) | — | phrase chunks, first text 3.2 s; mixes languages | — | ~2.2 | **out for EN** |

Soniox also formatted the references ("Isaiah 53:5", "1 Peter 2:24", "Mark 10:29 to 30", "Matthew 5:13-16") and
punctuated sentences. The context changed neither the latency nor the slice's WER beyond 0.2 pt; it is kept because
it costs nothing and targets exactly the vocabulary the interpreter misses.

### 22.3 §6.2 replaced — Soniox stt-rt-v6 is THE engine for the guest's English transcript

- **One continuous Soniox real-time session for the guest mic only**, `stt-rt-v6` (released 2026-11-03; `stt-rt-v5` is
  removed 2026-12-15 — the model id is an env setting so the next rename is a deploy, not a code change).
- **Config = the measured one:** `audio_format: "pcm_s16le"`, `sample_rate: 16000`, `num_channels: 1`,
  `language_hints: ["en"]`, and `context`:
  - `general`: `[{key: "domain", value: "Christian church sermon"}, {key: "topic", value: "preacher reads and cites
    Bible books, chapters and verses"}]` (the measured wording);
  - `terms`: the **66 English Bible book names** (from the resolver's English name table, not a hand list — the
    evaluation's list missed eight books), a curated list of Christian terms (Holy Spirit, Pentecost, righteousness,
    sanctification, atonement, Gethsemane, Pharisees, …), **today's prepared references** (#851, "Romans 8:37" style,
    English names) and **today's šepkár source terms** (§9.9);
  - no `translation`, no `translation_terms` in this phase (§22.10).
- **Endpoint detection** is NOT part of the measured config. It ships off; the acceptance run measures it on
  (`enable_endpoint_detection: true`, `max_endpoint_delay_ms` 1000–1500) and keeps it only if finals arrive sooner
  without hurting WER (it also gives the `<end>` token that closes a sentence, §6.5).
- **Gemini Live Translate and Gemini Transcribe Live are dropped for the EN transcript** (measured: phrase-chunk EN
  3+ s late and language mixing; Transcribe Live drops whole phrases incl. references, 23.5 % WER). Live Translate
  is also out as the SK engine (§22.10).
- The provider trait (§6.1) stays: `StreamingAsr` is implemented by `soniox.rs` + the test mock; a second engine
  (AssemblyAI Universal-3.6 Pro, measured second) is added only if Soniox fails in production.

### 22.4 §6.3 replaced — Soniox real-time protocol (docs.soniox.com, verified 2026-11-14)

- `wss://stt-rt.soniox.com/transcribe-websocket`; auth = HTTP header `Authorization: Bearer <key>` on the upgrade
  request (the start-message `api_key` field is deprecated and refused from 2027-01-15 — never sent); a non-default
  `User-Agent: presenter-listen/<version>` [SP: Cloudflare 1010].
- First message = TEXT frame with the config (§22.3; optional `enable_endpoint_detection`, `max_endpoint_delay_ms`
  500–3000, `endpoint_sensitivity`, `endpoint_latency_adjustment_level`, `translation {type: one_way,
  target_language}`, `context {general, text, terms, translation_terms}`).
- Audio = BINARY frames (100 ms of s16le). End of audio = an EMPTY TEXT frame; the server answers the remaining
  finals and `finished: true`, then closes.
- Control TEXT frames: `{"type":"finalize"}` → every pending token becomes final, followed by a `<fin>` token;
  `{"type":"keepalive"}` → keeps the stream open while no audio flows.
- Responses: `{tokens: [{text, start_ms, end_ms, confidence, is_final, language, translation_status}],
  final_audio_proc_ms, total_audio_proc_ms, finished?}`; errors `{error_code, error_type, error_message}` and the
  server closes. Non-final tokens are REPLACED by every response; final tokens are sent ONCE. `<end>` = an endpoint
  (only with endpoint detection), `<fin>` = a finalize completed — both markers, never text. Tokens are sub-word;
  the text carries its own leading spaces [SP].
- **≤ 300 min per stream** → the client rolls over to a new connection before that (§22.5).
- **Production note (measured 2026-11-14):** Soniox does **not answer WebSocket pings** promptly — the Python
  client's ping keepalive killed a run at 234 s (1011). The Rust client NEVER sends WS pings; liveness = the
  response cadence while audio flows (measured: a response every 0.19 s p50, 0.41 s p90, max 1.19 s) and Soniox's own
  `{"type":"keepalive"}` while no audio flows.
- Price $0.12 per streamed hour [unverified: whether a kept-alive stream with no audio is billed — the client bounds
  it anyway, §22.5].

### 22.5 §6.4 adjusted — lifecycle of the guest session

- **Which mic is the guest's:** the Dante mics are fixed (`dante-mic-1` / `-2`, role `moderator`, language `off`,
  #933), so the language cannot pick. A setting names it: `listen.transcription.guestMic` (a mic id from the
  published list, VBAN or Dante), chosen in Settings → Počúvanie before the service; none = nothing runs.
- Start / stop, paid gate, daily cap, cost meter: unchanged (§6.4, §18 defaults 1, 8) — only for that one mic.
- **Lazy connect:** no connection until the first voiced frame (> −50 dBFS); a 1 s pre-roll goes first.
- **Silence gate (replaces the keepalive sentence of §6.4):** 10 s below −50 dBFS (or 10 s without frames) → send
  `finalize` once (the open tail becomes final text at once), stop sending audio, send `keepalive` every 5 s;
  the first voiced frame resumes with its 1 s pre-roll on the same connection. After **5 min gated** the connection
  ends (empty text frame) — a later voiced frame reconnects with the same context. This bounds the billed time
  whatever Soniox counts for a kept-alive stream.
- **Reconnect** (close without `finished`, 408, 429, 5xx, a connect failure, a stall = no response for 15 s while
  ≥ 10 s of audio went out): after 1, 5, 20, 60 s, then every 60 s; the audio since the end of the last FINAL token
  (≤ 5 s) is resent first, so nothing finalized is transcribed twice and the unconfirmed tail is not lost.
  401 / 402 / 403 / 400 → stop, status `auth` / `payment` / `refused`, no retry loop. More than 20 connections in
  10 min → stop (a reconnect storm is a cost runaway).
- **Rollover before 300 min:** from 280 min of connection age, at the first gated moment (else at 295 min
  regardless): `finalize`, wait for `<fin>` (≤ 3 s), end the old stream, open the new one with the same config and
  send the frames buffered meanwhile — no audio lost, no word split.

### 22.6 §6.5 adjusted — transcript assembly from Soniox tokens

- Final tokens append to the open sentence; non-final tokens are the live tail and are replaced by every response;
  translation tokens (`translation_status: "translation"`) are ignored in this phase.
- A sentence closes at `. ! ? …` (a following quote allowed) when the next final token starts a new word, at 20 words
  (songplayer's cap), at `<end>` or `<fin>`; a pause alone never closes a sentence (`<fin>` is sent only by the
  silence gate, `<end>` only with endpoint detection).
- Token times are per connection; the session time = the connection's audio offset + `start_ms` / `end_ms`.

### 22.7 §1 C-S2 adjusted (derived from the verdict; the owner may override)

No accurate engine met "EN word p90 ≤ 1.0 s": the faster ones lose words (fast AssemblyAI 3.5 % WER) or whole phrases
(Gemini). The owner chose accuracy. **C-S2 becomes: guest speech → English word visible on the interpreter display,
p50 ≤ 0.9 s and p90 ≤ 1.8 s, measured on the Presenter path** (Dante/VBAN → chain → Soniox → `/listen/ws`; the
engine alone measured 0.76 / 1.55 s). **C-S3** (SK suggestion) waits for §22.10. C-S4 / C-S5 unchanged (C4).

### 22.8 Inputs for §7 (verse detection, the next plan after C3)

Seen in Soniox's real output and binding for the detector's corpus:

- references mostly formatted ("Isaiah 53:5", "1 Peter 2:24", "Matthew 5:13-16", "Mark 10:29 to 30", "Matthew chapter 5",
  "verse 5"), but **sometimes bare digit pairs** ("John 10 10") → accept `<book> <n> <n>` as chapter + verse;
- **translation names are not references:** "King James", "Amplified (classic)", NKJV / NIV / ESV / NLT / "the Message"
  must never produce a book — Soniox wrote "2 King James. 1:2." around a translation name (must NOT become 2 Kings 1:2);
- an allusion is not a citation ("build me a strong church" ≠ Matthew 16:18); "the book of Acts" = a book without a
  chapter (low confidence, no card);
- §7.2 fusion of two mics: the interpreter mic is not transcribed in this phase (§22.1), so C4 v1 detects on the guest
  transcript only; the SK-mic half of §7.1 / §7.2 returns if the interpreter mic is ever transcribed.

### 22.9 §8.5 adjusted

Row "Cloud ASR partial": measured (Soniox at the client) 0.76 s p50 / 1.55 s p90 from a word's end; the budget row
"Speech → EN word" follows C-S2 as amended (§22.7).

### 22.10 §8.6 adjusted — the Slovak suggestion stays OPEN

Decided after the EN transcript ships, by a replay of the English-only sermons scored by the owner's reading
(Christian idiom first). Candidates:

| Engine | Note from the evaluation |
|---|---|
| **A. Soniox built-in translation** (`translation {type: one_way, target_language: "sk"}` in the same session) | no second session; `context.translation_terms` now exists (a glossary: šepkár pairs + a curated Christian list) — the "no glossary" objection of §8.6 is outdated; quality on sermons unmeasured |
| **B. LLM per finished sentence** with the Christian glossary + previous sentences (owner's model) | most controllable; +0.5–1.5 s; token cost |
| ~~C. Gemini Live Translate~~ | **dropped**: errors on biblical idiom ("the story of Saul to Paul" → "príbeh Saula na Pavla"), audio output paid but unused (~$2.2/h), ~3 s behind |

### 22.11 §10 adjusted — cost

Guest mic only: **$0.12 per streamed hour**; with auto start/stop on `preach`, the silence gate and the 5-min park,
≈ $0.15–0.20 per guest service. The interpreter mic costs nothing in this phase. A future SK engine A adds Soniox
translation tokens; B adds the LLM's tokens.

### 22.12 §11 adjusted — failure modes (additions)

| Failure | Detected by | System does |
|---|---|---|
| Soniox `error_code` 401 / 403 / 402 / 400 | response `error_code` | stop, status `auth` / `payment` / `refused` + Slovak reason, no retry |
| 408 / 429 / 5xx / close without `finished` / stall 15 s | response or close or response cadence | reconnect 1 / 5 / 20 / 60 s, resend since the last final (≤ 5 s); the interpreter banner "Služba prepisu nedostupná — skúšam znova" after 3 s |
| 300-min stream limit | connection age | planned rollover at a gated moment (§22.5), no banner |
| WS ping unanswered | — | never sent (§22.4) |

### 22.13 §13 adjusted — the evaluation is DONE for EN

Method as run: English-only sermons, 1.0× streaming replay, word-level arrival scorer, three batch references,
citation lists drafted by Gemini 3.8 Flash and checked by hand. Results and the verdict: #852 comments of
2026-11-13 / 14; scripts and logs: `~/.claude/work-products/presenter/eval-audio/en-only/runs/` on dev2. The C3
acceptance repeats the Sloan slice and the Shuttlesworth windows through the real Presenter path (§22.7 numbers,
citations ≥ 37 / 38).

### 22.14 §16 adjusted — phasing

| # | Unit | Plan |
|---|---|---|
| C3 | Guest EN transcript: Soniox session for the guest mic, assembly, `/listen/ws`, the interpreter's EN column, key plumbing, Settings, mock + E2E, real-API acceptance | `docs/superpowers/plans/2026-11-14-listen-c3-soniox.md` |
| C4 | Verse detection (§7, with §22.8) + console cards | next plan, after C3 |
| C5a | Interpreter tablet: the rest (⏸, A−/A+, Bible area, numbers + names) | after C4 |
| C6 | SK suggestion (§22.10) | after the EN transcript runs at a real service |
| C3-persist | `listen_transcript` 7-day store (§12) | own small lane after C3 (the transcript lives in memory until then) |

### 22.15 §20 adjusted — owner actions

- **No new key needed:** the Soniox key is in the dev1 / dev2 vault; Claude stores it as the GitHub secret
  `LISTEN_SONIOX_API_KEY` from the vault (stdin, never argv or chat) in the acceptance lane. The Gemini key is not
  used by Presenter's C3 (it stays in the vault for evaluations).
- **Before a guest service:** pick the guest's mic in Settings → Počúvanie → "Prepis hosťa" (Dante mic 1 or 2) and
  switch the transcription on (off by default, §18 default 4 unchanged in spirit: the owner's money first).

### 22.16 Sources (2026-11-14)

- docs.soniox.com, read 2026-11-14: the WebSocket API reference (endpoint, Bearer header, config fields, binary audio,
  empty text frame = end, responses, error fields), real-time endpoint detection, manual finalization (`finalize`,
  `<fin>`), keepalive, context (`general`, `text`, `terms`, `translation_terms`), models (`stt-rt-v6`, `stt-rt-v5`
  removal 2026-12-15, 300 min per stream) and pricing ($0.12/h).
- Coval real-time benchmark (2026-11-13), Pipecat STT benchmark (Aug 2026) — shortlist only; the decision came from
  our replay (#852).
