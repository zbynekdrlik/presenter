> **DRAFT — not approved yet.** Owner decisions so far are recorded on #850 / #852 (tablet layout first; šepkár tips page added 2026-10-14). Becomes the approved spec only after the owner review.

# Listening to the sermon — verse suggestions (#850 part C) + interpreter assist (#852) — DESIGN DRAFT

**Status:** working draft, NOT approved. Supersedes `850-c-design-draft.md` (that one predates the VBAN and
language decisions and the songplayer-reuse correction).
**Issues:** #850 part C (Presenter listens to the sermon and suggests Bible verses), #852 (live transcript +
suggested Slovak translation for the interpreter).
**Date:** 2026-10-14. **Presenter baseline:** `dev` @ 9c726944 (v0.4.318). **Songplayer baseline:** `dev` @ dd551b7d (v0.81.0).
**Builds on:** the approved A + B spec `docs/superpowers/specs/2026-10-13-bible-verses-any-device-design.md`
(the resolver §5, the console reserved sections §7.4, the show log §8).

**Source tags used below**

- **[SP]** a fact songplayer verified in production or against the live API (path given). Reused, not re-researched.
- **[GAP]** songplayer does not cover it; a small targeted lookup, source URL in §20.
- **[TICKET]** earlier research already on #850 / #852 (not redone, cited only).
- **[unverified]** an assumption; each one says how it gets checked.

---

## 1. Goal and success criteria

During a sermon with an English-speaking guest and a Slovak interpreter, Presenter:

- shows the **interpreter** what the guest is saying, word by word, with a suggested Slovak translation, and the
  **Slovak Bible text** of any verse the guest cites;
- offers the **operator** cards for the verses about to be read, from both microphones, one tap to preview.

