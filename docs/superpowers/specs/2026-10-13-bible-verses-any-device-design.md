# Bible Verses From Any Device (#850 parts A + B) — Design

**Issue:** #850 (parts A + B). Part C (live listening) is designed separately; part D is #851.
**Date:** 2026-10-13
**Status:** Design APPROVED by the owner on 2026-10-13 ("navrh je celkom dobry") with one addition — the
translation / edition choice visible on the console (§7.1). Mockup: `docs/mockups/bible-phone-and-study-v2.html`
@ cc916ea6. This written spec waits for the owner's review before the implementation plan.
**Code baseline:** `dev` @ 3b8fae37 (v0.4.316). Every `file:line` below refers to that commit.
**Legend:** **[unverified]** marks an assumption that the code or a measurement does not prove yet. Each
one says how it gets checked.

## 1. Problem

At the 2026-10-12 evening service nobody was at the PC. The owner tried to show verses from a phone
through `prsnv.newlevel.media` and gave up (prod journal 18:41–19:27):

- The burger menu never closed.
- `zid 11:1` found nothing in either box. The main translation slk-seb stores Hebrews as "Hebrejom".
  The UI's Find-book filter is a substring match over the current translation's book names, and the
  header search is full-text search over verse text. Presenter has no reference parser.
- The phone layout is a 153 px nested scroll box, inputs are 13.6 px (iOS zooms on focus), and the
  header overflows to 1109 px.
- A cold load over 4G takes 7.1–7.3 s on the LAN and 3.25 s through the tunnel.

Evidence is in #850 Findings 1–7.

Owner decisions this spec builds on (ROZHODNUTÉ, 2026-10-13):

- A + B go first through spec → approval → implementation.
- C and D are designed in parallel and plug into B2's reserved sections.
- Cloudflare Access protects both public hosts. It is applied and live: 720 h sessions, a bypass for
  the church egress IPs, and a `/healthz` bypass.
- The console shows and switches the Bible translation / edition on screen (main + the line below),
  not in a settings sheet.
- Part C (out of scope here) listens to two VBAN microphones: the guest preacher (English) and the
  interpreter (Slovak); #852 reuses the English transcript for an interpreter display.

## 2. Goal and success criteria

Whoever holds a phone during a sermon can put the right verse on the wall within a few seconds of
hearing it, with no training, and cannot send anything by accident.

| # | Criterion | Measured by |
|---|---|---|
| S1 | All 100 cases in the committed corpus produce the expected outcome (kind, book, chapter, verses, alternatives order, hint). The 17 incident spellings of `zid 11:1` → HEB | `presenter-core` unit test (CI `test` job) |
| S2 | `Židom` / `zid` / `Hebr` / `Hebrews` / `HEB` on slk-seb work through `GET /bible/passage`, legacy `POST /bible/trigger`, Companion `bible.trigger`, and the AI tools `get_bible_passage`, `load_bible_verses`, `resolve_bible_slides`, `trigger_bible_verse` | server router/state tests |
| S3 | A verse fired via Companion/AI/legacy reaches `/stage`, `/ui/camera` and stream outputs: `LiveEvent::BibleSlide` is published and `/bible/active-slide` is non-empty | state test + Playwright stage check |
| S4 | Phone profiles 360×800, 390×844, 412×915 (portrait) and 844×390 (landscape), all with `hasTouch`: 0 visible elements past the right edge in all 5 sections; every input/select/textarea ≥ 16 px computed font size; every console control ≥ 44×44 CSS px | Playwright `phone-shell.spec.ts`, `bible-console.spec.ts` |
| S5 | A touch drag of ≥ 10 px that starts on a Worship card sends 0 `POST /stage/state`. A stage-layout change from a phone needs a confirm | Playwright (CDP touch events) |
| S6 | Phone, console open: `zid 11 1` → on the wall in ≤ 1 field + 2 taps (the design needs 1 tap, because the preview loads while typing). Next verse = 1 tap | Playwright counts the interactions |
| S7 | Desktop: type → Enter → Enter shows the verse. ←/→ step through verses | Playwright |
| S8 | Cold open on a phone at 9/1.5 Mbps, 150 ms RTT, CPU×4: console usable in ≤ 3.0 s through the public host and ≤ 3.5 s on the LAN (today 3.25 s / 7.1–7.3 s) | on-demand `@perf-phone` spec on dev + post-deploy run against prod (audit method) |
| S9 | Exactly 1 `.wasm` download per phone page load; served with `Content-Encoding: br`; raw and br sizes within the committed budget | CI `build` step + Playwright |
| S10 | Every lookup and show (console, legacy API, Companion, AI) logs one INFO line. The prod journal shows HTTP request lines (method, path, status, ms) | `journalctl -u presenter` after each deploy |
| S11 | Zero browser console errors and warnings in every new or changed spec | the specs' final assertion |

## 3. Scope

**In scope:**

- **A.** One reference resolver in `presenter-core`, used by the WASM UI and by every server entry
  point. Also: the `GET /bible/lookup` endpoint, the legacy trigger routed through the trigger-slide
  path, and the full-text fallback fix.
- **B1.** The operator shell on phones, all 5 sections: navigation, header, inputs, touch safety,
  payload, and lossless live events.
- **B2.** The Bible "Live" tab rebuilt as the verse console, one design from phone to desktop, with
  reserved sections for #851 and C.
- **Observability.** HTTP request logging plus INFO lines per lookup and per show.
- **Access.** Commit the Cloudflare Access apply script under `ops/cloudflare/`.

**Out of scope:**

- Part C (ASR, detectors) and #851 (study and sermon prep). This spec defines only the console slot
  they fill.
- `/ui/tablet` and `/ui/stream` tap-target redesigns. The global 16 px input rule (§6.4) still applies
  to them. See §14.
