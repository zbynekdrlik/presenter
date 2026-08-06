# AI-eval verification harness (#662)

Evaluates whether a candidate LLM (local, e.g. Qwen3-8B via `llama-server`, or any other
OpenAI-compatible endpoint) is "good enough" to replace `claude-opus-4-6` for the presenter AI
assistant — the tool-calling agent behind `/ui/operator`'s AI chat that authors worship
presentations and Bible-passage slides (`crates/presenter-server/src/ai/`). Full design rationale,
hardware constraints, and the serving-stack recommendation live in the research report for #662
(§1-§5); this harness is the concrete artifact for report §6.

## Status (read this first)

This PR builds the **skeleton and starter corpus only** — fixtures, schema, entrypoint scaffold,
and the judge config/rubrics. It deliberately does **not** implement the pieces that require
compiling Rust, calling a live model, or installing anything, per the constraints this PR shipped
under (a live event was running the night this was authored — no `cargo build`, no network
installs, no ssh). Concretely:

| Piece | Status |
|---|---|
| Corpus schema + starter fixtures (this README's "Corpus" section) | ✅ Done, this PR |
| `run.sh` stage scaffold (drive → score-l1 → judge → gate) | ✅ Done, this PR — every stage either does safe read-only work or fails loudly with a "not yet implemented" error citing the exact report step that builds it |
| `judge/promptfooconfig.yaml` + rubrics + trace provider | ✅ Done, this PR — structurally complete, **never run end-to-end** (no traces exist yet) |
| `crates/presenter-server/src/bin/ai_eval.rs` (the driver + Layer-1 scorer) | ❌ Not built — report §8 step 8, a separate PR |
| Golden `claude-opus-4-6` trace capture | ❌ Not run — report §8 step 9, needs step 8 first + a fresh CLIProxyAPI login |
| The remaining ~70/~90 bulk corpus cases (corpus generator + more hand-written fixtures) | ❌ Not built — report §8 steps 6-7 |
| CI wiring (`ai-eval.yml`, per-PR Layer-1-only lane) | ❌ Not built — report §8 step 11 |

Running `./run.sh` today will count the corpus (currently 30 cases) and then fail loudly at
whichever stage needs the driver — that is the intended, honest behavior of a skeleton, not a bug.

## What this evaluates, and how

Two layers, deliberately kept separate (report §6.4):

- **Layer 1 — deterministic, no model, zero flake.** Every tool-argument JSON the candidate
  emits is replayed through the REAL Rust structs `execute_tool` uses; every Bible `items[]`
  payload is replayed through the REAL `create_bible_presentation` packer
  (`compose_bible_items_into_slides`) and the REAL `bible_validator::validate_bible_slide` rules;
  verse text is diffed byte-for-byte against the canonical DB text on verbatim quotes; the
  delete-intent gate's actual behavior is checked; and tool-call sequencing is checked for sanity
  (no `create_bible_presentation` before a `load_bible_verses` for the same passage; no
  runaway iteration count). This lives in `ai_eval.rs` (not yet built) precisely so it can never
  drift from what the server actually accepts — see the report §6.2 rationale for why the driver
  is Rust, not a reimplementation in Python/JS.
- **Layer 2 — LLM-as-judge, for what structure cannot check.** Is the Slovak wording
  reconciliation faithful and natural; is slide splitting reasonable under the character limit;
  does the final chat reply read naturally in the user's language. Graded by `claude-opus-4-6`
  (fixed, version-pinned, temperature 0) against **binary checklists**, never a 1-10 score, and
  **pointwise** per candidate, never pairwise "which is better" — both choices are deliberate
  bias-avoidance calls explained in the report §6.4. See `judge/rubrics/*.md`.

`run.sh` orchestrates four stages — `drive` (call the candidate, capture traces) → `score-l1`
(deterministic Rust scoring) → `judge` (promptfoo Layer-2 pass) → `gate` (apply the pass/fail bar
below and exit non-zero on failure). Once `ai_eval.rs` exists:

```bash
./run.sh --candidate-url http://10.77.8.134:8787/v1 --model qwen3-8b
./run.sh --candidate-url http://10.77.8.134:8787/v1 --model qwen3-8b --slice bible-authoring
./run.sh --stage judge     # re-judge already-driven traces without re-driving the model
```

## Directory layout

```
scripts/dev/ai-eval/
├── README.md                     # this file
├── run.sh                        # entrypoint: drive -> score-l1 -> judge -> gate
├── .gitignore                    # traces/, report/ — never committed
├── corpus/
│   ├── SCHEMA.md                 # the case JSON schema, with the Layer-1-check mapping table
│   ├── worship-crud/*.case.json      # 7 starter cases (target: ~70 total, see "Corpus" below)
│   ├── bible-authoring/*.case.json   # 11 starter cases (target: ~100 total)
│   └── adversarial/*.case.json       # 12 cases, one per server validation rule found in the code
├── golden/                       # claude-opus-4-6 reference traces — VERSIONED in git, empty for now
├── traces/                       # candidate run output — gitignored
├── judge/
│   ├── promptfooconfig.yaml      # llm-rubric only; reads traces/, drives no model under test
│   ├── trace_provider.js         # loads a captured trace, reshapes it for the rubric assertions
│   ├── tests_from_traces.js      # generates one promptfoo test per traces/*.json file
│   └── rubrics/*.md              # binary criteria, one file per Layer-2 dimension
└── report/                       # results.json + markdown summary — gitignored
```

## Corpus

30 starter cases today, hand-written against **real data** — never invented verse text or fake
song titles:

| Slice | Starter count | Target (report §6.3) | Built from |
|---|---|---|---|
| `worship-crud` | 7 | ~70 | Real library/song names from `data/libraries/` (e.g. `SOYER`, `WONDER.pro`, `9. ráno.pro`) — the bulk still needs the corpus generator (report §8 step 6) that runs `.pro` files through `presenter-importer` for real Slovak slide text at scale |
| `bible-authoring` | 11 | ~100 | Real verses read directly out of `data/bibles/*.mybible`/`.usfm` (SQLite/USFM sources) covering every hard case named in the report: verbatim quote, paraphrase-vs-DB, overlapping/adjacent ranges, bold spanning a verse boundary, title interleaved mid-sermon, multi-translation, plus the two title conventions (`##title##` at the very start vs. `Názov:`) |
| `adversarial` | 12 | ~30 | One case per server validation rule enumerated below, hand-written |

See `corpus/SCHEMA.md` for the full case JSON schema and how each field maps to a Layer-1 check.

### Reading the raw Bible sources yourself

`data/bibles/*.zip` are NOT plain text — `rohacek`/`seb`/`milost` are zipped `mybible` SQLite
databases, `sevp` is a zipped raw SQLite3 file, and `kjv` is a zipped USFM tree. To pull a real
verse for a new fixture:

```bash
unzip -o -q data/bibles/rohacek.bbl.mybible.zip -d /tmp/roh   # -> roh.bbl.mybible (SQLite)
sqlite3 /tmp/roh/roh.bbl.mybible "SELECT * FROM Bible WHERE Book=43 AND Chapter=3 AND Verse=16;"
```

Book numbering in the `mybible`-format files (`rohacek`/`seb`/`milost`, table `Bible(Book, Chapter,
Verse, Scripture)`) is the standard 1-66 canonical order (Genesis=1 ... Malachi=39, Matthew=40 ...
Revelation=66; John=43, Romans=45, Hebrews=58, Psalms=19). `sevp` (table `verses(book_number,
chapter, verse, text)`, plus a `books(book_number, short_name, long_name)` lookup table) uses a
DIFFERENT numbering (John=500, Hebrews=650) — always resolve via its own `books` table rather than
assuming the mybible numbering carries over. `kjv.usfm.zip` is one `.usfm` file per book
(`NN-BBBeng-kjv.usfm`, e.g. `73-JHNeng-kjv.usfm` for John) with `\v N ...` verse markers and
Strong's-number word wrapping (`\w text|strong="..."\w*`) to strip by hand when quoting.

Translation codes used throughout the app (`ai/agent.rs`'s live-context list, `style_guide.md`'s
mapping table): `slk-seb`/SEB, `slk-roh`/ROH, `slk-sevp`/SEVP, `slk-mil`/MIL, `eng-kjv`/KJV — the
short code (used in `main_reference` and in `items[].translation`) is always the uppercased suffix
after the last `-`.

### Server validation rules enumerated (adversarial slice)

Found by reading `crates/presenter-server/src/ai/bible_validator.rs` (the `ValidationRule` enum)
and `crates/presenter-server/src/ai/tools/bible_presentation.rs`'s `parse_bible_items` — these are
every rule-/error-keyed response the server can send back to the LLM for self-correction on the
`create_bible_presentation` path (the only AI tool that reaches `validate_bible_slide` — there is
currently no exposed `add_bible_slide`/`update_bible_slide` AI tool, despite the validator's own
module doc comment mentioning them; see the finding below):

1. `reference_format_requires_parens` — `bible_validator.rs`
2. `missing_verse_number_prefix` — `bible_validator.rs`
3. `unprocessed_bold_markers` — `bible_validator.rs` (two cases: leaking into `main` vs. into the
   server-computed `main_reference`)
4. `empty_main_on_emphasis_slide` — `bible_validator.rs`
5. `main_exceeds_character_limit` — `bible_validator.rs`
6. `missing_items` — `bible_presentation.rs::create_bible_presentation`
7. `invalid_verse_item` — `bible_presentation.rs::parse_bible_items` (two cases: a missing
   required field, and the `u32::try_from` overflow-to-0 edge case its own code comment calls out)
8. `invalid_emphasis_item` — `bible_presentation.rs::parse_bible_items`
9. `invalid_item_kind` — `bible_presentation.rs::parse_bible_items` (two cases: a model inventing
   `kind: "reference"` for a bold section-header marker, and `kind: "title"` for a leading
   `##title##` — both should route elsewhere entirely rather than becoming an `items[]` entry)

**Finding worth flagging to whoever builds `ai_eval.rs` next:** rules 1 (`missing_verse_number_prefix`)
and 4 (`empty_main_on_emphasis_slide`) appear **unreachable** through the currently-exposed
`create_bible_presentation` tool — the server always auto-prefixes verse lines with `"N. "`
(`compose.rs`'s `format!("{number}. {text}")`) and `parse_bible_items` already rejects an
empty-text emphasis item before the composer or validator ever run. Both look like guards left
over from the pre-#236 architecture the validator's module doc describes (when the AI called
`add_bible_slide`/`update_bible_slide` directly with raw strings). Their corpus fixtures
(`adv-02`, `adv-05`) document this in full and are marked as regression-guard/canary cases rather
than true "trip the rule" cases — worth a second look (possibly a follow-up issue for dead-code
cleanup or a driver that constructs `items[]` payloads directly, bypassing a live LLM, purely to
exercise them) once the driver exists.

## How to add a case

See `corpus/SCHEMA.md` §"Adding a new case" for the full checklist. In short: pick the slice and
next sequence number, ground it in REAL data (a real verse from `data/bibles/`, a real
library/song name from `data/libraries/`), fill in `expected.notes` with the hard-case category or
rule it targets, and — once the driver + a fresh login exist — capture a golden trace for it (do
not let a case sit uncaptured, or the per-PR Layer-1 lane has nothing to regress-check it
against).

## How golden traces get captured

The `claude-opus-4-6` reference traces under `golden/` are the baseline every candidate is scored
against (report §6.3/§6.5's "no worse than 10 points below the reference"). They are captured
**once**, in one sitting, and then committed — never re-run casually:

1. **Do a fresh CLIProxyAPI login first.** The whole reason this eval architecture exists is that
   the Claude OAuth path (#597/#660) keeps expiring — see the `deploy` skill's login procedure.
   Capturing must happen immediately after a fresh login, because 200 cases × ~5-15 agent-loop
   iterations each is **~1,000-3,000 Opus calls through that exact OAuth path** (report §6.3), and
   a token that expires mid-capture leaves a partial, inconsistent golden set.
2. Once `ai_eval.rs` exists (report §8 step 8): run the drive stage against the bundled
   CLIProxyAPI endpoint with `model=claude-opus-4-6` (`DEFAULT_AI_MODEL`, `ai/mod.rs:15` — NOT
   Opus 5, not Sonnet; this must match what production actually runs) for **every** corpus case.
3. Commit the resulting trace JSON files under `golden/<caseId>.json`, one per case, reviewable in
   PR diffs. Do not commit `traces/` (candidate output — gitignored); only `golden/` is versioned.
4. Adding a new case later means capturing ONE new golden trace for it, not re-running the whole
   set — the golden set is append-only in practice, and a full re-capture is only warranted if the
   system prompt or tool schema itself changed underneath it (in which case the OLD golden traces
   are stale anyway and must be regenerated, not silently compared against a prompt they no longer
   reflect).

## Pass/fail bar (report §6.5, verbatim)

> **Qwen3-8B (or any candidate) is "good enough to replace Claude for this feature" when, over ≥150 cases with Wilson 95% CIs reported alongside every number:**
>
> 1. **≥ 98% Layer-1** on worship CRUD (schema-valid tool calls, correct delete-gate behaviour). Mechanical; any real shortfall disqualifies outright.
> 2. **≥ 90% Layer-1** on Bible authoring (schema-valid payloads, **zero verse-text corruption** on verbatim quotes, successful self-correction after ≥1 rule-keyed validation error within 3 retries).
> 3. **≥ 85% Layer-2** judge pass on the Bible rubric, majority-of-3, **with no single criterion below 75%** (stops one weak dimension hiding behind a good average).
> 4. **No worse than 10 points below the `claude-opus-4-6` reference** on the combined score.
>
> Any tier fails → not yet good enough, and the report must name *which case category* failed ("fails specifically on bold-marker-spanning-verse-boundary" is actionable; "73% overall" is not).

Deliberately **not parity with Claude** — #662 asks for "dostatočne kvalitne" (good enough), a
lower, task-specific bar. Report Wilson score intervals, never naive Wald/CLT normal
approximations — they under-cover badly at LLM-eval sample sizes and near 0%/100% pass rates
(report §6.3, citing arXiv 2503.01747).

## CI wiring (not yet built — report §8 step 11)

Mirrors this repo's existing on-demand-heavy-eval precedent (`mutation-full.yml`,
`ndi-latency.yml`):

- **Per-PR lane (cheap, deterministic, no LLM calls):** the Layer-1 structural scorer replayed
  against committed `golden/` traces — catches harness/packer/schema regressions in seconds, gates
  every PR. No model is called, so it can never flake or cost anything.
- **`ai-eval.yml` (`workflow_dispatch` + optional weekly):** the real thing — drive the candidate
  model, then the judge lane. Never a per-PR merge gate; a regression here is a signal to
  investigate, not a bar to loosen.
- Runs on the self-hosted `dev2` runner and must respect the GPU mutex against the `e2e-ndi` lane
  (report §4.2) — an eval run and `e2e-ndi` must never hold the shared RTX 5050 simultaneously.