| # | Criterion | Measured by |
|---|---|---|
| C-S1 | Both VBAN microphones are received on prod for a whole 2 h service. Every frame-counter gap is counted and logged; a stopped sender shows "žiadny zvuk" within 3 s | `/listen/status` read before/after + journal; box acceptance (§5.8) |
| C-S2 | Guest speech → English word on the interpreter display: p90 ≤ 1.0 s | token `end_ms` vs display arrival, replay + live (§12) |
| C-S3 | End of an English sentence → its Slovak suggestion on the display: p90 ≤ 2.0 s (hard ceiling 3 s, CAI research [TICKET #852 F1]) | same harness |
| C-S4 | A cited reference (English on the guest mic or Slovak on the interpreter mic) → the Slovak Bible text in the interpreter's Bible slot AND an operator card ≤ 1.5 s after the reference's last word | replay test with timestamps |
| C-S5 | On the bake-off recordings: ≥ 90 % of the references read aloud become a card (both mics fused), ≤ 2 false cards per hour | bake-off scorer (§12) |
| C-S6 | Verse text is ALWAYS from the database, never from machine translation. Nothing goes on the wall without the operator (tap = preview, ZOBRAZIŤ = show, A+B §7.4) | E2E: a suggestion produces 0 `POST /bible/trigger-slide` until ZOBRAZIŤ |
| C-S7 | Internet / provider outage → banner on the interpreter display + red chip on the console within 3 s; the manual console keeps working (no dependency) | E2E with the mock provider killed; replay with the network cut |
| C-S8 | No microphone configured = nothing runs and nothing is billed (PP). A daily cap stops streaming | unit test + `/listen/status` |
| C-S9 | Zero browser console errors/warnings in every new spec | each spec's final assertion |

## 2. Binding owner decisions (ROZHODNUTÉ on #850 / #852)

- **Audio source = two separate microphones over VBAN**, quality first ("budeš môcť sledovať prepisy obidvoch
  mikrofónov a vyhodnocovať a navrhovať miesto", #850, 2026-10-13).
- **Guest preacher speaks ENGLISH on one mic, interpreter speaks SLOVAK on the other; per-microphone language
  config** (#850, 2026-10-13). English book names/ordinals on the guest channel, Slovak on the interpreter channel.
- **The same guest transcript feeds the interpreter assist** (#852). Best existing models for transcription and
  translation (#852 owner text).
- **Reuse songplayer first**; vbanvst and audiomatrix are dead experiments and are never a source (#850/#852,
  2026-10-13). VBAN reuse = songplayer's production sender codec only; the receive side is designed from it.
- **The AI/LLM model is always the owner's choice** (project memory; prod today `google/gemini-3.8-flash`).
- From A + B: the reference resolver lives in `presenter-core` and is shared by every entry point; console cards
  only preview; Cloudflare Access protects the public hosts.

## 3. What is reused from songplayer

Songplayer has **no VBAN receiver** (only `UdpSocket::bind(("0.0.0.0", 0))` in the sender, `vban_out.rs:888`)
and **no real-time ASR** (its transcription is batch; its only live AI session is Gemini Live Translate on a file).
Those two parts are therefore new, designed from what songplayer did verify.

| Songplayer source | What it gives | Reused as |
|---|---|---|
| `crates/sp-server/src/playback/vban_packet.rs` | 28-byte LE header layout, `VBAN_MAGIC`, `VBAN_STREAM_NAME_LEN`, `VBAN_DATA_MAX` = 1436, `sr_index_of` (48k=3, 96k=4, 192k=5, 44.1k=16, 88.2k=17), format bits INT16 0x01 / INT24 0x02 / FLOAT32 0x04, `f32_to_int24` / `int24_le` / `f32_to_int16` / `clean_f32`, `stream_name_bytes`, `VbanEncoder` (counter +1 per packet, wraps at `u32::MAX`) | **CODE.** Constants + conversions ported into `presenter-core/src/audio/vban.rs` with the DECODE direction added. The encoder is ported **test-only** as the packet generator for decoder round-trip tests and the loopback test |
| `vban_packet_tests.rs` (`parse_packet`, `ramp_block`), `vban_packet_tests_format.rs` (15-row rate × format geometry table) | an independent decoder used as a test oracle; geometry pins | **CODE (tests).** `parse_packet`'s sign extension becomes production decode (without its `assert!`s); the 15 geometry rows become round-trip cases |
| `vban_rate.rs` (rubato `=5.0.1`, `Fft` + `FixedSync`, fixed in/out block sizes, delay = half the block FFT) | a sample-rate converter whose output length is exact per block | **CODE pattern.** Same crate, version and approach for 48 → 16 kHz |
| `.claude/rules/vban-out.md` | spec rev. 13 verified; default port 6980; "the RECEIVE thread never waits" (spec p. 11); Voicemeeter receivers keep ≥ 6 × 256 samples and senders burst 2–4 packets (spec p. 12); counter semantics; box acceptance by tcpdump (counter gaps, interval p99); FOH routing on fohabl = VB-Matrix over VBAN-TEXT, switch with `Mute` only, read back every point | **KNOWLEDGE** (receiver design §5, sender setup §19) |
| `vban_stall.rs`, `stat_window.rs` (`TwoBucketWorst`, `WarnLimiter`) | rolling worst value + one WARN per 5 s with `suppressed` | **KNOWLEDGE** (Presenter keeps its own #484 power-of-two log gate, `log-flood-backoff.md`) |
| `.claude/rules/dabing.md` round H + `scripts/dub_live_session.py` (`ContinuousSession`, `live_config`) | the production Gemini Live Translate session: model `gemini-3.5-live-translate-preview`; 16 kHz mono s16le 100 ms frames in, 24 kHz PCM out; `translation_config{target_language_code: sk, echo_target_language: False}`; `input_audio_transcription` + `output_audio_transcription`; `context_window_compression` (trigger 25 000 / sliding target 8 000 tokens) + `session_resumption`; GoAway = open the next connection with the ACTIVE connection's handle while still feeding the old one; a handle from a draining connection is ignored; ≤ 50 connections; 30 s connect/send timeouts; `google-genai==2.24.0` SDK traps (`receive()` ends per interaction → re-enter in a loop) | **CODE in the bake-off** (unchanged, §12). **KNOWLEDGE** for a Rust port only if Live Translate wins the Slovak column (§8.5) |
| `eval/dubbing/live_translate_continuous_probe.py` | replays an audio file at 1.0× through Live Translate, writes `input_text.txt`, `output_text.txt`, `events.jsonl` (every message with its arrival time), `summary.json` | **CODE (bake-off, unchanged)**: it produces exactly the latency + text data §12 needs for the Live Translate arm |
| `crates/sp-server/src/dabing/subtitles.rs` rules (`group_fragments`, `ends_sentence`, `MAX_WORDS_PER_LINE` = 20, "a pause never closes a line", `split_sentences_inside`, `assign_en_sentences` by content-interval overlap) | turning streamed fragments into sentence lines, and pairing EN sentences with SK lines; measured on a 36-min talk (95 % of SK lines end a sentence, mean EN coverage 0.93 with cap 20 vs 83 % / 0.88 with a 1.5 s pause rule) | **CODE (port of the pure rules)** for sentence assembly (§6.5) and EN ↔ SK pairing (§8.5) |
| `.claude/rules/dubbing-eval.md` | Live Translate price ~$0.037/min (in $0.0053 + out $0.0315); `gemini-3.8-live` with a "translate" instruction stops after ~1.4 s (turn-taking) → only the dedicated `-live-translate-` model works continuously; the session streams silence after speech; `VOICED_DBFS` −50 dBFS as the speech/silence line; Soniox needs a non-default `User-Agent` (Cloudflare 1010) | **KNOWLEDGE** (§6, §9) |
| `.claude/rules/lyrics-eval-backends.md` | live-verified API shapes: **Soniox** `stt-async-v5` (`Authorization: Bearer`, `translation:{type:"one_way", target_language:"sk"}`, translated tokens carry NO timestamps, `translation_status` original/none vs translation, sub-word tokens joined on leading spaces); **AssemblyAI** `universal-3-5-pro` (dashes; translation under `speech_understanding.request.translation`; `utterance["translated_texts"]["sk"]`); **Gemini 3.5 Transcribe** (`gemini-3.5-transcribe`, interactions API, `mode` is mandatory or 0 words come back, word timestamps on a 100 ms grid, ≈ $0.005/min, 30 min per file); scoring discipline (state the denominator, errored items stay in it, the decision rule is written before the run) | **KNOWLEDGE** for the Soniox token model (§6.3) and the bake-off; **CODE**: `eval/lyrics/backends/gemini_3_5_transcribe.py` drafts the reference transcripts in the bake-off |
| `.claude/rules/paid-ai.md` | one paid-AI switch per node, read where the paid call becomes certain; held work is held, never failed; calm logging while off | **KNOWLEDGE** (the listen start gate §6.4) |
| `.claude/rules/ai-proxy.md` | refusals are a prompt problem (neutral technical prompt); retry 5 s / 20 s / 60 s honouring `Retry-After`; newest-flagship policy verified live | **KNOWLEDGE** (LLM translation prompt + reconnect policy) |
| `docs/ai-providers.md` | one table of every external AI provider: model, credential setting, live post-deploy gate. Also: songplayer's `soniox_api_key`, `assemblyai_api_key`, `elevenlabs_api_key` are "stored but unused" and go with the key rotation (not before 21.10.2026) | **KNOWLEDGE**: Presenter adds the same table for the listen providers; Presenter gets its **own** keys (one account per project), never songplayer's |

**Not reused:** the sender-side scheduling (MMCSS, pacing, `SlewRemainder`, send latency L) — a receiver has no
playout clock. vbanvst / audiomatrix — owner ruling.

## 4. Architecture

Everything runs inside `presenter-server` on the prod box (10.77.9.205, Intel N100, iGPU only). Local ASR is
not realistic next to NDI + Companion [TICKET #850 F4], so transcription and translation are cloud calls.

```
FOH PC (VB-Matrix / Voicemeeter)
  ├─ VBAN "GUEST_EN"  ──┐  UDP 6980, LAN
  └─ VBAN "INTERP_SK" ──┤
                        ▼
presenter-server  listen/
  vban_rx  ── decode, pick stream (name + sender IP), gap → silence, health
     │
  mic[guest]  mic[interp]  ── channel select, 48 → 16 kHz (rubato), level, recorder (optional)
     │              │
  asr session    asr session        ONE session per microphone (provider trait; Soniox first)
  (lang en,      (lang sk)          the guest session is the ONE shared English transcription
   + EN→SK tokens)
     │              │
  transcript ◀──────┘  sentence assembly (songplayer H6 rules)
     │
     ├──▶ detect (EN + SK spoken references → presenter-core resolver) ──▶ fuse ──▶ LiveEvent::BibleSuggestions
     │                                                                  └──▶ Bible slot (SK text from the DB)
     ├──▶ translate (SK suggestion per finished EN sentence; engine per bake-off)
     └──▶ listen hub  ──▶ GET /listen/ws  ──▶ /ui/interpreter (EN live, SK suggestion, Bible slot, status)
                                         └──▶ console "Návrhy AI" section + listening chip
```

- **One shared English transcription.** The guest-mic session is the only transcription of that microphone. Its
  events fan out to the interpreter display, the reference detector and the translation layer. No consumer
  starts its own transcription (songplayer D3 made the same call: "a dubbed video does NOT run a second
  STT/translation pass" [SP `dabing.md`]).
- **Two transports to the browser.** Transcript traffic (several partials per second) goes over a dedicated
  `/listen/ws` so it never floods stage displays and operators. Suggestions (a few per minute) go through the
  normal live hub as one `LiveEvent` carrying the FULL current list, so the operator WS's latest-value coalescing
  (A+B §4) can never lose a suggestion.

## 5. Audio ingest — the VBAN receiver

### 5.1 Where and how it listens

- One UDP socket, `0.0.0.0:6980` (VBAN default [SP]), configurable port, in a tokio task.
- The receive loop never waits on anything downstream (spec p. 11 [SP/GAP]). Decoded blocks go into a bounded
  per-mic channel; on overflow the OLDEST block is dropped and counted (songplayer's queue rule [SP]).
- CPU: 2 streams × ~190 packets/s + two 48 → 16 kHz converters — negligible on the N100.

### 5.2 Decode (`presenter-core/src/audio/vban.rs`, pure, host-tested)

Ported from songplayer's encoder, decode direction added. The reference receive algorithm in the spec
[GAP: spec rev. 13 p. 10–11] fixes the checks:

1. Magic `VBAN`, length ≥ 28.
2. Sub-protocol `format_SR & 0xE0` must be AUDIO (0x00). SERIAL 0x20 / TXT 0x40 / SERVICE 0x60 / USER 0xE0 are
   counted as `ignored_non_audio` (VB-Matrix also speaks VBAN-TEXT on this port).
3. Codec `format_bit & 0xF0` must be PCM (0x00); VBCA/VBCV/USER → `unsupported_codec`.
4. Reserved bit `format_bit & 0x08` must be 0, else `corrupt`.
5. SR index `format_SR & 0x1F` < 21, from the full spec list
   `{6000, 12000, 24000, 48000, 96000, 192000, 384000, 8000, 16000, 32000, 64000, 128000, 256000, 512000,
   11025, 22050, 44100, 88200, 176400, 352800, 705600}` (songplayer maps 5 of them; same indices) [GAP].
6. Data type `format_bit & 0x07`: INT16 (2 B), INT24 (3 B), INT32 (4 B), FLOAT32 (4 B) accepted; BYTE8,
   FLOAT64, 12/10-bit → `unsupported_type`.
7. `(nbs + 1) × (nbc + 1) × sample_size == packet_len − 28`, else `corrupt` (spec: `nbByte == datasize`).
8. Output: `VbanPacket { stream_name, counter, rate_hz, channels, samples: Vec<f32> }` (interleaved, ±1.0).

Every reject is a typed error counted per sender; nothing panics on network data.

### 5.3 Stream selection, ordering, gaps

- A microphone is configured by **stream name** (exact, ≤ 16 ASCII) and optionally the **sender IP** (the spec's
  "check the IP-From and Stream Name"). Unmatched streams are listed in `/listen/status` as "seen, not used"
  (helps the first setup).
- Counter = expected → push. Counter ahead by 1–N packets totalling ≤ 1 s → insert that many packets of silence,
  count `lost_packets`. Counter behind (late/duplicate) → drop, count `late_packets`. A jump > 1 s or backwards
  by > 1 s (sender restart, counter reset) → reset the stream state, count `resyncs`.
- A rate/channel/format change mid-stream → reset the converter, log INFO.
- No jitter buffer: ASR has no playout clock, so packets are forwarded as they arrive (bursts of 2–4 packets are
  normal for Voicemeeter senders [SP]).

### 5.4 Per-mic chain

- **Channel:** `left | right | mix` (a mono mic sent as stereo is common).
- **Resample** 48 kHz (or whatever arrives) → **16 kHz mono s16le**, the input format both Soniox and Gemini
  Live accept [SP, GAP]. rubato `Fft` + `FixedSync`, fixed blocks (songplayer pattern); 100 ms frames out.
- **Level:** RMS dBFS per 100 ms frame; voiced above −50 dBFS (songplayer's `VOICED_DBFS` [SP]).
- **Health:** packets/s, `lost_packets`, `late_packets`, `resyncs`, last-packet age, level, current format.

### 5.5 Recorder (for the bake-off and debugging)

- Off by default. A Settings toggle writes each mic's 16 kHz mono stream to
  `/opt/presenter/recordings/<date>/<mic>.wav` (exactly what the ASR would get, so a replay reproduces
  production input). ~115 MB per mic-hour. Retention 30 days, then deleted by the server.
- Recording only with the owner's consent (§18 Q2).

### 5.6 Configuration

`app_settings` key `listen.mics` (JSON list), edited in Settings → "Počúvanie":

```json
[{ "id": "guest", "name": "Hosť", "streamName": "GUEST_EN", "senderIp": null, "channel": "left",
   "language": "en", "role": "guest" },
 { "id": "interp", "name": "Tlmočník", "streamName": "INTERP_SK", "senderIp": null, "channel": "left",
   "language": "sk", "role": "interpreter" }]
```

- `language`: `en | sk | off` (owner decision: per-mic). `role`: `guest | interpreter | preacher`. A normal
  Slovak service without a guest = one mic `sk`/`preacher`, the other `off` (no interpreter display needed).
- An empty list = the whole subsystem idle, no socket bound (PP).

### 5.7 Sender side (owner action, §19)

- VB-Matrix on fohabl (the FOH PC songplayer already sends to) or Voicemeeter sends two streams, unicast to
  10.77.9.205:6980, 48 kHz, INT16, mono or stereo. Names `GUEST_EN` / `INTERP_SK` (configurable).
- Tap point: **after the channel mute, before the fader** if the mixer allows it — a muted mic then sends silence
  (privacy), and the FOH engineer's fader rides do not change what the ASR hears.
- If Claude does the VB-Matrix routing, it follows songplayer's FOH rule: owner's go first, `Mute` only, record
  the prior values, read back every point after each write [SP `vban-out.md`].

### 5.8 Box acceptance

songplayer's method [SP]: a 10-min capture on the prod box (`tcpdump udp port 6980`) with 0 counter gaps and an
inter-arrival p99 < 15 ms, `/listen/status` read before and after (counters diffed), level moving when someone
speaks into each mic, silence when the mic is muted.

## 6. Transcription — one session per microphone

### 6.1 Provider trait

```rust
// presenter-server/src/listen/asr/mod.rs
#[async_trait]
pub trait StreamingAsr: Send + Sync {
    async fn open(&self, cfg: &AsrSessionConfig) -> anyhow::Result<Box<dyn AsrStream>>;
}
pub trait AsrStream: Send {
    fn send_frame(&mut self, pcm_16k_s16le: &[u8]) -> anyhow::Result<()>;   // 100 ms
    fn next_event(&mut self) -> /* async */ Option<AsrEvent>;
}
pub enum AsrEvent {
    Tokens { tokens: Vec<AsrToken> },              // partial + final, in order
    Finished, Error { kind: AsrErrorKind, message: String },
}
pub struct AsrToken { pub text: String, pub start_ms: Option<u32>, pub end_ms: Option<u32>,
    pub is_final: bool, pub confidence: f32, pub language: Option<String>, pub is_translation: bool }
```

Implementations: `soniox.rs` (first), `mock.rs` (replays a recorded JSONL, behind the existing
`mock-integrations` feature, used by E2E), and later only what the bake-off selects.

### 6.2 Why Soniox is the first implementation

Not a leaderboard pick:

- songplayer verified Soniox's API family and token model live (`stt-async-v5`, translation, `translation_status`,
  sub-word joining, the `User-Agent` trap) [SP];
- the owner already has a Soniox account (songplayer settings; `voiceagent/.env`) [SP `docs/ai-providers.md`,
  `dubbing-eval.md`];
- the real-time model `stt-rt-v5` gives, in ONE stream, English words with timestamps + Slovak translation tokens
  + a per-token language tag [GAP: Soniox docs] — exactly the shared session both consumers need;
- $0.12/h, translation included (token-billed) [GAP: Soniox pricing].

Its English accuracy and its Slovak translation quality are decided by the bake-off (§12) against songplayer's
production engine (Gemini Live Translate) and an LLM, not assumed.

### 6.3 Soniox session config [GAP: Soniox real-time docs]

- `wss://stt-rt.soniox.com/transcribe-websocket`, key in the connection's `Authorization` header (the config
  field `api_key` is deprecated), `User-Agent: presenter-listen/<version>` [SP: Cloudflare 1010 trap].
- First text frame:

```json
{ "model": "stt-rt-v5", "audio_format": "pcm_s16le", "sample_rate": 16000, "num_channels": 1,
  "language_hints": ["en"], "enable_language_identification": true, "enable_endpoint_detection": true,
  "context": "<Bible book names EN/SK, the guest's name, today's prepared references from #851>",
  "translation": { "type": "one_way", "target_language": "sk" } }
```

  The interpreter mic: `language_hints: ["sk"]`, no `translation`.
- Audio: binary frames of 100 ms. End: an empty TEXT frame (an empty binary frame is not an end).
- Keepalive control message at least every 40 s while no audio flows (silence gate §6.4). Max 300 min of audio
  per stream; a `max_duration_reached` (413) close → reconnect.
- Responses: `tokens[]` with `text`, `start_ms`/`end_ms` (absent on translation tokens), `confidence`,
  `is_final`, `language`, `translation_status` (`original`/`none` = spoken, `translation` = translated). Final
  tokens come once; non-final tokens are replaced by each response. Errors: `error_type` (branch on it, never on
  the message), codes 400/401/402/403/408/413/429/500/503; the server closes right after an error.
- Sub-word tokens are joined on leading spaces before grouping [SP].

### 6.4 Session lifecycle (`listen/session.rs`)

- **Start:** when the stage layout switches to `preach` (Kázeň) — the layout already brackets the sermon in the
  prod logs [TICKET #850 F2] — or by the console toggle. **Stop:** 60 s after leaving `preach`, or the toggle.
  The toggle overrides the automation until the next day (decided default §17-1).
- **Paid gate:** the start reads one switch (`listen.enabled` + a configured mic + a key present), at the moment
  the paid call becomes certain — songplayer's `paid_ai` rule [SP]. While off, nothing is held as an error; the
  status says why ("vypnuté", "chýba kľúč", "žiadny mikrofón").
- **Silence gate:** after 10 s below −50 dBFS the session stops sending audio and sends keepalives; the first
  voiced frame resumes with a 1 s pre-roll. Cuts cost during worship and long pauses.
  **[unverified]** that a Soniox stream kept alive by keepalives resumes with its context; checked in C3 against the
  real API, fallback = reconnect on resume.
- **Reconnect:** close / 1001 / 408 / 413 / 429 / 5xx → reconnect after 1 s, 5 s, 20 s, 60 s, then every 60 s
  (songplayer's retry spacing, honouring `Retry-After` [SP `ai-proxy.md`]); the last ≤ 5 s of audio is resent so a
  sentence is not cut. 401/402/403 → stop, status `error{auth|payment}`, no retry loop.
- **Daily cap:** 4 h of streamed audio per mic (configurable); reaching it stops the session with a status line.
- **Cost meter:** seconds streamed per mic per day, in `/listen/status` and one INFO summary per session.

### 6.5 Transcript assembly (`presenter-core/src/listen/transcript.rs`, pure)

- Final tokens build **sentences** with songplayer's measured rules [SP `subtitles.rs`]: a sentence closes at
  `. ! ? …` (also inside a token, `split_sentences_inside`), at 20 words, or at a Soniox endpoint; **a pause
  never closes a sentence**.
- Non-final tokens are the "live tail" of the current sentence (shown lighter).
- Each sentence keeps `start_ms`/`end_ms` (stream time), the language of its tokens, and its translation tokens
  (paired by order: Soniox emits translation tokens right after the spoken tokens they translate [GAP]).

## 7. Consumer 1 — verse suggestions for the operator

### 7.1 Spoken-reference detection (`presenter-core/src/listen/detect/`, pure)

The A resolver (`bible::resolver`, A+B §5) already knows Slovak, Czech and **English** book names (T2, BPRP
`eng`) and validates chapter/verse against a versification. The detector only turns spoken text into something
the resolver can read:

- **Number normalisers**, chosen per token language (not per mic config, so a wrong-language mic still detects):
  - English: "three sixteen", "chapter three verse sixteen", "verses sixteen through eighteen", "first
    Corinthians" → `1 Corinthians`, "Psalm one hundred nineteen".
  - Slovak: ordinals 1–176 in all cases and genders, pre-generated ("tretia kapitola, šestnásty verš", "v prvom
    liste Korinťanom, v trinástej kapitole"), "až" ranges.
  - Digits the ASR already formats ("John 3:16", "Ján 3,16") pass straight through.
- **Triggers:** a book name followed by numbers within ~8 words; "turn with me to …", "the book of …", "otvorte
  si …", "čítame z …".
- **State per mic** (2 min): last book + chapter, so "verse five" / "piaty verš" / "v tej istej kapitole" and
  "keep reading" / "čítame ďalej" resolve relatively (lower confidence).
- **Resolver call:** the normalised string goes to `resolver::lookup`. A guest's English reference is resolved
  against the **eng-kjv structure** (his numbering); the interpreter's against the **main (Slovak) translation**.
  Impossible chapter/verse candidates drop out (the resolver's structure filter).

### 7.2 Fusion of the two microphones (`fuse.rs`)

- Same book code + chapter + overlapping verses from both mics within 20 s → ONE card, confidence high, tagged
  "EN + SK". This is the "evaluate both transcripts" the owner asked for: the interpreter repeating the guest's
  reference is the strongest signal there is.
- One mic only → medium; a relative reference → low. Cards expire after 3 min; a card whose passage is already
  on the wall or in "Táto bohoslužba" is marked, not repeated.
- Prepared references from #851 rank first and seed the ASR `context`.

### 7.3 Versification between English and Slovak

There is no versification mapping table in the code (A+B §4). The guest cites KJV-style numbers; SEB may differ
(Mal 4:5 KJV = SEB 3:23; 154 chapters differ in verse count).

- Rule v1: fetch the Slovak text by the SAME code/chapter/verse. When that chapter's verse count differs between
  eng-kjv and the Slovak translation, or the chapter does not exist there, the card and the Bible slot say
  "číslovanie sa môže líšiť" and also show the English (KJV) text for comparison.
- A real mapping table is a later improvement (risk §16), not v1.

### 7.4 Into the console (A+B §7.4 contract)

- New `LiveEvent::BibleSuggestions { items: Vec<SuggestionDto> }` with the FULL current list (latest-value safe).
  Adding the variant breaks the one exhaustive match in `companion/variables.rs` (`live-events.md`) — a no-op arm.
- The console maps each item to `ConsoleCandidate { origin: Suggested { confidence }, label, book_code,
  chapter, verse_start, verse_end, detail: "počul som: „…turn with me to John three sixteen…“ (hosť)" }`.
- Section "Návrhy AI" (hidden while empty, A+B §13-5). A tap **previews** in the console's main translation; only
  ZOBRAZIŤ sends to the wall. Auto-show stays OFF (a later owner choice, only for fused high-confidence cards).
- A listening chip next to the on-air strip: "Počúvam ● hosť EN ● tlmočník SK"; tap → on/off sheet with the
  per-mic health.

## 8. Consumer 2 — the interpreter display

### 8.1 Page

- New top-level page **`/ui/interpreter`** in the WASM app. It is NOT an operator stage layout: like
  `camera-crew` (`stage_display.rs`, internal-only, served at `/ui/camera`) it does not follow the global stage
  layout, so the preacher's `preach` timer keeps working on the stage displays.
- Works on a TV (Android stage display via its launcher URL) or a tablet; which one = owner question §18 Q3.
- Subscribes to `/listen/ws` (snapshot of the last 20 sentences + Bible slot on connect, then events) and to the
  live hub for `BibleSlide` (what is on the wall). Reconnect + visibility resync as in A+B §6.7.

### 8.2 Layout

```
┌──────────────────────────────────────────────────────────┬───────────────────────────┐
│ HOSŤ (EN)                                                │ BIBLIA                    │
│  …and that is why Paul says we are more than        dim  │ Rimanom 8,37   (SEB)      │
│  conquerors.                                        dim  │ Ale v tomto všetkom       │
│  Turn with me to Romans eight, verse thirty-seven, │     │ slávne víťazíme skrze     │
│  and let's read it together ▌                  live tail │ toho, ktorý si nás        │
├──────────────────────────────────────────────────────────┤ zamiloval.                │
│ NÁVRH PREKLADU (SK)                                      │                           │
│  …a preto Pavol hovorí, že sme viac než víťazi.     dim  │ ČÍSLA A MENÁ              │
│  Otvorte so mnou Rimanom 8, verš 37 …       strojový     │ 8 · 37 · Paul             │
├──────────────────────────────────────────────────────────┴───────────────────────────┤
│ ● hosť EN  −18 dB   ● prepis OK  0,6 s        [EN] [EN+SK] [SK]   A− A+      19:42   │
└──────────────────────────────────────────────────────────────────────────────────────┘
```

From the CAI research already on #852 [TICKET #852 F1]: fixed positions, one scroll direction (newest at the
bottom), newest highlighted and older dimmed, a source tag on every suggestion, numbers as digits, a fixed side
slot that does not scroll away.

- **EN zone:** the current sentence word by word (final words white, the live tail lighter), 3 previous sentences
  dimmed.
- **SK zone:** the suggestion for each finished EN sentence, aligned to it; tag "strojový preklad". While a cited
  verse is being read, its SK row shows "↳ Biblia: Rimanom 8,37" instead of machine translation.
- **Bible slot:** reference (large) + the Slovak verse text **from the database**, auto-fit, up to the cited range.
  Source priority: the verse **on the wall** (`BibleSlide`, operator-confirmed) > the latest detected reference.
  It stays until replaced or 5 min pass. Numbering warning per §7.3.
- **Numbers and names:** digits and proper nouns from the last 2 sentences (they help interpreters most
  [TICKET #852 F1]).
- **Controls:** EN / EN+SK / SK, font size; kept per device (localStorage, try/catch).

### 8.3 Latency budget

| Step | Budget | Basis |
|---|---|---|
| VBAN sender packetising + LAN | ≤ 30 ms | ≤ 256 samples/packet = 5.3 ms at 48 kHz, bursts of 2–4 packets [SP] |
| Receiver framing (100 ms frames) + resampler delay | ≤ 120 ms | 100 ms frame + FFT half-block |
| Cloud ASR partial | ≤ 500 ms | vendor; measured in the bake-off |
| Server fan-out + WS + render | ≤ 50 ms | |
| **Speech → EN word on screen** | **≤ 1.0 s p90** | C-S2 |
| Sentence end → endpoint/final | ≤ 500 ms | Soniox endpoint detection finalises at once [GAP] |
| Translation (built-in tokens / LLM sentence call) | 0–1.5 s | built-in arrives with the finals; LLM measured in the bake-off |
| **Sentence end → SK suggestion** | **≤ 2.0 s p90** (ceiling 3 s) | C-S3; CAI: ≤ 2 s target, ≤ 3 s tolerable, > 4–5 s hurts [TICKET #852 F1] |
| **Reference spoken → Bible slot / operator card** | **≤ 1.5 s** | final + resolver < 1 ms + DB ~1 ms |

For comparison, songplayer's measured Live Translate figures [SP `dabing.md`]: first SK output ~3.1 s after the
input onset; SK placement latency clamped 1–6 s; the **English input transcription arrives per phrase ~4–5 s
after the audio** — too slow for the EN zone, so Live Translate can never be the EN source here.

### 8.4 When the guest cites a verse

1. "Turn with me to Romans eight, verse thirty-seven" → detector → `resolver::lookup("Romans 8:37", eng-kjv)` →
   ROM 8:37.
2. The Bible slot fetches ROM 8:37 from the Slovak translation (default = the wall's main translation, §18 Q4)
   and shows it ≤ 1.5 s after "thirty-seven".
3. The interpreter reads the Bible text instead of interpreting the guest's reading; the SK zone marks the
   guest's reading rows "↳ Biblia".
4. The operator gets the same reference as a card (fused with the interpreter's "Rimanom 8, verš 37" a few
   seconds later → high confidence). When the operator shows it, the slot switches to the on-air verse.

### 8.5 The Slovak suggestion — three candidate engines, one decided by the bake-off

| Engine | How | For | Against |
|---|---|---|---|
| **A. Soniox built-in one-way translation** (default at C3) | translation tokens in the same guest-mic stream (§6.3), paired to EN sentences by order | no extra session, no extra latency, no extra cost | no glossary/instructions documented; quality unknown on sermons |
| **B. LLM per finished sentence** | existing `ai/client.rs` `call_chat_completions_with_options` (OpenRouter), prompt = previous 3–5 EN sentences + the SK already shown + a church glossary + a neutral technical instruction (songplayer: refusals are a prompt problem [SP]); one call per sentence, no revision storm (at most one revision) | glossary, context, owner's model choice (project rule) | +0.5–1.5 s per sentence; token cost; network round trip per sentence |
| **C. Gemini Live Translate** (songplayer's production engine) | a second session on the guest mic, audio → audio, its `output_audio_transcription` = the SK text; paired to EN sentences with songplayer's content-interval overlap (`assign_en_sentences`) [SP] | proven on 36-min talks, keeps a speaker's flow, interpretation-style output | preview model; no instructions/glossary [GAP]; audio output is paid even though unused ($0.0315/min [SP]); ~3 s behind; text-only output undocumented [GAP] |

- Whatever wins, a detected verse replaces machine translation with the database text (§8.4).
- If C wins: port `ContinuousSession`'s connection logic to Rust (GoAway overlap, handle from the active connection
  only, compression 25 000/8 000, 30 s timeouts, ≤ 50 connections) with songplayer's fake-server scenarios ported as
  tests. A live stream drops the file-specific parts (1.0× pacing anchor, drain to the end, placement). Decided in
  C6, not before.

## 9. Cost and latency per service hour

| Setup | Components | ≈ $/h | Source |
|---|---|---|---|
| **Soniox only** (both mics, built-in EN→SK) | 2 × `stt-rt-v5` $0.12 + translation tokens (~9 000 words/h → ~15 k tokens × $4/1 M ≈ $0.06) + context tokens per (re)start | **≈ $0.30** | [GAP] Soniox pricing; token estimate ours |
| Soniox both mics + **LLM** per sentence | $0.24 + ~600 sentences/h × (~1.5 k in + ~60 out tokens) ≈ 0.9 M in + 40 k out tokens at the owner's model price | $0.24 + model-dependent | token volumes ours; the price follows the model the owner picks |
| Soniox both mics + **Gemini Live Translate** for SK | $0.24 + $0.037/min × 60 | **≈ $2.46** | [SP] `dubbing-eval.md` ($0.0053 in + $0.0315 out per min) |
| Bake-off reference drafts (offline) | Gemini 3.5 Transcribe batch | ≈ $0.30/h of recording | [SP] `lyrics-eval-backends.md` (≈ $0.005/min) |

- With the auto start/stop on `preach` and the silence gate, a service streams ~1–1.5 h → Soniox-only
  ≈ $0.30–0.45 per service; with Live Translate ≈ $2.5–3.7.
- Bandwidth: 2 × 256 kbit/s upstream.

## 10. Failure modes

| Failure | Detected by | Interpreter sees | Operator sees | System does |
|---|---|---|---|---|
| VBAN sender off / mic not routed | last packet > 3 s ago | banner "Nie je zvuk z mikrofónu hosťa (VBAN)" | chip red "hosť: žiadny zvuk" | session kept open on keepalive; resumes on packets |
| Packet loss / sender restart | counter gaps / resync | nothing (silence inserted) | counters in Settings health | silence for ≤ 1 s gaps; reset on big jumps; rate-limited WARN |
| Venue internet down (e.g. the 17 s tunnel drop on 2026-10-12 [TICKET #850 F2]) | WS close / connect failure | after 3 s: banner "Bez internetu — prepis pozastavený", last text dimmed | chip orange "prepis: bez spojenia" | reconnect 1/5/20/60 s; resend last ≤ 5 s; manual console unaffected |
| Provider outage / 5xx / 429 | `error_type` / close code | banner "Služba prepisu nedostupná (Soniox) — skúšam znova" | chip orange with the reason | same backoff; `Retry-After` honoured |
| Auth / payment (401/402/403) | `error_type` | banner "Prepis vypnutý — problém s účtom služby" | chip red + Settings shows the reason | stop, no retry loop; INFO + one WARN |
| Translation engine fails (LLM) | call error / timeout 3 s | EN keeps flowing; SK row "preklad nedostupný" | — | next sentence retried; built-in tokens as fallback if A is enabled |
| Wrong language on a mic (Slovak preacher on the guest mic, Czech guest) | ≥ 70 % of final tokens over 30 s tagged ≠ the mic's language (`enable_language_identification`) | banner "Mikrofón hosťa: počujem slovenčinu, nie angličtinu"; SK suggestions suppressed for Slovak tokens (shown as-is) | chip "hosť: iný jazyk — over nastavenie" | no auto-switch (the per-mic config is binding); detection keeps working (per-token normalisers, bilingual resolver) |
| Guest audio bleeding into the interpreter mic | English-tagged tokens on the SK mic | — | — | English tokens on the SK mic are ignored by the SK normaliser |
| Daily cap reached | cost meter | banner "Denný limit prepisu dosiahnutý" | chip grey | stop; manual restart allowed by the operator (raises the cap for that day) |
| Wrong suggestion / wrong verse | — | Bible slot shows the reference large; the interpreter judges | a card only, never on the wall by itself | expires after 3 min |

## 11. Privacy, retention, control

- Audio leaves the building only to the chosen ASR/translation provider, only while listening runs (§18 Q1).
- Audio is never stored unless the recorder toggle is on (§5.5, §18 Q2).
- Final sentences + suggestions go to `listen_transcript` / `listen_suggestions` tables (incremental migration)
  for tuning and the bake-off, deleted after 7 days (decided default §17-3).
- No key in the repo: keys come from `/etc/presenter/listen.env` (0600), written by the deploy workflows from
  GitHub secrets — the same pattern as `ai.env` (#761). The repo is public.

## 12. Bake-off on our own recorded SNV services

songplayer's evaluations do not settle this: its ASR numbers are on SUNG worship lyrics (batch), and its Live
Translate verdict was judged by ear on the dub audio, not on text latency or interpreter usefulness.

**Inputs.** The recorder (§5.5) captures 3–5 services with a guest: `guest.wav` (EN) + `interp.wav` (SK), 16 kHz
mono, exactly the production input. Labels:

- references actually shown: the `presenter::bible::show` INFO lines from O1 (A+B §8) give time + reference;
- references read without being shown: added by hand from the transcripts;
- reference transcripts: Gemini 3.5 Transcribe drafts with word timestamps (songplayer's
  `eval/lyrics/backends/gemini_3_5_transcribe.py`, verbatim mode), then a 10-min excerpt per service
  hand-corrected = the WER gold.

**Arms** (keys via `airuleset.py secret request` / `secret exec`, never on argv [SP practice]):

| Arm | What runs | Reuse |
|---|---|---|
| Soniox `stt-rt-v5` EN (+ built-in SK) and SK | new replay script: streams a WAV at 1.0× in 100 ms frames on one wall-clock anchor (songplayer's pacer pattern, injectable clock for tests [SP]) and logs every token with its arrival time | pattern from `dub_live_session.py` |
| Gemini Live Translate | `eval/dubbing/live_translate_continuous_probe.py --audio guest.wav --voice none` | **unchanged songplayer script** |
| LLM sentence MT | Soniox finals → one call per sentence via OpenRouter with the owner's model | prompt rules from `ai-proxy.md` |
| (only if Soniox EN WER misses the bar) another streaming EN ASR from the #852 shortlist | same replay script, second provider | [TICKET #852 F1] |

**Metrics** (state every denominator; errored items stay in it [SP scoring rules]):

- EN WER on the gold excerpts; word latency = arrival − token `end_ms` (p50/p90);
- sentence end → SK suggestion latency (p50/p90);
- SK quality: ~200 sentences, blind side-by-side, rated by the church's interpreters (1–5 adequacy/fluency) [TICKET #852 F1];
- references: recall of read references per mic and fused, lead time to the operator's show, false cards/hour;
- cost per hour from each provider's usage report.

**Decision rule (written before the run):** EN engine = lowest WER unless its word-latency p90 > 1.0 s. SK engine
= best interpreter rating whose sentence→SK p90 ≤ 2.5 s; a tie goes to the cheaper one. Detection ships when
fused recall ≥ 90 % with ≤ 2 false cards/h; otherwise the misses are classified (ASR error / normaliser / relative
reference / quote without reference) before any LLM or quote-matching step is added.

**Where it runs:** dev2, Python scripts under `scripts/dev/listen-eval/` (Tier-0 safe, no Rust compile); a small
Python VBAN sender (from the ported header layout) can also replay a recording into the dev server.

## 13. Observability

- INFO `presenter::listen::mic` on state changes (packets started/stopped, format, resync), with counters.
- INFO `presenter::listen::asr` on connect / close / error (`provider`, `mic`, `reason`, `error_type`), and one
  session summary (`seconds_streamed`, `reconnects`, `tokens_final`, `cost_estimate`).
- INFO `presenter::listen::suggest` per card (`reference`, `mics`, `confidence`, `snippet`, `shown_later`).
- WARNs rate-limited through the #484 power-of-two gate (`log-flood-backoff.md`).
- `GET /listen/status`: per-mic health, ASR state + reason, language mismatch, cost meter, last latency figures.
- A post-deploy live gate per listen provider, like songplayer's provider table [SP `docs/ai-providers.md`]:
  `docs/listen-providers.md` lists provider, model, credential, gate.

## 14. Testing

- **Decoder (core):** round-trip against the ported songplayer encoder for all 15 geometry rows + mono and stereo
  INT16 (the format the FOH sender is asked for, §5.7) + INT32; rejects: magic, non-audio sub-protocol, VBCA codec, reserved bit, size mismatch,
  SR index ≥ 21, BYTE8/FLOAT64; counter wrap at `u32::MAX`.
- **Receiver:** real `UdpSocket` loopback on `127.0.0.1:<ephemeral>` (songplayer's loopback test pattern):
  stream selection by name + IP, gap → silence count, late/duplicate drop, resync, drop-oldest on overflow.
- **Resampler:** 1 kHz tone 48 → 16 kHz keeps its frequency, exact output length per block, constant delay.
- **Soniox client:** a fake WS server (tokio) replaying recorded frames: config first, empty text frame at the
  end, keepalive during the silence gate, reconnect on 1001/413/503 with the backoff, no retry on 401/402,
  branching on `error_type`.
- **Transcript rules:** songplayer's H6 tests ported (sentence end inside a token, 20-word cap, pause never
  closes) + fixtures cut from our recordings.
- **Detectors:** a committed spoken-reference corpus (≥ 80 EN + ≥ 80 SK phrases from the recordings) like A's
  100-case corpus; fusion window and expiry with an injected clock.
- **Server integration:** VBAN packets over loopback → decode → resample → `mock` ASR records the frames
  (proves the ingest path end to end without a provider).
- **Playwright** (mock provider replaying a JSONL): `/ui/interpreter` shows EN words, an SK suggestion, and the
  SEB text of a cited verse in the Bible slot (asserted against the DB text); the console shows a "Návrhy AI"
  card, a tap previews it, 0 `POST /bible/trigger-slide` until ZOBRAZIŤ; outage banner when the mock dies;
  TV 1920×1080 and tablet profiles; zero console errors.
- **Mutation:** `/mutation-sweep` after the decoder and detector units merge; survivors there are that unit's work.

## 15. Phasing into PR units

Each PR: version bump first, ≤ 600 LoC (fixtures and generated tables excluded), CI all green, auto-merge,
deploy, post-deploy verification. New playbook rules land with their PRs (`.claude/rules/listen-vban.md`,
`listen-asr.md`, `interpreter-display.md`).

| # | Unit | Content | ≈ LoC | Depends on |
|---|---|---|---|---|
| C1a | **VBAN ingest core** | `presenter-core/src/audio/vban.rs` (ported codec + decode), `listen/vban_rx.rs`, per-mic chain + resampler, `listen.mics` config, `GET /listen/status` | 450 + 250 | — |
| C1b | **Settings + recorder** | Settings "Počúvanie" card (mic rows via the shared `ListCard`, health), recorder toggle + retention | 350 + 150 | C1a |
| — | **Owner action:** two VBAN streams from FOH → box acceptance (§5.8) → record 3–5 services | | | C1b |
| C2 | **Bake-off harness** (scripts only, runs alongside C3) | replay script, songplayer probe run, LLM arm, scorer, report → owner picks engines | scripts | recordings, keys |
| C3 | **Transcription** | provider trait, Soniox client (`tokio-tungstenite` 0.29 — today a dev-dependency for the `/live/ws` test and axum's `ws` feature; moves to `[dependencies]` with a rustls TLS feature), mock provider, session lifecycle (auto start/stop, paid gate, silence gate, reconnect, cap, cost meter), transcript assembly, `/listen/ws` hub + snapshot, retention tables | 550 + 250 | C1a, a Presenter Soniox key |
| C4 | **Verse suggestions** | detectors EN/SK + state + fusion (core), `LiveEvent::BibleSuggestions`, console "Návrhy AI" cards + listening chip | 500 + 250 | C3, A1a/A1b (resolver), B2d (sections) |
| C5 | **Interpreter display (EN + Bible)** | `/ui/interpreter`: EN zone, Bible slot (detected / on-air, DB text, numbering warning), numbers/names, status bar, controls | 500 + 200 | C3, C4 |
| — | **Milestone M-C1:** interpreter sees the guest live + verse texts; operator gets cards (S1, S2, S4–S7) | | | |
| C6 | **Slovak suggestion** | the bake-off winner (A built-in pairing / B LLM per sentence / C Live Translate port), SK zone, "↳ Biblia" substitution, EN/EN+SK/SK toggle | 300–550 + 200 | C2 verdict, C5 |
| C7 | **Hardening** | language-mismatch detection + banners, provider failover if the bake-off keeps a second engine, provider live gate, `docs/listen-providers.md`, cost summary | 300 + 150 | C6 |

**Conditional, decided by the bake-off data (filed as tickets only if the rule in §12 says so):** quote matching
without a spoken reference (exact text, then embeddings); an LLM for unclear relative references; marking the
EN sentences the interpreter already interpreted (from the SK mic).

## 16. Risks

| Risk | Mitigation |
|---|---|
| Soniox English or its built-in Slovak translation not good enough on sermon audio | provider trait from day one; the bake-off has two alternative SK engines and an EN fallback |
| Gemini Live Translate is a preview with no instructions; its text-only output is undocumented | only a candidate for the SK zone; songplayer's upgrade path = model id as a setting [SP] |
| Versification mismatch EN ↔ SK shows the wrong Slovak verse | numbering warning + English text side by side (§7.3); a mapping table later |
| Keepalive during the silence gate may lose the stream context **[unverified]** | C3 checks it on the real API; fallback = reconnect on resume (+ context resend) |
| Mic routing changes on fohabl touch critical FOH production | owner's go; `Mute`-only, read-back discipline from songplayer [SP] |
| Transcript partials flooding operator/stage sockets | separate `/listen/ws`; suggestions as one full-list event |
| Cost runaway (a forgotten toggle) | auto stop off `preach`, silence gate, daily cap, cost meter in status |
| Near-cap files (`router.rs`, `live.rs` match sites) | all logic in new `listen/` modules; size/fn gates run on every touched file |
| `tokio-tungstenite`'s TLS feature pulls new crates **[unverified]** (0.29 is already in the tree without TLS) | pick the rustls feature matching reqwest's rustls; check the lockfile diff in C3 |

## 17. Decided defaults (the owner may override any)

1. Listening starts automatically when the stage layout switches to **Kázeň (`preach`)** and stops 60 s after
   leaving it; the console toggle overrides for the rest of the day.
2. Suggestions **never** go on the wall by themselves; a tap previews, ZOBRAZIŤ shows.
3. Transcripts + suggestions kept **7 days** for tuning, then deleted; audio only with the recorder on.
4. The interpreter display is its own page `/ui/interpreter`, not in the operator's layout picker.
5. The mic feed is taken **after the mute** (a muted mic sends silence).
6. Daily cap **4 h** of streamed audio per mic; silence gate after 10 s below −50 dBFS.
7. No mic configured = nothing runs (PP stays off until set up).
8. Marking "already interpreted" sentences is not in v1.

## 18. Open owner questions (Slovak, one decision each — asked one at a time)

**Q1 — Cloud.** Presenter na serveri v kostole nemá grafickú kartu, takže prepis reči sa nedá robiť lokálne.
Počas kázne by posielal zvuk mikrofónu hosťa a tlmočníka do cloudovej služby na prepis a preklad (Soniox, pri
porovnaní aj Google).
- áno, cloud je v poriadku (odporúčam — bez toho funkcia nejde)
- nie → časť C a tlmočnícka pomoc sa nerobia

**Q2 — Nahrávky na porovnanie.** Aby sme vybrali najlepšiu službu na našom zvuku, potrebujem nahrať oba
mikrofóny na 3–5 bohoslužbách s hosťom. Nahrávky ostanú len na našom serveri a po vyhodnotení ich zmažem.
- áno, nahrávaj (odporúčam)
- nie → služba sa vyberie bez porovnania (Soniox), s väčším rizikom

**Q3 — Kde je tlmočník.** Podľa toho navrhnem veľkosť písma a zariadenie pre tlmočnícku obrazovku.
- na pódiu vedľa kazateľa, pozerá na TV/monitor na zemi z 2–3 m (odporúčam, ak to tak dnes robíte)
- pri stole alebo v kabíne, pozerá na tablet zblízka

**Q4 — Preklad Biblie pri citovanom verši.** Keď hosť cituje verš, tlmočník uvidí jeho slovenský text z našej
databázy.
- ten istý preklad, ktorý ide na stenu (dnes SEB) (odporúčam)
- iný pevne nastavený preklad len pre tlmočníka (napíš ktorý)

## 19. Owner actions (not questions)

- **VBAN streams:** on the FOH PC (VB-Matrix on fohabl, or Voicemeeter) send the guest mic and the interpreter
  mic as two VBAN streams to `10.77.9.205:6980`, 48 kHz, 16-bit, names `GUEST_EN` / `INTERP_SK`, taken after the
  channel mute. Claude can do the VB-Matrix routing instead, with the owner's go, following songplayer's FOH rule.
- **Keys for Presenter (its own, not songplayer's):** a Soniox API key (production + bake-off) and, for the bake-off
  Live Translate arm, a Gemini API key — each through the one-time secret-request page; stored as GitHub secrets
  for the deploy and in the dev2 vault for the bake-off.

## 20. Sources

**Songplayer** (clone of `zbynekdrlik/songplayer` @ dd551b7d, read-only):

- `.claude/rules/vban-out.md`, `.claude/rules/audio-outputs.md` (VBAN per destination)
- `crates/sp-server/src/playback/vban_packet.rs`, `vban_packet_tests.rs`, `vban_packet_tests_format.rs`,
  `vban_rate.rs`, `vban_out.rs` (sender socket only)
- `.claude/rules/dabing.md` (rounds D1–H, the frozen Live Translate path, subtitle rules)
- `scripts/dub_live_session.py` (`ContinuousSession`, `live_config`, constants)
- `.claude/rules/dubbing-eval.md`, `eval/dubbing/README.md`, `eval/dubbing/live_translate_continuous_probe.py`
- `.claude/rules/lyrics-eval-backends.md`, `eval/lyrics/reports/` (CHAMPION.md, report list — lyrics only)
- `.claude/rules/ai-proxy.md`, `.claude/rules/paid-ai.md`, `docs/ai-providers.md`
- `docs/superpowers/specs/2026-09-17-dubbing-feature-design.md`

**Gaps researched (small targeted lookups):**

- VBAN receive side (full SR list, sub-protocol / data type / codec values, reference receive algorithm, stack
  sizes): VB-Audio "VBAN Protocol Specifications" rev. 13, Sep 2025 —
  https://vb-audio.com/Voicemeeter/VBANProtocol_Specifications.pdf
- Soniox real-time model, endpoint, translation in real time, token fields: https://soniox.com/docs/stt/rt/real-time-translation
- Soniox WebSocket API (config fields, end-of-stream, keepalive ≤ 40 s, 300 min per stream, errors):
  https://soniox.com/docs/stt/api-reference/websocket-api
- Soniox pricing ($0.12/h real-time, translation included, token rates): https://soniox.com/pricing
- Gemini Live Translate limits (audio-only input, translation only, no tools/instructions, 24 kHz PCM output):
  https://ai.google.dev/gemini-api/docs/live-api/live-translate

**Earlier ticket research (cited, not redone):** #850 Findings 2–4 (prod forensics, N100 constraint, audio tap),
#852 Finding 1 (CAI latency targets and display principles, interpreter rating plan, EN ASR shortlist).