- Bible data quality and licensing (#851 track).
- App-level login.
- Lists and cross-chapter ranges as ONE show request. They become several result rows instead (§5.7).

## 4. Verified constraints (from the code)

**Build and code rules**

- Tier-0: no local cargo compilation; CI is the compiler.
- File cap ≤ 1000 lines (warn 800); function cap ≤ 120 lines (warn 80).
- No `unwrap` / `expect` / `panic!` in server or core production code. WASM is exempt.
- Files this work touches that are near the cap: `pages/bible.rs` 794, `pages/bible_slides.rs` 789,
  `pages/operator.rs` 786, `components/slide_list.rs` 919, server `state/bible.rs` 760,
  `ai/bible_validator.rs` 759. **All new code goes into new files.**

**presenter-core and the WASM bundle**

- `presenter-core` is already a dependency of `presenter-ui`.
- `unicode-normalization` is already compiled into the WASM (via `normalise_book_key`,
  `bible/search.rs:3-11`).
- The `canonical.rs` alias table is NOT in today's WASM. The UI never calls it, so it is
  dead-code-eliminated (grep of the shipped `.wasm`).
- The name `bible::reference` is taken: `bible/reference.rs` holds `BibleReference`. The new module is
  therefore `bible::resolver`.

**Versification**

- There is no versification table in the code. Per-chapter verse counts are `MAX(verse_end)` per
  (book, chapter) of the imported rows:
  - `repository/bible/query.rs:282-322` `bible_book_chapter_summaries`
  - wrapped by `AppState::bible_book_chapter_summaries` (`state/bible_source.rs:128-140`; the NLT uses
    the eng-kjv structure + `bible_remote::nlt_structure`)
  - served as `GET /bible/books` (`router/bible/browse.rs:37-78`)
  - held in the UI as `BibleBookDto.chapters[].verse_count` → `SelectedBook.verse_counts`
    (`state/bible.rs:9-15`)
- Versification differs per translation: Joel has 3 or 4 chapters, Malachi 4 or 3, and 154 chapters
  differ in verse count.
- The book list is ordered by stored NAME (`query.rs:310` `order_by_asc(Book)`), not in canonical
  order.
- `verse_end = None` means the whole chapter on `/bible/resolve`, but a single verse on broadcast and
  browse (#702).

**Logging**

- The server has **no HTTP request logging layer**.
  - tower-http's `trace` feature is compiled in (`Cargo.toml:41`), but `build_router`
    (`router.rs:41-442`) installs no `TraceLayer`, and `git log -S TraceLayer` finds none ever.
  - Removing the `ableset-debug.conf` drop-in alone therefore brings back **nothing**.

**Live events and keyboard**

- The operator WS coalesces events into one latest-value signal (`ws/mod.rs:40-50`).
- The hub forwards every event, including the 1.5 s `Heartbeat` (`state/background_tasks.rs:29`,
  `live.rs:101-124`), so a Bible event can be overwritten before it is handled.
- The reconnect resync (`operator.rs:91-130`) does not re-read the Bible state.
- The global keydown handler (`operator.rs:656-709`) is not scoped to a view: in Live mode ←/→ move
  Worship slides even on the Bible view, and Space focuses the header search.

**Stage preview**

- The stage-preview iframe sits in `<Show when=is_worship>` (`stage_preview.rs:145,166-176`). CSS hides
  it at ≤ 768 px (`operator.css:2485`), but it stays mounted, so the app loads twice.

**Repo, tests and integrations**

- The repo is **PUBLIC**. No e-mails, egress IPs or Cloudflare account ids may be committed.
- E2E servers ingest KJV, SEB, ROH and SEVP (`tests/e2e/support.ts:284-292`). Real-data E2E is
  therefore possible and needs no "no Bible data" skips.
- There is no per-PR mutation gate. `Cargo.toml:22-26` says so, and `mutation-full.yml` is
  `workflow_dispatch` only.
- Companion `BibleTriggerPayload` is `deny_unknown_fields` (`companion/protocol.rs:364-373`). Fixes
  must be server-side, with no new payload fields.
- `presenter-ui` web-sys features lack `MediaQueryList` and `VisualViewport`. They must be added.

## 5. Part A — one reference resolver

**Approach (chosen):** our own small resolver in `presenter-core`. It is compiled into the server and
the WASM, so the UI resolves locally with zero round trips.

Rejected alternatives:

- The openbibleinfo JS parser via interop: UI-only, JS inside a WASM app, and the server would not
  share it.
- LLM parsing: takes seconds, needs the network, and is non-deterministic.
- `topos-bible`: young, English-only.

### 5.1 Module and public API

New directory `crates/presenter-core/src/bible/resolver/`. `bible/mod.rs` adds `pub mod resolver;`
(like `pub mod search;`). Consumers import `presenter_core::bible::resolver::…`, so no root `pub use`
is needed.

| File | Content |
|---|---|
| `mod.rs` | API below |
| `types.rs` | `ParsedQuery`, `BookCandidate`, `Segment`, `Passage`, `BookRef`, `Resolution`, `MatchKind`, `Tier` |
| `fold.rs` | `fold_book_text(&str) -> FoldedBook { ordinal: Option<u8>, key: String }` |
| `grammar.rs` | hand-written tokenizer + chapter/verse grammar (no `regex` dependency) |
| `rank.rs` | candidate scoring, `SHORT_DEFAULTS`, bounded Damerau-Levenshtein |
| `versification.rs` | `Versification`, `BookStructure`, verse stepping |
| `hint.rs` | Slovak hints. `counted()` moves here from `presenter-ui/src/state/bible_range.rs:141` |
| `names_table.rs` | `@generated` (§5.2) |
| `tests.rs`, `testdata/` | corpus + structure fixtures (§10) |

```rust
pub fn parse(input: &str) -> ParsedQuery;                       // pure text → book candidates + raw segments + leftover words
pub fn resolve(q: &ParsedQuery, s: &Versification) -> Resolution; // bind to the MAIN translation's structure
pub fn lookup(input: &str, s: &Versification) -> Resolution;    // parse + resolve
pub fn resolve_book_name(name: &str) -> Option<BookCandidate>;  // name only, no structure (server choke point §5.8)
pub fn resolve_translation_code(input: &str, installed: &[&str]) -> Option<String>; // "KJV"→"eng-kjv", "seb"→"slk-seb"

pub struct Segment { pub chapter: u16, pub verse_start: Option<u16>, pub verse_end: Option<u16> } // both None = whole chapter
pub struct Passage { pub book: BookRef, pub matched_form: String, pub segments: Vec<Segment> }
pub struct BookRef { pub code: String, pub number: u16, pub name: String /* stored name in the main translation */ }
pub enum Resolution {
    Passage { passage: Passage, alternatives: Vec<Passage> },        // alternatives = other books that fit, best first
    Ambiguous { default: Passage, alternatives: Vec<Passage> },      // default is a guess of equal (tier, kind)
    OutOfRange { book: BookRef, requested: Segment, hint: String, fix: Option<Passage> },
    BookOnly { book: BookRef, alternatives: Vec<BookRef> },          // no numbers typed
    Incomplete { ordinal: u8, books: Vec<BookRef> },                 // "1", "I."
    NoBook { segments: Vec<Segment>, hint: Option<String> },         // "11:1" → "Napíš aj knihu"; words only → text search
}
impl Versification {
    pub fn new(books: Vec<BookStructure>) -> Self;
    pub fn from_chapter_summaries(rows: &[BibleBookChapterSummary]) -> Self; // server
    pub fn step(&self, code: &str, chapter: u16, verse: u16, dir: Step) -> Option<(String, u16, u16)>; // crosses chapter + book bounds
}
pub struct BookStructure { pub code: String, pub number: u16, pub name: String, pub verse_counts: Vec<u16> }
```

The stored book names in the `Versification` are matched at runtime as T1 keys too. A future
translation's own names therefore work without regenerating the table.

### 5.2 Name table: sources, generator, data fixes

**Committed sources:** `data/bible-book-names/`

- `bprp-slk.yaml`, `bprp-ces.yaml`, `bprp-eng.yaml`: openbibleinfo Bible-Passage-Reference-Parser-
  Languages, MIT, pinned commit; its LICENSE is copied in.
- `biblia-sk.json`: abbreviation + title per system for seb / sep / ssv / roh, from the biblia.sk
  reader menus.
- `presenter-stored.json`: the stored names of the 6 installed translations, snapshot of
  `GET /bible/books`.
- `curated.yaml` with these keys:
  - `short_defaults` (keys ≤ 2 letters → ranked codes, Finding 6 table)
  - `no_fold` (`P.Š.`, `PŠ`)
  - `remove` (`Ab`→OBA)
  - `add` (`Mr`→MRK at T1; keep SEB's `Filimonovi`)
  - `frequency_rank` (66 codes, from OpenBible.info cross-reference votes, CC-BY → attribution)
- The SEB / ROH / SEVP arrays are read straight from `canonical.rs` (`SLOVAK_*_NAMES`). The generator
  fails loudly when an array is not exactly 66 entries.

**Generator:** `scripts/dev/gen_bible_book_names.py`, Python, so Tier-0-safe. It starts from the
review's prototype `build_inventory.py` (scratchpad).

- It writes `bible/resolver/names_table.rs` with the header `// @generated by
  scripts/dev/gen_bible_book_names.py — do not edit`.
- The table is a key-sorted `static BOOK_KEYS: &[(&str /*folded key*/, u8 /*book idx*/, u8
  /*tier<<4 | ordinal*/)]`, so exact lookup is a binary search and prefix lookup uses
  `partition_point`.
- It also writes `NO_FOLD`, `SHORT_DEFAULTS` and `FREQUENCY_RANK: [u8; 66]`.
- Size: ≈ 770 keys, ≈ 5–13 KiB in the WASM + 10–30 KiB of code.
- `--check` regenerates into memory and diffs. The CI `quality` job runs `python3
  scripts/dev/gen_bible_book_names.py --check` and `python3 -m unittest
  scripts/dev/test_gen_bible_book_names.py` (the `scripts/ci/check_*.py` pattern).

**Data fixes:**

- Remove `("Ab", "OBA")` from `canonical.rs:610`. "Ab" = Habakkuk in SEP and BPRP; Obadiah is
  "Abd" / "Ob.".
- `canonical_book_by_name` stays exact and is used only by the importer (`import.rs:190`). Fuzzy
  matching must never assign codes at ingest.

**Attribution:** a new `THIRD_PARTY_NOTICES.md` (BPRP MIT, OpenBible.info CC-BY).

### 5.3 Folding (`fold.rs`)

1. NFD, drop combining marks, lowercase.
2. A leading ordinal becomes a digit: `1`, `1.`, `I`, `I.`, `prvý/prvá/první (kniha|list)`, `1 k.` →
   `1`; the same for 2–5.
3. Drop filler words: kniha, list, epištola, evanjelium, podľa, svätého, apoštola, Pavla, proroka.
4. Keep `[a-z0-9]` only. The key is (ordinal, name).
5. `NO_FOLD` forms are matched before folding.

After folding there are only 3 real collisions: `ab` → HAB, `1k` → 1CO, `2k` → 2CO. `ps` is solved by
the no-fold list.

### 5.4 Grammar (`grammar.rs`)

- **Book/number split.** A leading 1–3 followed by a letter is an ordinal (`1kor13:4`). The book text
  runs to the first digit. A trailing `.` is stripped (`Rim.8,25`).
- **Chapter–verse separator precedence:**
  1. `:`, and then `,` / `.` separate list items
  2. else `,`, and then `.` separates list items
  3. else the first `.`
  4. else a space between two numbers (`11 1`, `1 kor 13 4-7`)
- **Ranges:** `-`, `–`, `—`, `až` / `az`, with spaces allowed around them.
- **Cross-chapter:** `1:1-2:3` / `1,1-2,3`. **Chapter range:** `23-24`. **Lists:** `5:3.5.7`,
  `3:16,18`.
- **Single-chapter books** (OBA, PHM, 2JN, 3JN, JUD): a lone `n > 1` is verse n (`jud 5`); `jud 1` is
  the whole book.
- **Comma meaning.** Here `,` means chapter:verse. `ai/bible_validator.rs:204 normalize_reference`
  (AI slide labels, where `,` = a verse list) is **not** migrated: it resolves no books, and sharing
  the tokenizer would mix up the two meanings.

### 5.5 Ranking and ambiguity (`rank.rs`)

**Tiers**

- T1: Slovak (db names, biblia.sk systems, canonical Slovak arrays and abbreviations)
- T2: BPRP slk, English, OSIS
- T3: Czech, BPRP English short forms

**Match kinds**

1. exact
2. ordinal-stripped exact (`kr` = "1 Kr" family)
3. prefix
4. fuzzy

Candidates compare lexicographically by (tier, kind), then by `FREQUENCY_RANK`.

**Rules**

- **Fuzzy** = Damerau-Levenshtein ≤ 1 at a length of 4–6 and ≤ 2 at 7+. It is used only when nothing
  else matched, and never for keys shorter than 4.
- **Keys ≤ 2 letters** go through `SHORT_DEFAULTS` first (e.g. `z` → PSA; `j` → JHN; `jo` → JOB; `g` →
  GEN, alternative GAL). This fixes the 14 sk/cs forms that pure scoring maps to the wrong book.
- **No ordinal typed and only numbered books match:** pick the family's ordinal-1 book. The others
  become alternatives.
- **The structure filter runs last** (§5.6): candidates whose chapter/verse does not exist are
  dropped. This is the best disambiguator (`pt 5:7` → 1PE, `ab 2:4` → HAB, `fil 4:13` → PHP, `z 119` →
  PSA).
- **Ambiguous vs Passage.** `Ambiguous` means the default ties on (tier, kind) with another fitting
  book (`kor 13`, `tim 2:15`). Otherwise the result is `Passage`, and the fitting lower-ranked books
  still go into `alternatives` (`jan 3 16` → Ján, alternative 1 Ján, as in the mockup).

### 5.6 Validation against the main translation's versification

- **UI:** `Versification::new` is built from `BibleState.books` (the `/bible/books` response the page
  already loads for the main translation).
- **Server:** `Versification::from_chapter_summaries(state.bible_book_chapter_summaries(main))`, about
  12 ms per call (measured). There is no cache in v1; add one only if lookup latency shows up in
  logs.
- **Out-of-range hints** use the existing wording, which `bible_range.rs:27,60` already uses and the
  corpus expects:
  - "Kniha má len 13 kapitol" / "Kapitola má len 40 veršov"
  - plus a `fix` passage at the last chapter or verse ("Hebrejom 13", "Hebrejom 11:40")
  - The mockup's "Židom má 13 kapitol" is reworded on purpose, to match the existing UI.

### 5.7 Resolution kinds → corpus outcome → console behaviour

| Kind | Corpus outcome | Console |
|---|---|---|
| `Passage` (1 segment) | `resolved`, and `book_only` when a chapter is given (`jud 1`) | default row, preview loads, alternative chips ("Alebo") |
| `Passage` (n segments) | `list` (`mt 5:3.5.7` → 3 segments), `cross_chapter` (`gn 1:1-2:3` → 1:1-31 + 2:1-3) | one row per segment; ▶ walks them; each show is one contiguous request |
| `Ambiguous` | `ambiguous` | default row marked "predvolené" + alternative chips |
| `OutOfRange` | `out_of_range` | hint + fix chip; no preview |
| `BookOnly` | `book_only` without a chapter (`jan`) | row "Ján — dopíš kapitolu"; a tap completes the field to `Ján ` |
| `Incomplete` | `incomplete` (`1`) | chips of the numbered books |
| `NoBook` | `no_book` (`11:1`) | hint "Napíš aj knihu"; for words only: a text-search row (§5.11) |

### 5.8 Server

**Choke point.**

- `AppState::find_bible_passage` and `AppState::bible_passage_range` (`state/bible_source.rs:61-115`)
  resolve `book` through `resolver::resolve_book_name` whenever `book_code` is `None` or blank, then
  pass the code to the repository.
- `canonical_book()` (`bible_source.rs:206-211`, used by the NLT and the secondary lookup) uses the
  resolver in its name branch.
- This one change fixes every server path that sends a typed name, without per-site edits.
- When the typed name is not the translation's stored name, one INFO line is logged:
  `presenter::bible::name` with `name`, `code`, `match`, `caller`.

**Translation alias.**

- New `AppState::resolve_translation(input) -> anyhow::Result<String>` in `state/bible_lookup.rs`:
  exact code, else `resolve_translation_code` against the installed codes, else
  `RepositoryError::NotFound` → 404.
- Used by the legacy trigger, Companion, `GET /bible/passage`, `GET /bible/lookup` and the AI tools.
- Fixes Companion's default `"KJV"` (`ops/companion/presenter/index.js:439,715`).

**`GET /bible/lookup?q=&translation=`** (`router/bible/lookup.rs`, route next to `router.rs:85`)

- `translation` defaults to the main preference.
- Returns 200 with the resolution; 400 for an empty `q`; 404 for an unknown translation.
- Logs one `presenter::bible::lookup` INFO line.
- Example response:

```json
{ "query": "zid 11 1", "translation": "slk-seb", "kind": "passage",
  "default": { "bookCode": "HEB", "bookNumber": 58, "book": "Hebrejom", "matched": "Žid",
               "segments": [{ "chapter": 11, "verseStart": 1, "verseEnd": 1 }], "label": "Hebrejom 11:1" },
  "alternatives": [], "hint": null, "fix": null }
```

### 5.9 Call sites

| # | Site (file:line) | Today | Change |
|---|---|---|---|
| 1 | UI book keys + filter `presenter-ui/src/state/bible.rs:87-95, 136-157` | substring over the main translation's names | **removed** in B2c; the console calls `resolver::lookup` |
| 2 | UI `BookFilter` `pages/bible.rs:476-552` (Enter `:493`, `filtered_books().first()` `:502`) | alphabetical first match | removed (B2c) |
| 3 | UI `BookList` `pages/bible.rs:554-660` (click `:616`) | name-ordered list | removed (B2c) |
| 4 | UI number inputs `pages/bible_reference.rs:220,238,256` (+ `state/bible_range.rs`) | 3 `type=number` fields | removed (B2c); out-of-range = `Resolution::OutOfRange` |
| 5 | Header Bible search `components/header.rs:91-143` (`use_context` None `:118`, call `:124`) | FTS with an empty translation | removed on the Bible view (B2c); the console field does reference + text search |
| 6 | Search result click `components/search.rs:212-238` (`use_context` None `:216`) | dead click | removed (B2c) |
| 7 | `GET /bible/passage` `router/bible/browse.rs:122-135` | exact stored name | choke point + translation alias (A2a) |
| 8 | Legacy `POST /bible/trigger` `router/bible/broadcast.rs:59-95` → `state/bible.rs:450-481` | exact name, legacy events only | choke point + rerouted (§5.10) |
| 9 | Companion `bible.trigger` `companion/protocol.rs:146-157`, run `:466-479` | exact name; "KJV" never matches | choke point + translation alias; payload unchanged |
| 10 | AI `get_bible_passage` `ai/tools/bible.rs:32-73` (`book_code: None` `:47`) | exact name | choke point |
| 11 | AI `load_bible_verses` `ai/tools/bible.rs:91-193` (`canonical_book_by_name` `:129`) | exact alias | `resolver::resolve_book_name` |
| 12 | AI `resolve_bible_slides` `ai/tools/bible.rs:195-236` | exact name; 0 slides on a miss | choke point (via `generate_bible_slides` → `bible_passage_range`) |
| 13 | AI `trigger_bible_verse` `ai/tools/bible.rs:238-266` (`:253`) | exact name, legacy path | choke point + reroute |
| 14 | AI tool schema `ai/tool_defs.rs:168,193,208,241` | "Full book name (e.g. Židom)" fails on SEB | "Any book form (Židom, Hebr, Heb 11 …)" |
| 15 | AI style guide `ai/style_guide.md:57-91` | false claim at `:91`, names the non-existent tool `find_bible_passage`, wrong names | rewritten from the generated table's T1 forms |
| 16 | `ai/bible_validator.rs:204` `normalize_reference` | label grammar | **unchanged** (§5.4) |
| 17 | Secondary + NLT `state/bible.rs:744-749`, `state/bible_source.rs:206-211` | exact alias | resolver in `canonical_book` |
| 18 | Importer `persistence/src/repository/bible/import.rs:190` | exact alias | **unchanged** |
| + | `repository/bible/query.rs:310` | order by name | `order_by_asc(BookNumber)` (canonical order) |

### 5.10 Legacy trigger through the trigger-slide path (A2b)

`trigger_bible_passage` (`state/bible.rs:450-481`) keeps computing the main passage
(`trigger_main_passage`) and the secondary text (`trigger_secondary_text`). Then:

1. It builds a `BibleSlideOutput` in the new `state/bible_labels.rs`:
   - `main_text` = the numbered verse text (unchanged)
   - `main_reference` = `"{stored book} {c}:{v[-v]} ({SHORT})"`, using the stored name from the first
     passage row, so Companion `"John"` on SEB shows "Ján 3:16 (SEB)"
   - `secondary_reference` = today's `legacy_translation_reference`, moved from
     `resolume/legacy_reference.rs` to keep the #824 secondary-book swap
2. It calls `trigger_bible_slide_output(output, metadata, source)`. That publishes `Bible` +
   `BibleSlide`, stores `slide_output` for `/bible/active-slide`, and drives Resolume through
   `from_slide_output`.
3. It returns the stored legacy `BibleBroadcast`. The `/bible/trigger` and Companion `apply_bible`
   contracts stay unchanged.

**Removed:**

- `HostDriver::handle_bible_legacy` (`resolume/handlers.rs`, the `else if let Some(ref passage)`
  branch at `:375` and the function at `:493`)
- the `BibleUpdate` fields `passage` / `secondary_text` / `secondary_translation_code` /
  `secondary_book`
- `resolume/legacy_reference.rs`

**Test literals to update** (grep every `BibleUpdate {`):

- `resolume/tests.rs:748,805,854`
- `legacy_bible_tests.rs:64`
- `bible_clear_tests.rs:248-252`
- `provisional_schedule_tests.rs:236`

**[unverified]**

- That `handle_bible_slide_output` writes the four lanes identically to the legacy branch for empty
  secondary text. Check: read both before deleting; pin it with a Resolume driver test.
- That Companion variables stay consistent when a legacy trigger now emits both `Bible` and
  `BibleSlide` (`companion/variables.rs:44-46`). Check: a variables unit test.

### 5.11 Full-text fallback fix

- `build_fts_query` (`query.rs:20-36`):
  - split on every non-alphanumeric character, so `11:1` becomes `11`, `1`
  - drop tokens that are all digits (no more `111*`)
  - measure length with `chars().count() >= 2` (today `w.len()` counts bytes, so `Ž` passes)
- The console asks `GET /bible/search?translation=<main>&query=…&limit=10` only:
  - for `NoBook`, or
  - when the input has ≥ 2 words, ≥ 3 letters and no exact or prefix book match.
- Each hit is labelled with the translation's short code. A tap previews the hit; it never goes live
  directly.

## 6. Part B1 — the operator shell on phones

### 6.1 Phone shell condition

- One constant: `PHONE_SHELL_QUERY = "(max-width: 768px), (pointer: coarse) and (max-height: 500px)"`
  in `state/phone_shell.rs`. The second clause covers 844×390 landscape phones, which today fall into
  the desktop layout.
- A `MediaQueryList` change listener drives `is_phone_shell: Signal<bool>` and sets
  `body[data-shell="phone|desktop"]`.
- CSS keys on `[data-shell]`, so there is one source of truth.
- New stylesheet: `styles/phone_shell.css`, linked in `index.html`.

### 6.2 Navigation

- New `components/phone_tab_bar.rs` with Piesne · Biblia · Časovače · AI · Nastavenia (labels: open
  point §13-1).
  - fixed at the bottom; 56 px + `env(safe-area-inset-bottom)`
  - items ≥ 48 px tall
  - `<nav aria-label="Sekcie">` with `aria-current="page"`
  - In landscape it becomes a 64 px left rail.
- `set_view` (`header.rs:157-173`) moves to `state/view_nav.rs` and is shared by the header and the tab
  bar.
- The burger is deleted: `OperatorState.mobile_nav_open` (`state/operator.rs:80,203`), the toggle
  (`header.rs:192-195`), the class sync (`operator.rs:54-59`), `.operator__hamburger`, and the overlay
  CSS (`operator.css:2431-2471`). This removes the stuck-menu bug class entirely.

### 6.3 Compact header + "Viac" sheet

**Header**

- Portrait header ≤ 56 px: "Presenter" + a WS connection dot on the left; the Live/Edit toggle (always
  visible, 44 px) and "Viac" on the right.
- On Worship, song search is an icon that expands the field to full width. On Bible there is no header
  search; the console field takes its place.
- Landscape: one 44 px row.

**`components/more_sheet.rs`** (bottom sheet; closes on ✕, backdrop tap, Escape and Back) holds:

- the `SurfaceNav` links (new tabs, as today)
- the status chips (Resolume, AI, version), in their own top group, keeping the #573 rule "status
  chips never sit next to Stage Output"
- Stage Output
- `.operator__brand-nav` (`operator.css:3198-3205`) is not rendered in the phone header.

**Stage Output from a phone** opens a confirm step ("Prepnúť stage na Kázeň?" Prepnúť / Zrušiť).
`POST /stage/layout` (`header.rs:183-190`) fires only after the confirm.

### 6.4 Inputs and tap targets

- `@media (pointer: coarse) { input, select, textarea { font-size: max(16px, 1em); } }` in
  `phone_shell.css`. It loads on every surface, so tablet, stream editor and settings are covered too.
- Viewport meta (`index.html:5`): `width=device-width, initial-scale=1, viewport-fit=cover,
  interactive-widget=resizes-content`. Never `maximum-scale` or `user-scalable=no`.
- ≥ 44 px applies to: tab bar, header, sheet, console, and the Worship slide toolbar (columns ±
  26×20, add-slide 24×21 today).
- `.operator__timer-actions { flex-wrap: wrap }` (`operator.css:1667-1670`; the Preach card overflows
  to 474 px today).

### 6.5 Touch-safe triggers

- `components/slide_list.rs:455-478`: the immediate `pointerdown` trigger runs only when
  `ev.pointer_type() == "mouse"`.
- Touch and pen use the existing `on:click` path (`:419-453`) with a guard: cancel when the pointer
  moved > 10 px since `pointerdown` (position kept in a per-card `Cell`).
- Browsers already suppress `click` after a scroll; the guard is a second line of defence.
- The mouse keeps today's latency and its 250 ms dedupe.

### 6.6 Payload

1. **wasm-opt.**
   - `index.html:19`: `data-wasm-opt="z"` + `data-wasm-opt-params="--mvp-features
     --enable-mutable-globals"`, matching the Safari-12 target in `.cargo/config.toml`.
   - presenter-ui `[profile.release] opt-level = "z"`, `lto = true`, `codegen-units = 1`.
   - Pick the smallest variant whose E2E is green.
   - **[unverified]** that Trunk supports `data-wasm-opt-params` and fetches wasm-opt on the runner (the
     `Trunk.toml` comment says it "can fail on some runners"). Fallback: run `wasm-opt` explicitly in
     `scripts/build-ui.sh`.
   - **[unverified]** the size gain; measure in the PR.
2. **Precompressed assets.**
   - A new step in `scripts/build-ui.sh` writes `brotli -q 11` and `gzip -9 -n` siblings for
     `dist/*.{wasm,js,css}`.
   - `wasm_ui_asset` (`router/wasm_ui.rs:28-46`) serves `.br` → `.gz` → identity according to
     `Accept-Encoding`, with `Content-Encoding`, `Vary: Accept-Encoding` and the original
     `Content-Type`.
   - Rejected: a runtime `CompressionLayer`. It would re-compress 5.8 MB per cold load on the N100,
     and the `compression-*` features are off.
   - **[unverified]** that the `brotli` CLI is installable on the runner (apt `brotli`).
   - Binary growth ≈ +3.5 MB (projection).
3. **No second app on phones.** In `stage_preview.rs:145`, `is_worship` becomes
   `is_worship && !is_phone_shell`.
4. **Safari-12 constraints kept.**
   - No preload hints (`build-ui.sh:47-48`).
   - CI validates MVP features: `wasm-tools validate --features mvp,mutable-global` on the dist
     `.wasm`.
   - **[unverified]** the exact flag names; the alternative is a `wasm-objdump` feature scan.
5. **Size budget.**
   - A `build`-job step checks raw and `.br` sizes against `scripts/ci/ui_size_budget.json` (budget =
     the first measurement + 5 %), with a unittest per the `scripts/ci/` pattern.
   - Projection for S8 **[unverified]**: the tunnel at 1.56 MiB br took 3.25 s; wasm-opt + br from the
     origin should bring the LAN under 3.5 s.

### 6.7 Live WebSocket: lossless events + resync

- **Per-event handler.** `ws/mod.rs` becomes `use_live_websocket(surface, on_event: impl Fn(LiveEvent)
  + 'static) -> ReadSignal<WsState>`.
  - Each frame is handled synchronously in arrival order, the stage's #793 pattern.
  - Callers to update: `pages/operator.rs:81` (+ `setup_ws_dispatch`) and `pages/tablet.rs:53`.
  - The operator gains a `LiveEvent::BibleSlide` arm that feeds `ctx.active_bible_slide`, which the
    on-air strip reads.
- **Liveness.**
  - The hub's 1.5 s heartbeat reaches every socket.
  - A 1 s check closes and reconnects a socket that is "Connected" but silent for > 5 s (3 missed
    heartbeats).
  - When `visibilitychange` turns visible, the same check runs at once; after > 30 s hidden it
    reconnects unconditionally. This is the iOS "WebSocket is closed due to suspension" case.
- **Resync on every reconnect** (extends `operator.rs:91-130`):
  - adds `GET /bible/active-slide`, `GET /bible/active` and the stage layout
  - a generation guard drops a fetch answer when a live event arrived while the fetch was in flight
    (the `StageSyncGeneration` idea)

## 7. Part B2 — the verse console

### 7.1 Layout

**Phone** (mockup screens 1–4), top to bottom:

1. **On-air strip** `data-role="console-on-air"`: "Na stene: nič" (grey) / "Na stene: Hebrejom 11:1"
   (red), with the translations of what is on the wall ("SEB + NLT") on the right.
1b. **Translation bar** `console-translations` (owner addition, mockup v2), always visible under the strip:
   - row "Preklad" = chips for the installed full translations, Slovak first (SEB · ROH · SEVP · KJV);
     a partial translation (MIL, Gospels only) sits behind a "…" chip; NLT is never a main option (remote,
     #826)
   - row "Pod ním" = NLT · KJV · žiadny (the secondary line on the wall)
   - one tap switches; the current preview re-resolves at once in the new translation, and the book name
     follows the translation (Hebrejom in SEB, Židom in ROH); what is already on the wall changes only on
     the next show
   - the choice keeps today's #701 semantics: it writes the server preference (`/bible/preferences`,
     the default every console starts from) and the browser's localStorage
   - chips ≥ 44 px tall; `data-translation` + `aria-pressed` for tests
2. **Field** `console-field`, placeholder "Odkaz alebo slová…":
   - `enterkeyhint="go"`, `autocapitalize="off"`, `autocorrect="off"`, `spellcheck="false"`,
     `autocomplete="off"`
   - a ✕ button clears it
   - it does not autofocus on phones (the keyboard would cover the screen); it autofocuses on desktop
3. **Result rows** `console-row` (`data-kind`, `data-book-code`, `data-chapter`, `data-verse-start`,
   `data-verse-end`, `data-default`):
   - label "Hebrejom 11:1 · Žid · SEB" (stored name · matched form · translation)
   - alternative chips `console-alt-chip`
   - hint `console-hint`
   - text-search row `console-text-search`
4. **Preview card** `console-preview` (`data-reference`, `data-slide-index`, `data-slide-count`, settle
   marker `data-preview-for=<query key>`): main text + secondary text, reference, "1/2" when the
   composer splits the verse.
5. **Sections** (field empty): "Táto bohoslužba", "Pripravené kazateľom", "Návrhy AI" (§7.4).
6. **Zhasnúť verš** `console-clear-live`: outlined red. Shown only while something is on air, placed
   above the action bar and never adjacent to the primary button.
7. **Sticky action bar** `console-prev` · `console-show` · `console-next`:
   - "◀ 10:39" · **ZOBRAZIŤ** (after a show: **ĎALŠÍ: 11:2**) · "11:2 ▶"
   - every button ≥ 48 px tall
   - it sits above the phone tab bar
   - the keyboard offset comes from `visualViewport` (CSS var `--kb-inset`); Chrome Android gets
     `interactive-widget=resizes-content`
   - **[unverified on real iOS]**: the bar above the keyboard. Fallback: the preview card has its own
     inline ZOBRAZIŤ button, which is always reachable.

**Desktop** (same component, `[data-shell=desktop]`):

- Left column: field, rows and sections.
- Right column: preview + the existing `BibleSlidesColumn` (multi-select, add-to-presentation, Edit
  mode) for the prepared workflow until #851 replaces it.
- The Prepared and Settings tabs of `BibleTabNav` stay.

### 7.2 Interactions

- **Typing.** Every keystroke runs `resolver::lookup` locally. The target is < 1 ms; **[unverified]**
  at CPU×4 (perf spec). Rows update with no debounce.
- **Auto-preview.**
  - When the top row is a `Passage` or `Ambiguous`, the console waits 150 ms and calls `POST
    /bible/resolve` with the code (existing endpoint, composer #828).
  - The request carries the new optional fields `query`, `source: "console"` and `device:
    "touch|desktop"`.
  - #702 mapping: a single verse → `verse_end = Some(start)`; a whole chapter → `verse_start = 1,
    verse_end = None`.
  - The resolved slides also feed `bs.slides`, so the desktop grid stays in sync.
- **Preview vs show.** Enter or "Go" previews and blurs the field (the phone keyboard closes).
  ZOBRAZIŤ shows the current slide through the existing `POST /bible/trigger-slide`, with new optional
  `source` / `device` fields.
- **After a show.**
  - The field clears (desktop keeps the focus).
  - The preview advances to the next slide of a split verse, then to the next segment, then to the next
    verse (`Versification::step`, across chapter boundaries; disabled at the end of the book).
  - The primary button now reads "ĎALŠÍ: …". One tap = next verse.
  - ◀ / ▶ only move the preview. **Only the primary button sends to the wall.**
- **Zhasnúť** → `POST /bible/clear`.
- **Chips.** An alternative chip swaps the preview. An `OutOfRange` fix chip rewrites the field.
- **Errors are never silent.** A failed resolve or trigger shows an inline card error + toast. If the
  NLT secondary is unavailable, the existing `warning` text appears inline.
- **Leptos rules apply:**
  - keyed `<For>` with key = code + chapter + verses + kind
  - named closures for `each`
  - deferred `focus()` after a signal change (#819)
  - toast asserted via `data-visible`

### 7.3 Keyboard map (desktop)

| Key | Where | Action |
|---|---|---|
| any text | field | live rows |
| ↓ / ↑ | field | move the row selection (the default is selected first) |
| Enter | field | preview the selected row, then move focus to ZOBRAZIŤ |
| Enter / Space | ZOBRAZIŤ focused | show |
| Esc | field | clear; when already empty, blur |
| ← / → | Bible view, focus not in an input | previous / next preview (decided default §13-3) |
| Space or `/` | Bible view, focus not in an input | focus the console field |

`setup_keyboard_shortcuts` (`operator.rs:656-709`) becomes view-scoped:

- slide ←/→ only when `ctx.view == "worship"`
- on `bible` it delegates to the console
- Space focuses the view's own search field

### 7.4 Reserved sections (contract for #851 and C)

```rust
// state/bible_console.rs
pub struct ConsoleCandidate { pub key: String, pub label: String, pub book_code: String, pub chapter: u16,
    pub verse_start: Option<u16>, pub verse_end: Option<u16>, pub translation: Option<String>,
    pub detail: Option<String> /* prepared text or "počul som: …" */, pub origin: CandidateOrigin }
pub enum CandidateOrigin { ShownToday, Prepared, Suggested { confidence: f32 } }
```

- Each section renders `Signal<Vec<ConsoleCandidate>>` as `console-section[data-section=shown-today|prepared|suggested]`.
- **A section is hidden while empty** (decided default §13-5).
- A tap always **previews**; it never shows directly.
- With text in the field, the sections collapse below the result rows.
- #851 fills `prepared`; C fills `suggested` from its live event. Neither needs another console
  redesign.

**"Táto bohoslužba"**

- Server ring `BibleManager.shown_today` (`state/bible_manager.rs`; new code in
  `state/bible_shown.rs`):
  - max 20 entries, de-duplicated by label
  - reset when the local date changes
  - appended in `trigger_bible_slide_output` when the metadata carries a book code
- New endpoint `GET /bible/shown-today`.
- The console refetches it on every `BibleSlide` event. No new `LiveEvent` variant, so the exhaustive
  match in `companion/variables.rs` (live-events rule) is untouched.

### 7.5 Files

- **New:** `crates/presenter-ui/src/pages/bible_console/{mod,field,results,preview,action_bar,on_air,sections,translation_bar}.rs`,
  `state/bible_console.rs` (pure functions, host-tested: rows from a `Resolution`, request mapping,
  stepping, slide advance), `styles/bible_console.css`.
- **Changed:**
  - `pages/bible.rs` (`BibleLiveTab` mounts the console)
  - `TranslationSelectors` (`bible.rs:400-474`) is replaced by the always-visible translation bar
    (`pages/bible_console/translation_bar.rs`); the #701 localStorage behaviour and the server-pref write
    are kept
- **Removed** (B2c), and `.claude/rules/bible-page.md` is rewritten for the console DOM contract:
  - `BookFilter`, `BookList`, `ReferenceInputs` (`bible_reference.rs`), `LoadButton`, the auto-load
    effect (`bible.rs:225-256`) and `load_passage` (`:675-759`)
  - `state/bible_range.rs`
  - the `BibleState` selection fields only those read
  - the header Bible-search branch + `AppContext.bible_search_*`
  - the `SearchResults` Bible branch

## 8. Observability

- **Request log.**
  - `TraceLayer::new_for_http()` in `build_router`.
  - The span is DEBUG and carries method, path + query, user agent, and client IP. Client IP uses
    `live.rs`'s rule: X-Forwarded-For only from a loopback peer.
  - `on_response` is DEBUG with status + latency in ms.
  - The unit already sets `tower_http=debug` (`scripts/deploy/presenter.service:67`).
- **Drop-in cleanup.**
  - `deploy.yml` (next to `:436-441`) and `release.yml` (`:410-416`) run `sudo rm -f
    /etc/systemd/system/presenter.service.d/{ableset-debug,noresolume}.conf`.
  - Both files are leftovers: `ableset-debug.conf` is from closed #600 and overrides `tower_http=info`;
    `noresolume.conf` sets an env var no code reads.
  - Idempotent on SNV and PP. This is a prod config delete, so it is part of what the owner approves
    with this spec.
  - **[unverified]** journal volume. The settings card polls every 5 s on every open operator tab,
    ≈ 720 lines/h per tab. Measure on dev first; if too noisy, exclude `/ui-pkg/*` and the poll
    route.
- **INFO lines:**
  - `presenter::bible::show`: `source` (console | prepared | api | companion | ai), `device`,
    `client_ip`, `reference`, `secondary`. Emitted in `trigger_bible_slide_output`, which gets a
    `source` parameter.
  - `presenter::bible::lookup`:
    - `/bible/lookup` logs `q`, `kind`, `book`, `chapter`, `verses`, `translation`, `ms`.
    - `/bible/resolve` logs the same when the request carries `query`. The UI resolves the reference
      locally, so without that field the typed text would never reach the server.
  - `presenter::bible::name`: see §5.8.

## 9. Cloudflare Access follow-up

Access is live (4 apps, verified 2026-10-13). Commit a reproducible apply script:

- **`ops/cloudflare/presenter_access.py`** is the session script reworked into **reconcile** mode:
  - it creates missing apps and updates the policies and session duration of existing apps to the
    desired state
  - default is a dry run; `--apply` writes
  - it prints ids and names only
  - pure `desired_apps(config)` and `plan(existing, desired)` functions are unit-tested in
    `ops/cloudflare/test_presenter_access.py`, without network
- **Config.**
  - The account id, allowed e-mails, hosts and bypass egress IPs come from the JSON file named by
    `PRESENTER_ACCESS_CONFIG`.
  - Only `ops/cloudflare/presenter-access.example.json` with placeholders is committed (public repo).
  - The real config lives in the dev2 vault next to the token.
  - The token is passed with `secret exec --env CF_TOKEN`.
- Adding a person = one more e-mail in the config + `--apply`.

## 10. Test plan

**Unit tests, presenter-core** (CI `test` job; `rstest` tables, no new dependency):

- `bible/resolver/tests.rs`:
  - corpus: the 100 cases in `testdata/corpus.json` (the review's `850-resolver-corpus.json`) against
    `testdata/structure-slk-seb.json` (66 books × verse counts from `/bible/books`)
  - corpus mapping rules: `book_only` with a chapter → whole-chapter `Passage`; `list` /
    `cross_chapter` → n segments
- Versification:
  - `structure-eng-kjv.json`: `mal 4:5` is valid on KJV but `OutOfRange` "Kniha má len 3 kapitoly" on
    SEB
  - `structure-slk-sevp.json` (PP main): `J 3,16`, `1K 13,4`
- Fold table: diacritics; the ordinals `1` / `1.` / `I` / `I.` / `prvý` / `prvá kniha` / `1 k.`; filler
  words; `P.Š.` no-fold.
- Grammar table: every separator; `heb11:1`; `-` `–` `—` `až` `az`; lists; cross-chapter; `23-24`;
  single-chapter books.
- Ranking: the 3 collisions; `SHORT_DEFAULTS`; fuzzy limits (length < 4 never; 4–6 ≤ 1; 7+ ≤ 2).
- Table invariants: 66 codes present; no duplicate (key, code); every `SHORT_DEFAULTS` code exists.
- `resolve_translation_code` and `Versification::step` across a chapter and a book end.
- Generator: `scripts/dev/test_gen_bible_book_names.py` (canonical arrays parsed = 66 each; `--check`
  detects drift).

**Server tests** (CI `test` job):

- `/bible/lookup`:
  - `zid 11 1` → HEB 11:1, "Hebrejom 11:1"
  - `zid 14` → `outOfRange` + hint + fix
  - empty → 400; unknown translation → 404
- `/bible/passage?book=Židom` on slk-seb.
- Legacy `/bible/trigger` with `book: "zid"`, `translation: "KJV"`:
  - publishes `BibleSlide`
  - `/bible/active-slide` is non-empty
  - labels keep the #824 secondary book
- Companion `{"translation":"SEB","book":"Židom",…}` is acknowledged.
- The 4 AI tools work with `Hebr`.
- `build_fts_query("zid 11:1") == Some("zid*")`; `"Ž 23"` → `None`.
- `bible_book_chapter_summaries` returns canonical order.
- `wasm_ui` encoding negotiation (br / gz / identity, `Vary`).
- `shown_today`: de-dup, cap, date reset with an injected clock.
- The migrated Resolume legacy tests → `state/bible_labels.rs` tests.

**UI host tests:** `state/bible_console.rs`, `state/phone_shell.rs`.

**Playwright** (chromium project, e2e shards):

- **Profiles in `tests/e2e/support.ts`:**
  - `PHONE_390`: 390×844, DPR 3, `isMobile`, `hasTouch`, iOS UA
  - `PHONE_360`: 360×800
  - `PHONE_LANDSCAPE`: 844×390
- **Helpers:** `assertNoHorizontalOverflow`, `assertMinFontSize(16)`, `assertMinTapTarget(44)`,
  `touchDrag` (CDP `Input.dispatchTouchEvent`), `selectPassage(page, "heb 11 1")`.
- **`phone-shell.spec.ts`:**
  - the tab bar switches all 5 sections + URL + Back
  - Viac opens and closes via ✕, backdrop and Escape
  - the layout change needs a confirm: no `POST /stage/layout` before it
  - overflow, font and tap checks on 5 sections × 3 profiles
  - touch drag on a Worship card → 0 `POST /stage/state`; tap → exactly 1
  - desktop: top nav present, no tab bar, mouse `pointerdown` still triggers
- **`phone-payload.spec.ts`:** phone cold load → exactly 1 `.wasm` response, `content-encoding: br`, no
  `/stage?preview=1`. Desktop iframe behaviour stays pinned by the unchanged
  `operator-view-isolation.spec.ts`.
- **`operator-live-resync.spec.ts`:**
  - `routeWebSocket` drops a `Bible` / `BibleSlide` frame, then `close({code: 4000})` → after
    reconnect the strip / header preview shows the dropped verse
  - visibility hidden → visible with a silent socket → reconnect ≤ 1.5 s
- **`bible-console.spec.ts`** (phone + desktop):
  - `zid 11 1` → default row HEB (matched "Žid") → preview = SEB Heb 11:1
  - ZOBRAZIŤ → exactly 1 `POST /bible/trigger-slide` with `mainReference: "Hebrejom 11:1 (SEB)"` and
    `source: "console"`; on-air strip updated; `/stage` (second page) shows the verse
  - ĎALŠÍ → 11:2 in 1 tap
  - `jan 3 16` → Ján + chip "1 Ján"
  - `zid 14` → "Kniha má len 13 kapitol" + fix chip
  - `11:1` → "Napíš aj knihu"
  - `viera je zárukou` → a hit labelled SEB → preview with the correct `data-book-code` / chapter /
    verse
  - `mt 5:3.5.7` → 3 rows walked by ▶
  - a long range → 1/2 then 2/2
  - Zhasnúť → `POST /bible/clear`
  - desktop: type → Enter → Enter; Esc; ←/→; Space
  - zero console errors
- **Migrated in B2c:**
  - The 7 specs that drive the old inputs move to `selectPassage`: `wasm-bible`,
    `wasm-bible-keyboard-nav`, `bible-range-hint`, `bible-slide-char-count`, `bible-nlt-translation`,
    `bible-verse-end-autofill`, `operator-slide-columns`.
  - Their conditional `test.skip(… "No Bible data available")` calls are removed.
  - Behaviour that no longer exists is deleted: the #702 end-mirror becomes a host test of the request
    mapping.
  - **The false positive** `wasm-bible.spec.ts:1851-1880` ("clicking search result sets reference
    inputs" only checks `/\d+/` on a field that defaults to "1") is replaced by the console
    text-search test above, which asserts book code, chapter and verse.
- **On-demand `@perf-phone`:**
  - new `ui-perf.yml`, `workflow_dispatch`, the #386 `ndi-latency.yml` pattern, because timing on the
    shared runner is load-sensitive
  - CDP 9/1.5 Mbps, 150 ms, CPU×4: console usable ≤ 3.5 s; keystroke → row ≤ 50 ms
  - After each deploy that touches payload or console, the same Playwright script runs against prod LAN
    + tunnel (the audit method).
- **Mutation:** no per-PR gate exists. After A1b merges, run `/mutation-sweep`. Survivors in
  `bible/resolver/` are A1 work, not follow-ups.

## 11. Rollout — PR-sized units

Each PR: version bump first, ≤ 600 LoC (generated table and JSON fixtures excluded), CI all green,
auto-merge, deploy and post-deploy verification. New playbook rules land with their PRs:
`.claude/rules/bible-resolver.md`, `phone-shell.md`, the rewritten `bible-page.md`, and the
`bible-slide-composer.md` legacy section.

| # | Unit | Content | ≈ LoC (code + tests) | Tests shipped |
|---|---|---|---|---|
| 1 | **O1 observability + Access script** | `TraceLayer`, drop-in cleanup in deploy/release, `bible::show` INFO (source from the endpoint), `ops/cloudflare/` reconcile script + example config | 180 + 120 | Access plan unittest; `check_deploy_workflows.py` green; journal check after deploy |
| 2 | **A1a name table** | `data/bible-book-names/`, generator + `--check` in `quality`, generated `names_table.rs`, `fold.rs`, `THIRD_PARTY_NOTICES.md`, remove `Ab`→OBA | 250 + 150 (+ ≈800 generated) | fold table, table invariants, generator unittest |
| 3 | **A1b resolver** | `grammar.rs`, `rank.rs`, `versification.rs`, `hint.rs`, API, fixtures | 400 + 200 | corpus 100/100, grammar, ranking, versification |
| 4 | **B1a phone shell nav** | `phone_shell.rs`, tab bar, compact header, Viac sheet + confirm, burger deleted, `phone_shell.css`, viewport meta, shared `set_view` | 420 + 150 | `phone-shell.spec` (nav, sheet, confirm, overflow, landscape, desktop regression) |
| 5 | **B1b touch + inputs** | `pointer_type` gating, 16 px rule, 44 px toolbar, timers wrap | 120 + 150 | touch drag/tap, font size, tap targets, timers overflow |
| 6 | **B1c payload** | wasm-opt + release profile, precompression + negotiation, iframe gating, size budget + MVP validation, `ui-perf.yml` | 200 + 120 | `wasm_ui` unit tests, `phone-payload.spec`, budget unittest |
| 7 | **B1d live WS** | per-event handler (operator + tablet), silence watchdog, visibility resync, `BibleSlide` arm | 250 + 120 | `operator-live-resync.spec` |
| 8 | **B2a console core** | field, local lookup, rows, auto-preview, ZOBRAZIŤ, on-air strip, translation bar (§7.1 1b), `query` / `source` / `device` fields + lookup INFO in `/bible/resolve`; old inputs hidden in the phone shell, still present on desktop | 480 + 180 | `bible-console.spec` (core flows, translation switch re-resolves the preview + book name), host tests |
| 9 | **B2b stepping** | action bar ◀ / ĎALŠÍ / ▶, slide split stepping, segments, Zhasnúť, text-search row + FTS fix (§5.11) | 260 + 150 | `bible-console.spec` (next / list / long range / clear / text search), FTS unit |
| — | **Milestone M1: the phone verse flow works on prod (S1, S4–S6, S8)** | | | |
| 10 | **A2a server resolver** | choke point, `/bible/lookup`, translation alias, AI descriptions + style guide, `load_bible_verses`, canonical book order, `bible::name` INFO | 330 + 200 | router/state tests (S2) |
| 11 | **A2b legacy → trigger-slide** | reroute, `state/bible_labels.rs`, Resolume legacy branch + fields deleted, tests migrated | ≈ −150 net + 200 | state tests, Resolume driver test, Companion → stage check (S3) |
| 12 | **B2c migration + removal** | `selectPassage` helper, 7 specs migrated, old Live-tab inputs + header Bible search removed, false positive replaced | ≈ −900 prod / 400 test lines | the migrated specs |
| 13 | **B2d console complete** | `shown_today` + endpoint + chips, sections contract, desktop wide layout, keyboard map + view-scoped global keys | 380 + 160 | `bible-console.spec` (sections, keyboard), `shown_today` unit |

Docs updates ride along: `docs/architecture.md` endpoint table (`/bible/lookup`, `/bible/shown-today`)
and `docs/functional-needs.md:58` ("mobile remote control?" is answered).

## 12. Risks

| Risk | Mitigation |
|---|---|
| wasm-opt emits post-MVP features and breaks Safari 12 / iOS 12 | `--mvp-features` + a CI validation step; fall back to an explicit `wasm-opt` call |
| iOS keyboard hides the bottom action bar (Playwright cannot emulate it) | `visualViewport` offset + inline ZOBRAZIŤ on the preview card; **[unverified on a real iPhone]** |
| E2E migration is broad: 7 specs, ≈ 90 tests | the old inputs stay until B2c; one helper; migration in its own PR |
| Canonical book order (A2a) breaks tests that take the first `book-item` (`bible-page.md` calls it "1CH") | grep `book-item` + `.first()` and fix in the same PR (the list is removed in B2c anyway) |
| Fuzzy false positives on short inputs | fuzzy only for length ≥ 4 and only when nothing else matched; structure filter; corpus |
| Main translation ≠ the versification the preacher reads (Mal 4:5 KJV = SEB 3:23) | structure always comes from MAIN; hint + fix chip; the secondary is looked up by code and may be missing (today's behaviour) |
| The first NLT fetch for a chapter delays the preview by 0.5–0.9 s | auto-preview starts 150 ms after typing, so the latency is mostly hidden; main-first loading only if logs show it matters |
| Request-log volume | DEBUG level; exclude polling routes if the dev measurement is noisy |
| Near-cap files | every new piece of logic in a new file; run `fn_length_check.py` + `count_prod_lines.sh` on every touched file |
| Two consoles at once | console state is per browser; the on-air strip and "Táto bohoslužba" are shared through the server |

## 13. Decided defaults (from the approved mockup; the owner may override any of them)

1. **Labels in Slovak** on phone AND desktop, from one label source (the approved mockup is Slovak:
   "Piesne, Biblia, Časovače, AI, Nastavenia", "ZOBRAZIŤ", "Zhasnúť").
2. **The desktop 3-field entry is removed** in B2c; the single field is the only path (a `BookOnly`
   result completes the field, so browsing by book still works).
3. **Arrow keys on the desktop Bible view step the preview**; Enter / the primary button shows. Only the
   primary button ever sends to the wall, on every device.
4. **"Táto bohoslužba" is shared** by all devices (server list, resets daily) — there is one wall.
5. **Reserved sections stay hidden while empty** until #851 / C deliver data (no "príde s …"
   placeholders in the live UI).

## 14. Found in the review, outside this spec (tracked on #850 Findings 2 and 5; worked after A + B)

- `/ui/tablet` on phones:
  - the 260 px sidebar takes 2/3 of the width (`tablet.css:132-135`)
  - header actions are cut off
  - Clear is 68×29 next to the title (`tablet.rs:314`)
- `/ui/stream` editor: 75/75 tap targets < 44 px (13×13 checkboxes). Inputs are fixed by §6.4.
- Tunnel stage-preview iframe reloaded every ≈ 61 s for 1.5 h on 2026-10-12 (NDI dead-video watchdog
  loop, forensics §8.2).
- `cloudflared` needed SIGKILL on deploy stop (06:52) + a 17 s tunnel drop at 10:03 on 2026-10-12.
- GUPnP / libnice log noise (≈ 820 lines/day).
- Cloudflare edge caching of `/ui-pkg/*` (`cf-cache-status: DYNAMIC` today), to evaluate after the B1c
  measurement.
