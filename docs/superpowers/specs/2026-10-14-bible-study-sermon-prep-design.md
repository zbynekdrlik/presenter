# Bible Study + Sermon Preparation (#851) — Design

**Issue:** #851 — Bible study + sermon preparation for the preacher on phone/tablet (replace MySword; prepared
verses become the service presentation directly). Designed with #850 (A + B approved, C drafted on #852).
**Date:** 2026-10-14
**Status:** Written spec, waiting for the owner's review before the implementation plan. Every design question
on the ticket is settled (below); nothing here is open.
**Mockups:** `docs/mockups/bible-phone-and-study-v1.html` @ 3b8fae37 and `-v2.html` @ cc916ea6, section
"Štúdium a príprava kázne" (identical in both versions).
**Code baseline:** `dev` @ 9c726944 (v0.4.318). Every `file:line` below refers to that commit. Names from the
approved A + B spec (`docs/superpowers/specs/2026-10-13-bible-verses-any-device-design.md`, "A+B §n") are used
unchanged.
**Legend:** **[unverified]** marks an assumption that the code or a measurement does not prove yet. Each one
says how it gets checked.

Binding decisions this spec applies:

| Source | Decision |
|---|---|
| Owner, 2026-10-13 | Several translations are first-class: at least SEB, ROH, SEVP and KJV. KJV carries the Strong's numbers. Cross-references ("odkazy") are in scope, not "later". |
| ROZHODNUTÉ Q2, 2026-10-14 | Sermon verse edits are **deletions only + emphasis**, no rewording. Slide text always derives from the database text plus the recorded deletions/emphasis; the reference label (`16a`, `16b`, `…`) is computed automatically. |
| Default Q1, 2026-10-14 | Tapping a KJV word AND tapping its Strong's number both open the word sheet. |
| Default Q3, 2026-10-14 | No offline mode in v1. `/ui/study` needs a connection (LAN or the Cloudflare-Access tunnel). |
| Default Q4, 2026-10-14 | The Slovak AI paraphrase of English definitions is deferred with the other AI items. |
| ROZHODNUTÉ Q5, 2026-10-14 | SEB / ROH / SEVP / MIL leave the public repo (#859). |
| A+B §1, §9 | Cloudflare Access protects both public hosts (720 h sessions, bypass for the church egress IPs). No app-level login. |

## 1. Problem

The preacher studies in MySword (Android) and uses a small part of it: reading, search, compare, KJV word
details via Strong's numbers, cross-references, back/forward. To prepare a sermon he copies verses into
Gmail tasks, removes bracketed parts, shortens verses, pastes the result into Discord, and someone feeds it to
Presenter AI on Sunday morning. The team then re-checks whether the verses came through right.

What Presenter has today (#851 Findings 1 + 2, re-verified 2026-10-14):

- `bible_presentations` has 0 rows on prod and dev. The AI path created three presentations on 2026-10-11,
  all deleted later. Nothing of a past sermon is kept.
- A real sermon (2026-08-13): 7 passages, ~25 verses, SEB, scattered verses (Dt 28: 12, 17, 20, 24, 29, 31,
  33, 38-40, 44), shortened verses that still carry the full reference, CAPS emphasis ("skrze VIERU DOSTALI").
- Import defects reach the walls: ROH headings glued into 2,785 verses, KJV footnotes in 5,844 verses, SEB
  stray spaces in 490 verses, ROH Mt 17:21 lost "[a pôstom]".
- The repo's KJV source carries 348,884 Strong's tags in the 66 books; the importer throws them away. There is
  no dictionary, no reading view, no compare beyond two translations, no history, no cross-references.

## 2. Goal and success criteria

The preacher studies and prepares on his own phone or tablet inside Presenter. The prepared verse list IS the
service's Bible presentation: no Gmail, no Discord, no helper, no re-checking.

| # | Criterion | Measured by |
|---|---|---|
| T1 | Import defects gone: ROH GEN 1:1 = "Na počiatku stvoril Bôh nebesia a zem."; 0 eng-kjv rows containing `¶` (2,970 today) or footnote text (KJV GEN 2:14 ends "…the fourth river is Euphrates."); 0 slk-seb rows matching `* [.,;:!?]*` (490 today); ROH MAT 17:21 = "Ale tento druh nevychádza, iba modlitbou [a pôstom]." | importer unit tests + read-only `sqlite3 -readonly` queries on SNV and PP after deploy (database skill procedure) |
| T2 | KJV word data complete: all 31,102 KJV verses carry markup whose plain text equals `content`; 348,884 tags → 310,076 occurrence rows over 14,047 numbers; every number has a dictionary entry (0 missing, checked 2026-10-14); 8,674 Hebrew + 5,624 Greek entries; 328,119 cross-reference rows (votes ≥ 2, OpenBible file of 2026-10-12) covering 29,233 verses | importer tests on the bundled KJV + post-deploy count queries |
| T3 | Phone profiles `PHONE_360`, `PHONE_390`, `PHONE_LANDSCAPE` (A+B §10): 0 elements past the right edge, every input ≥ 16 px, every control ≥ 44×44 px — in reader, search, compare, word sheet, sermon list and editor | Playwright `study-*.spec.ts`, `sermon-*.spec.ts` with the A+B helpers |
| T4 | MySword parity in interactions: typed reference → reader at the verse = 1 field + Enter; switch translation = 1 tap; next chapter = 1 swipe or 1 tap; compare a verse = 2 taps; word sheet = 1 tap on a KJV word or its number; back = 1 tap or the system Back gesture | Playwright interaction counts |
| T5 | The 2026-08-13 sermon rebuilt in E2E (7 passages, ~25 verses, SEB, end cuts on Dt 28:20 and 28:24, "VIERU DOSTALI" in Gal 3:14): labels such as "Deuteronómium 28:20a (SEB)"; slides obey #828 at the prod limit 150; it appears in "Pripravené kazateľom" and on `/ui/tablet` without retyping; ZOBRAZIŤ puts exactly the edited text on `/stage` | `sermon-prep.spec.ts`, `sermon-handoff.spec.ts` |
| T6 | A re-import (PARSER_VERSION bump) leaves every `bible_slides` row and every `bible_sermon_items` row byte-identical; only items whose DB text changed are flagged `sourceChanged` | persistence + server tests |
| T7 | A stale write from a second device gets 409; the editor reloads and nothing is lost silently | server test + two-page Playwright test |
| T8 | Every sermon write logs one INFO `presenter::bible::sermon` line; every import logs its counts | `journalctl -u presenter` after each deploy |
| T9 | `/ui/study` cold open on a phone under the A+B S8 conditions is usable in ≤ 3.5 s on the LAN and ≤ 3.0 s through the tunnel; a chapter switch on the LAN renders in ≤ 300 ms | on-demand `@perf-phone` (`ui-perf.yml`, A+B §10) |
| T10 | Zero browser console errors and warnings in every new or changed spec | each spec's final assertion |
| T11 | Nothing on `/ui/study` can change the walls: 0 requests to `/bible/trigger*`, `/bible/clear`, `/stage/*`, `/bible/presentations/*/slides/*/trigger` during every study spec | request listener in every study spec |

## 3. Scope

**In scope:**

- **D — data.** Import fixes; KJV re-import keeping Strong's numbers, italics and footnotes as study markup;
  SEB notes and ROH headings as study markup; Strong's Hebrew + Greek dictionaries; OpenBible
  cross-references; the Strong's occurrence index.
- **S — study.** The `/ui/study` page: reader, search through the A resolver, compare, KJV word sheet,
  cross-references, back/forward with a history list.
- **P/E — sermon preparation.** Sermon = Bible presentation + date + source items; deletions + emphasis;
  computed labels; slides through the existing composer; reorder; translation change; preview.
- **H — hand-off.** The operator console's "Pripravené kazateľom" section (A+B §7.4), the operator Prepared
  tab, `/ui/tablet`, and the reference list part C reads.

**Out of scope:**

- Offline use (Q3), any AI feature (Q4 and the "later, only on request" list: verse finder, Slovak
  paraphrase).
- Notes, highlights, bookmarks, reading plans, commentaries, interlinear/morphology, audio.
- App-level login and per-author permissions (A+B §3).
- Moving the four Slovak modules out of the repo (#859, its own PR).
- Retiring the operator's ad-hoc Bible presentation building (desktop `BibleSlidesColumn`, Prepared tab).
  A+B §7.1 keeps the column "until #851 replaces it": #851 does not remove it. Sermons are the preacher's path,
  the column stays the operator's ad-hoc path (the AI tab's presentations need it too), and sermons are
  read-only there (§8.2).
- Slovak/Czech Strong's data: none exists (Finding 2), so word details stay KJV + English lexicons.

## 4. Verified constraints (from the code)

**Build and code rules**

- Tier-0: no local cargo compilation; CI compiles. File cap ≤ 1000 lines (warn 800), function cap ≤ 120
  (warn 80). No `unwrap` / `expect` / `panic!` in server, core, persistence or bible crates.
- Near-cap files this work touches: `presenter-bible/src/parsers.rs` 840, `presenter-bible/src/lib.rs` 845,
  server `state/bible.rs` 760, `router.rs` 760, UI `pages/bible_slides.rs` 789, `pages/tablet.rs` 780.
  **All new logic goes into new files**; the near-cap files get call sites only.

**Bible storage and import**

- One row per verse in `bible_passages` (`entities.rs:371-410`); `content` is plain text; ids are fresh UUIDv4
  on every import (`import.rs:196`).
- A re-import deletes the translation row and its passages by FK cascade (`PRAGMA foreign_keys = ON`,
  `repository/mod.rs:142`), then reinserts, inside one transaction with the FTS triggers dropped
  (`import.rs:31-110`).
- Every deploy runs `ingest_bibles` (dev and PP with the server stopped; SNV after the binary swap,
  with the new server running — see `.claude/rules/bible-import.md`). A translation is skipped when
  SHA-256(file bytes ‖ `PARSER_VERSION`) equals the stored digest (`bible_digest.rs:10`,
  `ingest_bibles.rs:64-87`). Bumping `PARSER_VERSION` (today 1) re-imports all translations.
- E2E servers ingest through the same binary (`tests/e2e/support.ts` `refreshDevData` →
  `scripts/dev/refresh-dev-data.sh`).
- `bible_slides` has an FK to `bible_presentations` only (`entities.rs:440-480`). Slide text is a copy, so
  **saved presentations survive every re-import today** (verified in `import.rs` steps 3–5).
- `BiblePassage` is serialized in `/bible/search`, `/bible/passage` and the AI tools; adding a field to it
  would grow every response. `BibleIngestionBatch` has private fields and one constructor (21 call sites).
- `fts_search_raw` maps an explicit SELECT column list into `bible_passage::Model` (`query.rs:161-206`): a new
  field on that entity would fail every search at runtime. `bible_passage::ActiveModel` is built in one place
  (`import.rs:195`).

**Slides, presentations, live events**

- Composer `compose_bible_slides` (`state/slides/compose.rs:424-490`): line `"{verse}. {text}"`, split when the
  LONGER of main/translation would pass the limit (`chars()`), a lone oversized verse stays whole (#828).
  Labels are integer ranges only (`build_reference_label`, `:309-326`).
- `replace_bible_presentation_slides` deletes and reinserts all slides of a presentation in one transaction,
  keeping the given slide ids (`repository/bible/presentations.rs:156-187`).
- The tablet triggers the STORED slide by id (`router/bible/broadcast.rs:176` `trigger_presentation_slide`),
  per the CLAUDE.md tablet rule.
- The presentation list is ordered by name (`presentations.rs:28`); the tablet auto-selects the first one
  (`pages/tablet.rs:97-108`).
- `LiveEvent::BibleSlidesChanged { presentation_id }` exists (`live.rs:51`); the UI bumps
  `AppContext.bible_presentations_version` (`state/mod.rs:139-140`) and the tablet refetches
  (`tablet.rs:60`). **No new `LiveEvent` variant is needed**, so `companion/variables.rs` stays untouched.

**Data facts (computed 2026-10-14 from `data/bibles/kjv.usfm.zip`, 66 books)**

- 31,102 verses, 348,884 `strong="[GH]\d{4}"` attributes (all single numbers), 310,076 distinct
  (number, verse) pairs, 14,047 distinct numbers, 21,469 `\add` / `\+add` spans, 6,959 `\f…\f*` footnotes in
  5,844 verses, 2,970 lines with `¶`.
- SEB: 969 verses with `<FI>[ note ]<Fi>`; 2 verses with bare bracketed text (Mk 16:20 shorter ending,
  1 Thes 3:13 "[Amen.]"). ROH: 2,785 `<TS>heading<Ts>`; 4 bare-bracket verses (Mt 17:21, 1 Jn 2:7, Jud 22,
  Zj 8:13). Today's regex `\[[^\]]*\]` deletes all of them.
- Open Scriptures `StrongHebrewG.xml` (8,674 entries) and `strongsgreek.xml` (5,624 entries) cover every
  number the KJV uses. The Hebrew file also carries TWOT numbers (`gloss`, "Copyright © 1980 Moody Bible
  Institute") and a definition outline (`<list>`) whose source the file does not state.

**Repo, UI, dependencies**

- The repo is PUBLIC: no e-mails, no copyrighted text. Tests use `preacher@example.org`.
- One WASM bundle, routed by pathname (`presenter-ui/src/lib.rs:35-70`). The tablet injects its manifest link
  at runtime (`tablet.rs:644`); `/ui/tablet` is a shell route (`router.rs:167`); sub-routers merge via
  `routes()` (`router.rs:423-441`).
- `presenter-ui` web-sys has `History`, `PopStateEvent`, `PointerEvent`, `Storage`; chrono with `wasmbind`.
  B1a adds `MediaQueryList` (A+B §4).
- `quick-xml` 0.39 is already in `Cargo.lock` (via `gst-plugin-ndi`); uuid `v5` is enabled workspace-wide
  (`Cargo.toml:44`).
- Client IP trust rule: a forwarding header counts only from a loopback peer, i.e. `cloudflared` on the same
  box (`router/integrations/mod.rs:51-62`, #793).
- NLT (#826): fetched on demand, LRU in memory, never bulk-copied, ≤ 500 requests/day on the anonymous key
  (`.claude/rules/bible-remote-translations.md`). Every verse read goes through `state/bible_source.rs`.

## 5. Data layer

### 5.1 Import fixes (unit D1)

| Defect | Cause | Fix |
|---|---|---|
| ROH headings glued into 2,785 verses | `sanitize_mysword_text` strips the `<TS>`/`<Ts>` tags but keeps the text between (`parsers.rs:481-488`) | drop every `<TS>…<Ts>` span (case-insensitive); D2 keeps it as a heading |
| KJV footnotes in 5,844 verses | `sanitize_text` drops markers but keeps the text inside `\f … \f*` (`parsers.rs:417-465`) | drop `\f … \f*` and `\x … \x*` spans; D2 keeps the `\ft` text as a note |
| SEB space before punctuation in 490 verses | the bracket regex removes `[ … ]` inside `<FI>…<Fi>` and leaves the space before `<FI>` (`parsers.rs:18-20, 469`) | remove notes with `\s*<FI>\s*\[[^\]]*\]\s*<Fi>` (leading whitespace included) before tag stripping; then, for every MySword module, drop whitespace before `. , ; : ! ?` (the module's own spacing around `<FR>` quotes leaves 6 SEB and 2 ROH verses otherwise; simulated on the archives 2026-10-14: 0 left) |
| ROH Mt 17:21, 1 Jn 2:7, Jud 22, Zj 8:13 and SEB Mk 16:20, 1 Thes 3:13 lose bracketed text | the regex deletes every `[…]`, also bracketed words that belong to the translation | only `<FI>[…]<Fi>` notes are removed; bare `[…]` stays in the text (the preacher can strike it per item) |
| KJV "¶" at the start of 2,970 verses | a literal paragraph mark in the USFM | drop a leading `¶` and the whitespace after it |

`PARSER_VERSION` becomes 2, so the next deploy re-imports all five translations. New rule file
`.claude/rules/bible-import.md` (paths: `presenter-bible/src/**`, `repository/bible/import.rs`).

### 5.2 Verse markup (unit D2a)

**Approach (chosen):** a side table `bible_passage_markup` (one row per verse that has markup) holding the
verse as ordered segments. The plain `content` is derived from the markup by the parser, so the two can never
drift.

Rejected:

- A nullable `bible_passages.markup_json` column (the draft's idea): `fts_search_raw` maps an explicit column
  list into `bible_passage::Model` (`query.rs:161-206`), so a new entity field breaks every search at runtime
  unless both statements change, and every search hit would then load its markup.
- A word table (one row per word, ~800k rows for KJV): heavier writes on every import and a join for every
  chapter read, for no capability the segments lack.
- Re-parsing the USFM at request time: the archive is a deploy-time file outside the database, and every
  chapter read would re-parse a whole book.
- Adding a field to `BiblePassage`: it is serialized in search, passage and AI responses (§4).

New module `crates/presenter-core/src/bible/markup.rs` (pure, used by the importer, server and WASM):

```rust
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct VerseMarkup(pub Vec<MarkupSegment>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkupSegment {
    #[serde(rename = "t")] pub text: String,
    #[serde(rename = "k", default, skip_serializing_if = "SegmentKind::is_text")] pub kind: SegmentKind,
    #[serde(rename = "s", default, skip_serializing_if = "Vec::is_empty")] pub strongs: Vec<String>,
    #[serde(rename = "i", default, skip_serializing_if = "std::ops::Not::not")] pub italic: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SegmentKind { #[default] Text, Note, Heading }

impl VerseMarkup {
    pub fn plain_text(&self) -> String;                       // Text segments joined; == bible_passages.content
    pub fn strongs_counts(&self) -> BTreeMap<String, u16>;    // "G4102" → tagged words in this verse
}
pub fn normalize_strongs(raw: &str) -> Option<String>;        // "H0430" → "H430", "g26" → "G26", else None
```

Example (KJV HEB 11:1, source `\w Now|strong="G1161"\w* \w faith|strong="G4102"\w* \w is|strong="G2076"\w*
the \w substance|strong="G5287"\w* …`): `[{"t":"Now","s":["G1161"]},{"t":" "},{"t":"faith","s":["G4102"]},
{"t":" "},{"t":"is","s":["G2076"]},{"t":" the "},{"t":"substance","s":["G5287"]},…]`; a KJV italic such as
GEN 2:14's `\add is\add*` becomes `{"t":"is","i":true}`. Whitespace between words lives in Text segments; a
Note sits right after the word it annotates; Headings come first.

| Source | Text | Strong's | Italic | Note | Heading |
|---|---|---|---|---|---|
| KJV USFM | words, `\nd`/`\wj` content | `\w …\|strong="…"` and nested `\+w` | `\add`, `\+add` | `\ft` text of `\f…\f*` | `\d` Psalm titles → verse 1 |
| MySword (SEB, ROH, MIL) | tags stripped, bare `[…]` kept | — | `<FI>word<Fi>` without brackets | `<FI>[ … ]<Fi>` | `<TS>…<Ts>` |
| Obohu (SEVP) | as today | — | — | — | — |

- Markup is stored only when a verse has a Strong's number, an italic, a note or a heading; otherwise there is
  no row and the reader uses `content`. Expected: all 31,102 KJV verses, 1,217 SEB (969 with notes, the rest italics),
  2,785 ROH, 0 MIL, 0 SEVP.
- `BibleIngestionBatch::with_markup(self, Vec<Option<VerseMarkup>>) -> Result<Self, BibleIngestionError>`
  checks one entry per passage and `plain_text() == text`. `into_parts()` is unchanged; the import uses the new
  `into_parts_with_markup()`. The 21 `::new` call sites stay as they are.
- The segment parsers live in new files `presenter-bible/src/markup/{mod,usfm,mysword}.rs`; `parsers.rs` only
  calls them. The old `sanitize_text` / `sanitize_mysword_text` are deleted in D2.
- Content stability: `presenter-bible/src/testdata/kjv-content.sha256` holds the SHA-256 over all 31,102 KJV
  `content` strings as D1 produces them; D2's parser must reproduce it exactly. SEB/ROH forms are pinned by
  synthetic-archive tests (the real files are private after #859).
- `PARSER_VERSION` becomes 3 in D2b, when the markup is first stored.

### 5.3 Markup and Strong's occurrence tables (unit D2b)

```sql
CREATE TABLE bible_passage_markup (
  passage_id TEXT PRIMARY KEY REFERENCES bible_passages(id) ON DELETE CASCADE,
  markup_json TEXT NOT NULL           -- serialized VerseMarkup
) WITHOUT ROWID;

CREATE TABLE bible_strongs_occurrences (
  translation_code TEXT NOT NULL REFERENCES bible_translations(code) ON DELETE CASCADE ON UPDATE CASCADE,
  strongs TEXT NOT NULL,              -- "G4102" / "H430" (normalize_strongs)
  book_code TEXT NOT NULL,
  book_number INTEGER NOT NULL,
  chapter INTEGER NOT NULL,
  verse INTEGER NOT NULL,
  word_count INTEGER NOT NULL,        -- tagged words in this verse with this number
  PRIMARY KEY (translation_code, strongs, book_number, chapter, verse)
) WITHOUT ROWID;
```

- Both are written inside `replace_bible_translation_passages`, in the same transaction, chunked like
  `insert_bible_passages_chunked` (new helpers in `repository/bible/markup.rs`; the passage id is known when
  the passage row is built). The translation delete in step 3 cascades the old rows away: translation →
  passages → markup, and translation → occurrences.
- `bible_passage` (the entity) is unchanged, so search, `find_bible_passage` and `bible_passage_range` read
  exactly what they read today. Markup is read by a dedicated query (join on translation, book code, chapter).
- The occurrences primary key serves "all occurrences of G4102, canonical order" and the per-book counts for the
  scope chips.
- Migration `m20261014_000001_bible_passage_markup.rs` creates both tables (`if_not_exists`).

### 5.4 Strong's dictionaries (unit D3a)

**Source (chosen):** Open Scriptures `strongs` at commit `0acd2f25` — `hebrew/StrongHebrewG.xml` and
`greek/StrongsGreekDictionaryXML_1.4/strongsgreek.xml`, both marked Public Domain (Strong 1890/1894).

Rejected: the same repo's `strongs-*-dictionary.js` (CC BY-SA, share-alike); STEPBible TBESH (needs Online
Bible's permission); STEPBible TBESG / Abbott-Smith (better Greek, but a second format for v1 — can be added as
another `bible_study_sources` row later).

| Field | Hebrew (`<div type="entry">`) | Greek (`<entry strongs="04102">`) |
|---|---|---|
| `strongs` | `w/@ID` "H430" | "G" + number without zeros |
| `lemma` | `w/@lemma` אֱלֹהִים | `greek/@unicode` πίστις |
| `transliteration` | `w/@xlit` ʼĕlôhîym | `greek/@translit` pístis |
| `pronunciation` | `w/@POS` el-o-heem' | `pronunciation/@strongs` pis'-tis |
| `definition` | `note[@type=explanation]` | `strongs_def` |
| `derivation` | `note[@type=exegesis]` | `strongs_derivation` |
| `kjv_renderings` | `note[@type=translation]` | `kjv_def` without the leading ":--" |
| `related` | `w/@src` refs → "H433" | `strongsref` → "G3982" (JSON array) |

- **Not imported:** the TWOT numbers (`w/@gloss`, copyrighted) and the Hebrew `<list>` outline (unstated
  source; abridged-BDB outlines carry the TBESH permission problem). `<hi>` keeps its text; an embedded
  `<w src="433" xlit="…"/>` or `<strongsref strongs="3982"/>` becomes the text "H433 (ʼĕlôwahh)" / "G3982",
  which the UI links.
- Parser: `presenter-bible/src/lexicon.rs` with `quick-xml = "0.39"` (already locked). Rejected: a hand-written
  scanner (entities, nested `<w>` inside notes).

Migration `m20261014_000002_bible_study_sources.rs`:

```sql
CREATE TABLE bible_study_sources (
  code TEXT PRIMARY KEY,              -- "strongs-hebrew" | "strongs-greek" | "openbible-xref"
  name TEXT NOT NULL,
  license TEXT NOT NULL,              -- "Public Domain" | "CC BY (OpenBible.info)"
  source_digest TEXT NOT NULL,
  imported_at TEXT NOT NULL
);
CREATE TABLE bible_lexicon_entries (
  strongs TEXT PRIMARY KEY,
  source_code TEXT NOT NULL REFERENCES bible_study_sources(code) ON DELETE CASCADE,
  lemma TEXT NOT NULL, transliteration TEXT NOT NULL, pronunciation TEXT NOT NULL,
  definition TEXT NOT NULL, derivation TEXT NOT NULL, kjv_renderings TEXT NOT NULL,
  related TEXT NOT NULL               -- JSON array of Strong's ids
);
```

### 5.5 Cross-references (unit D3b)

**Source (chosen):** OpenBible.info cross-references (CC BY; 344,799 rows "From / To / Votes", OSIS ids,
largely the Treasury of Scripture Knowledge plus user votes).

Rejected: the raw TSK module (public domain, no votes to rank by, SWORD format).

- Import keeps rows with votes ≥ 2: 328,119 rows covering 29,233 verses (≥ 5 would cover only 19,892 verses and
  leave 11,210 with nothing).
- OSIS ids map through an exact 66-entry table in `presenter-bible/src/osis.rs`; an unknown id fails the
  import loudly (script-failure policy). Fuzzy matching is never used at ingest (A+B §5.2).
- A target may be a range, also across chapters (`Gen.1.1-Gen.2.3`; 88,150 rows are ranges).

```sql
CREATE TABLE bible_cross_references (
  source_code TEXT NOT NULL REFERENCES bible_study_sources(code) ON DELETE CASCADE,
  from_book_number INTEGER NOT NULL, from_chapter INTEGER NOT NULL, from_verse INTEGER NOT NULL,
  to_book_number INTEGER NOT NULL, to_chapter INTEGER NOT NULL, to_verse INTEGER NOT NULL,
  to_chapter_end INTEGER NOT NULL, to_verse_end INTEGER NOT NULL,
  votes INTEGER NOT NULL,
  PRIMARY KEY (from_book_number, from_chapter, from_verse, to_book_number, to_chapter, to_verse)
) WITHOUT ROWID;
```

Migration `m20261014_000004_bible_cross_references.rs`. Numbering is the KJV's. A Slovak translation is read at the
same code/chapter/verse; a missing verse falls
back to KJV with the note "číslovanie sa môže líšiť" (the rule part C uses, C draft §7.3).

### 5.6 Import mechanics (units D3a, D3b)

- Committed public data in `data/study/`: `strongs.zip` (the two XML files, 1.6 MB deflated) and
  `openbible-cross-references.zip` (the upstream zip, 1.98 MB). `data/study/sources.json` records upstream URL,
  commit/date and SHA-256 per file.
- `scripts/dev/fetch_study_sources.py` downloads and verifies them; `--check` verifies the committed files
  offline and runs in the CI `quality` job with `scripts/dev/test_fetch_study_sources.py` (the
  `scripts/ci/check_*.py` pattern).
- The bytes are compiled into the `ingest_bibles` binary only (`include_bytes!` in
  `presenter-importer/src/bin/ingest_bibles.rs`), so no deploy, release, import-data or E2E workflow changes
  are needed. The server binary does not contain them. `scripts/dev/refresh-dev-data.sh`'s staleness check
  also looks at `data/study/`, so a changed data file refuses a stale binary (the #559 rule).
- After the translations, `ingest_bibles` runs one pass per study source: digest = SHA-256(bytes ‖
  `STUDY_PARSER_VERSION`), skipped when equal to `bible_study_sources.source_digest`, else replaced in one
  transaction (delete the source row → cascade → insert). Prints e.g. `Imported 8674 Strong's entries
  (strongs-hebrew, digest 1f9659ea…)`.
- Attribution: `THIRD_PARTY_NOTICES.md` (created by A1a; D3 creates it if A1a has not landed) gets Open
  Scriptures (public domain) and OpenBible.info (CC BY). ADR 0002 gets a "Study sources" section.

### 5.7 Re-import safety for saved presentations and sermons (verified against the code)

| Data | What a re-import does | Why |
|---|---|---|
| `bible_presentations`, `bible_slides` | nothing | no FK to passages or translations; slide text is a copy (`entities.rs:440-480`) |
| `bible_passage_markup` | deleted with its passage by cascade, rebuilt in the same transaction | FK to `bible_passages` with `ON DELETE CASCADE` |
| `bible_strongs_occurrences` | deleted by cascade, rebuilt in the same transaction | FK to `bible_translations` with `ON DELETE CASCADE` |
| `bible_sermons`, `bible_sermon_items` | nothing | **deliberately no FK to `bible_translations`**: an FK with cascade would delete sermon items when step 3 of the import deletes the translation row. Items keep a text snapshot (§7.1). |
| sermon slides | nothing | regenerated only by a sermon write, never by an import |

After a re-import an item whose snapshot differs from the current DB text is flagged `sourceChanged` (§7.5).
The FK-free design is pinned by a test that re-imports a translation and asserts all sermon rows unchanged.

### 5.8 Size

- Projection: KJV markup ≈ 15 MB, occurrences ≈ 12 MB, cross-references ≈ 14 MB, lexicon ≈ 4 MB, so about
  45 MB more database. `WITHOUT ROWID` keeps the two big tables to one B-tree each.
- **[unverified]** the real growth and the import time: D2 and D3 measure both on the dev deploy and record
  them on #851 (D2b and D3a/D3b). Each pre-deploy backup (5 kept) grows by the same amount.

## 6. The `/ui/study` page

### 6.1 Shape

- Route `/ui/study` (WASM shell) in a new `router/study_pwa.rs` with `routes()`, merged in `build_router` like
  the stream routers. It also serves `/ui/study/manifest.json` (id and scope `/ui/study`, name "Presenter
  Štúdium", short name "Štúdium", orientation any, the tablet icons) and `/ui/study/sw.js` (network-only, no
  caching, Q3). The page injects the manifest link and registers the worker exactly like the tablet
  (`tablet.rs:644`, `tablet.rs:709`), whose install path `tablet-pwa.spec.ts` already proves.
- `presenter-ui/src/lib.rs` routes `p == "/ui/study"` to `pages::study::StudyPage`. SurfaceNav gets
  ("Štúdium", "/ui/study").
- **No live controls.** The page never calls a trigger, clear or stage endpoint (T11). Its only writes are
  sermon writes (§7).
- Layout keys on `is_phone_shell` from `state/phone_shell.rs` (A+B §6.1) plus one wide query:

| Device | Layout |
|---|---|
| Phone (phone shell) | one column: header 48 px (title, ◀ ▶, "Kázne"), search field, favourites row, reader, bottom bar "◀ kap. 10 · Porovnať · kap. 12 ▶" above the safe area; word sheet, verse actions, cross-references and the history list are bottom sheets |
| Tablet portrait (> 768 px, < 1024 px) | phone layout with a wider text column; compare uses 2 columns |
| Tablet landscape / desktop (≥ 1024 px) | two panes: reader (text column ≤ 720 px) and a 380–480 px side pane for the word details, cross-references or the open sermon; compare uses 3 columns over both panes |

- Text size: 3 steps in the header menu, kept per device. All storage goes through the non-throwing
  `state/session.rs` helpers (keys `study.last`, `study.favourites`, `study.textSize`, `study.strongsNumbers`,
  `study.activeSermon`, `study.history`).
- New files: `pages/study/{mod,header,reader,markup_view,favourites,verse_actions,search,history,compare,
  crossrefs,word_sheet,sermons,add_to_sermon,sermon_editor,sermon_item,sermon_reorder,sermon_preview}.rs`,
  `state/study.rs`, `state/sermon_edit.rs`, `api/study.rs`, `api/sermons.rs`, `styles/study.css`,
  `styles/sermon.css`.

### 6.2 URL state and history

Every view has its own URL, so the system Back gesture works. `state/study.rs` holds the pure, host-tested
`StudyRoute::{parse, to_url}`:

| View | URL |
|---|---|
| Reader | `/ui/study?t=slk-seb&p=HEB.11` (`&v=1` = highlighted verse) |
| Word sheet over the reader | `…&w=G4102&wv=1.3` (verse 1, token 3) |
| Cross-references over the reader | `…&x=HEB.11.1` |
| Compare | `/ui/study?view=compare&p=HEB.11.1&tt=slk-seb,slk-roh,slk-sevp,eng-kjv,eng-nlt` |
| Text search | `/ui/study?view=search&t=slk-seb&q=viera+zarukou&scope=nt` |
| Strong's occurrences | `/ui/study?view=strongs&s=G4102&scope=all` |
| Sermon list / one sermon | `/ui/study?view=sermons` / `/ui/study?view=sermon&id=<uuid>` |

- Each navigation calls `pushState({idx})` and appends `{url, label}` to the session list (max 50, e.g.
  "Hebrejom 11 · SEB"); `popstate` restores the view from the URL. Closing a sheet is a Back.
- ◀ ▶ in the header call `history.back()` / `history.forward()` (forward disabled at the newest entry).
  A long press (≥ 500 ms) on ◀ opens the list; choosing an entry calls `history.go(-n)`. MySword's tap = step,
  long tap = list.
- `/ui/study` without a query opens the last position (`study.last`), else `JHN.1` in the first favourite.

### 6.3 Reader

- **Favourites:** five one-tap chips, default SEB · ROH · SEVP · KJV · NLT (`data-translation`,
  `aria-pressed`). A long press on a chip picks which installed translation sits in that slot (MIL included);
  kept per device. Switching keeps book/chapter/verse.
- **Chapter view:** `GET /bible/study/chapter` (§11). Verse numbers are superscripts; headings render as
  `<h3 data-role="study-heading">`; italics as `<i>`; notes as a superscript "†" that opens a small popover
  (`study-note`). A partial translation without the chapter shows "Tento preklad obsahuje len evanjeliá".
- **Chapter change:** horizontal swipe (pointer events; ≥ 60 px horizontally and less than 30° off the
  horizontal; the text column has `touch-action: pan-y pinch-zoom`, the reader stays zoomable) or the bottom-bar buttons. It crosses book
  boundaries and stops at Genesis 1 / Revelation 22. A loaded chapter carries the settle marker
  `data-loaded-for="slk-seb:HEB.11"` (bible-page rule: settle signal, never a timing poll). The last 30
  chapters stay in memory.
- **Strong's numbers (KJV only):** the "Čísla" chip shows the number after each tagged word, MySword style.
  Tapping a word with a number or the number itself opens the word sheet (Q1).
- **Verse actions:** tapping a verse number selects the verse; tapping another extends the selection to a
  range in the same chapter. The verse-action sheet offers Porovnať · Odkazy (N) · Pridať do kázne ·
  Kopírovať (text + reference to the clipboard).
- **NLT** is read live through the existing client; a failure shows the router's "NLT nedostupné — …" text
  inline with a retry button (§12).

### 6.4 Search

One field, placeholder "Odkaz, slová alebo G26…", with `enterkeyhint="search"` and the A+B input attributes:

1. `^[GgHh]\d{1,4}$` (no space) → the Strong's occurrences view, with the resolver's reading ("G26" → Genezis
   26) as an alternative chip. With a space ("g 26") the resolver wins and "G26" is the alternative chip.
2. Otherwise `resolver::lookup(input, &Versification)` (A+B §5.1). The `Versification` is built from
   `/bible/books` of the reader's translation. The rows come from `state/bible_console.rs` (A+B §7.5), so the
   console and the study page share one row builder:
   - `Passage` / `Ambiguous` → open the reader at the passage (highlight the verses); alternative chips.
   - `OutOfRange` → hint + fix chip. `BookOnly` → a chapter grid for the book. `Incomplete` → book chips.
3. `NoBook` with words, or the A+B §5.11 text condition → `GET /bible/search` in the reader's translation,
   limit 100, with scope chips Celá Biblia · SZ · NZ · this book. A query in double quotes is an exact phrase.
   Hits show "Hebrejom 11:1 · SEB" + the snippet with the match marked; actions Otvoriť · Porovnať · Pridať do
   kázne.

Server additions in D4: `/bible/search` gains an optional `scope=ot|nt|<BOOK>` (a `book_number` range
filter in `fts_search_raw`), and `build_fts_query` turns a fully quoted input into an FTS5 phrase. Both build on
the B2b version of `build_fts_query` (A+B §5.11). Existing callers pass `BookScope::All`.

### 6.5 Compare

- **Phone:** one verse stacked across the favourites (`data-layout="stacked"`), each row labelled with the
  short code. ◀ ▶ and swipe move by verse, as MySword's Compare does. The KJV row keeps its tappable words.
- **Tablet / desktop:** 2 columns below 1024 px, 3 at ≥ 1024 px (`data-layout="columns"`,
  `data-columns`). Each column is one translation (a column chip switches it); rows are a CSS grid keyed by verse
  number, so the columns stay aligned by construction, and the chapter scrolls as one.
- Data: the reader's chapter endpoint per translation, fetched in parallel and cached.
- A verse missing in one translation shows "— (v tomto preklade tento verš nie je; číslovanie sa môže
  líšiť)". Example: KJV Mal 4:5 vs SEB, which has 3 chapters.
- An NLT failure affects only the NLT row.

### 6.6 KJV word sheet

Bottom sheet on phones (`word-sheet`, `data-state="peek|full"`), side pane on wide screens:

- **Peek:** the word as printed · the Strong's number · lemma (Hebrew `dir="rtl"`) + transliteration +
  pronunciation · the definition's first sentence · "V KJV: …" · buttons **Všetky výskyty (N)** · **Celé heslo**
  · **◀ ▶ slovo** (previous/next tagged word in the chapter).
- **Full** (drag the handle up or "Celé heslo"): the whole definition, the derivation with every `G…`/`H…`
  linked (opens that entry inside the sheet; Back returns), the KJV renderings.
- Swipe down, tap outside or Back closes it; the reading position never moves.
- Data: `GET /bible/study/lexicon/{strongs}` (+ occurrence count), cached in memory. A word with several
  numbers shows one tab per number.
- "Všetky výskyty" opens the Strong's view: KJV verses in canonical order, the matching words marked, scope
  chips with per-book counts, 100 per page with "Ďalšie". Tapping a verse opens the reader there.
- A missing dictionary (a box without the D3 import) shows "Slovník nie je nainštalovaný".

### 6.7 Cross-references

- "Odkazy (N)" on a verse opens `crossref-list`: the top 10 by votes, each with its label and text in the
  reader's translation, then "Ďalšie (N)" for the rest.
- Tapping one opens the reader there (pushState, so Back returns). Pridať do kázne is available per row.
- Footer "Krížové odkazy: OpenBible.info (CC BY)".

### 6.8 DOM contract (for the specs)

`study-field`, `study-fav` (`data-translation`, `aria-pressed`), `study-chapter` (`data-translation`,
`data-book-code`, `data-chapter`, `data-loaded-for`), `study-verse` (`data-verse`, `data-selected`),
`study-word` (`data-strongs`, `data-verse`, `data-token`), `study-strongs-badge`, `study-heading`,
`study-note`, `study-prev-chapter` / `study-next-chapter`, `history-back` / `history-forward` /
`history-list`, `search-results` / `search-hit` / `search-scope` (`data-scope`), `strongs-results`,
`study-compare` (`data-layout`, `data-columns`), `compare-row` (`data-translation`, `data-verse`),
`word-sheet` (`data-strongs`, `data-state`), `word-sheet-occurrences` / `word-sheet-full` /
`word-sheet-prev` / `word-sheet-next`, `verse-actions`, `crossref-list` / `crossref-item` (`data-votes`),
`add-to-sermon`.

## 7. Sermon preparation

### 7.1 Data model (unit P2)

**Approach (chosen):** a sermon IS a Bible presentation (its slides are what the operator, the tablet and the
walls already play), plus a sermon row and source items. Slides are generated from the items, never edited
directly.

Rejected: a separate sermon entity exported into a presentation on "publish" (a second copy that drifts,
exactly the Logos → Proclaim one-way problem in Finding 2); free-text slides with an edited flag (Q2 forbids
rewording).

```sql
CREATE TABLE bible_sermons (
  presentation_id TEXT PRIMARY KEY REFERENCES bible_presentations(id) ON DELETE CASCADE ON UPDATE CASCADE,
  service_date TEXT NOT NULL,               -- 'YYYY-MM-DD', server-local date of the service
  default_translation_code TEXT NOT NULL,   -- for "+ Pridať verš" and adds from the NLT reader
  secondary_translation_code TEXT NULL,     -- the line under on the wall; NULL = none
  author_email TEXT NULL,                   -- Cloudflare Access e-mail at creation; NULL on LAN / church Wi-Fi
  revision INTEGER NOT NULL DEFAULT 0,      -- +1 on every write (optimistic concurrency)
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE INDEX idx_bible_sermons_service_date ON bible_sermons(service_date);

CREATE TABLE bible_sermon_items (
  id TEXT PRIMARY KEY,                      -- UUIDv4
  presentation_id TEXT NOT NULL REFERENCES bible_sermons(presentation_id) ON DELETE CASCADE,
  item_order INTEGER NOT NULL,
  translation_code TEXT NOT NULL,           -- no FK on purpose (§5.7)
  book_code TEXT NOT NULL,
  book_number INTEGER NOT NULL,
  book_name TEXT NOT NULL,                  -- the translation's stored name at snapshot time ("Hebrejom")
  chapter INTEGER NOT NULL,
  verse_start INTEGER NOT NULL,
  verse_end INTEGER NOT NULL,
  source_json TEXT NOT NULL,                -- [{"verse":1,"text":"Viera je zárukou …"}]: DB text at add time
  edits_json TEXT NOT NULL,                 -- {"1":{"deleted":[7,8,9,10,11],"emphasized":[0]}}
  slide_count INTEGER NOT NULL DEFAULT 0,   -- slides this item produced at the last generation
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE INDEX idx_bible_sermon_items_order ON bible_sermon_items(presentation_id, item_order);
```

- The title is `bible_presentations.name`, default "Kázeň 19. 10. 2026". Deleting the presentation (Prepared
  tab, AI tool or `/ui/study`) cascades to the sermon and its items.
- An item is one contiguous verse range in one chapter, ≤ 60 verses. A cross-chapter selection becomes one
  item per chapter.
- NLT is never an item translation (its text must not be stored as a sermon source). An add from the NLT
  reader uses the sermon's `default_translation_code`, and the button says so ("Pridať (SEB)").
- Migration `m20261014_000003_bible_sermons.rs`; entities `bible_sermon`, `bible_sermon_item` in
  `entities.rs`; repository `repository/bible/sermons.rs`.
- `BiblePresentationSummary` and `BiblePresentation` gain `#[serde(default, skip_serializing_if =
  "Option::is_none")] sermon_date: Option<NaiveDate>` (1 + 3 literal sites, grepped per quality-gates.md). The
  UI DTOs `BiblePresentationSummary` / `BiblePresentationDetail` gain the same optional field.

### 7.2 Editing model (units P1 + E2)

Pure module `crates/presenter-core/src/bible/sermon/{mod,tokens,edits,render,label,dates}.rs`, used by the server
for slides and by the WASM editor for instant display, so the two never disagree.

- **Tokens:** `tokenize(text) -> Vec<Token { start, end }>` = maximal runs of non-whitespace. "nevidíme," is one
  token, punctuation included. Imported text is whitespace-normalized, so joining kept tokens with one space
  reproduces the original. The snapshot never changes, so token indices stay valid forever.
- **Edits per verse:** `deleted` and `emphasized` = sorted token indices. A struck token cannot be
  emphasized (normalization drops it). Indices out of range → 422.
- **Editor gestures** (`state/sermon_edit.rs`, pure and host-tested):
  - The mode toggle **Škrtať | Zvýrazniť** decides what a tap does; a long press (≥ 500 ms) applies the other
    mode (the mockup's "ťuknutím vyškrtneš · dlhým podržaním zvýrazníš").
  - A tap on a token that starts with "(" or "[" strikes through the token that closes it in the same verse,
    as one step. Otherwise only that token.
  - **Do konca verša** / **Od začiatku verša** strike from / up to the last tapped token.
  - **Späť** undoes the last step (session stack of 50 steps).
- Tokens use `user-select: none; -webkit-touch-callout: none` so a long press never starts a text selection.

### 7.3 Rendering and the reference label

`render_item(source, edits) -> RenderedItem { verses: Vec<RenderedVerse { verse, text }>, suffix: String }`,
with these deterministic rules:

| Situation | Text | Label |
|---|---|---|
| End of the verse cut, verse at the end of a run | kept words; trailing `, ; : – —` removed | `16a` |
| Beginning cut, verse at the start of a run | kept words, original case | `16b` |
| Range with both edge cuts | — | `16b-18a` |
| Gap with kept words on both sides (same verse) | ` … ` (U+2026 with spaces) at the gap | unchanged |
| Cut at an edge that the label cannot express: an inner verse, or one verse cut at both ends | leading `… ` and/or trailing ` …`; no trailing ellipsis after `. ! ?` | no letter |
| Whole verse struck | verse omitted | the range splits: `5-7, 9` |
| Emphasized word | Unicode uppercase ("vieru" → "VIERU") | — |
| Every token struck | the item produces no slides; the editor warns "Položka je celá vyškrtnutá — nezobrazí sa" | — |

- Main label: `"{book_name} {chapter}:{suffix} ({SHORT})"`, e.g. "Hebrejom 11:1a (SEB)", "Deuteronómium
  28:20a (SEB)", "Galaťanom 3:5-7, 9 (SEB)". `SHORT` = `translation_short_code`.
- The live composer's line format stays `"{verse}. {text}"`: the line prefix is the integer verse, the label
  carries the letter.
- Example (the mockup): SEB Heb 11:1 is "Viera je zárukou toho, v čo dúfame, a zdôvodnením toho, čo
  nevidíme," (12 tokens). Tokens 7–11 struck ("a zdôvodnením toho, čo nevidíme,") and token 0 emphasized →
  "1. VIERA je zárukou toho, v čo dúfame" labelled "Hebrejom 11:1a (SEB)". Dt 28:20 cut after "podnikať," →
  "…a čo budeš podnikať" labelled "Deuteronómium 28:20a (SEB)", the August sermon's text with a truthful
  reference.

### 7.4 Slide generation (unit P3)

- **Composer refactor (no behaviour change):** `compose_bible_slides` becomes a thin wrapper that computes
  today's labels and calls the new `compose_bible_slides_labelled(main_translation, secondary_translation,
  main_passages, secondary_lookup, character_limit, labels: SlideLabels { main: String, translation:
  Option<String> })`. All existing composer tests stay unchanged.
- **Sermon slides:** for each item in order, `state/sermons/generate.rs` builds `BiblePassage` values whose
  `text` is the rendered verse text and calls `compose_bible_slides_labelled` with the item labels and
  `get_bible_preferences().character_limit` (prod 150). #828 applies unchanged: longer of main/translation,
  `chars()`, a lone oversized verse stays whole.
- **Secondary line:** the sermon's `secondary_translation_code` (default at creation = the Bible preference's
  secondary). Its text is the FULL verses of the kept verse numbers (no deletions mapped across languages), and
  its label names the secondary book with plain numbers ("Hebrews 11:1 (NLT)"), so each label is true for its
  own text. The lookup reuses `AppState::secondary_verse_lookup` (made `pub(super)`): an unavailable NLT
  produces slides without the line plus a warning in the response, never a failed write. NLT verses stored in
  sermon slides are bounded to the sermon's verses, exactly like today's append path.
- **Stable slide ids:** `BibleSlideId::from_uuid(Uuid::new_v5(&item_id, format!("slide/{n}").as_bytes()))`.
  Regenerating an unchanged item keeps its slide ids, so a selection on the tablet or in the console survives an
  edit elsewhere in the sermon.
- Metadata per slide: today's `BibleSlideMetadata` (translation, secondary, stored book name, code, number,
  chapter, the slide's integer verses, both labels). No new field.
- The slides are written with `replace_bible_presentation_slides` semantics inside the sermon write
  transaction (§7.5), then `LiveEvent::BibleSlidesChanged { presentation_id }` is published.
- A change of `character_limit` in `set_bible_preferences` regenerates the slides of every sermon with
  `service_date ≥ today` (a few rows), with one retry on a revision conflict, logged at INFO.

### 7.5 Writes, revisions, concurrency (units P2 + P3 + P4)

- Every write carries `expectedRevision`. The state computes the new item set and the slides in memory, then
  `Repository::save_sermon_change` runs ONE transaction: check the revision, apply the item change, store each
  item's `slide_count`, replace the presentation's slides, bump `revision` and `updated_at`.
- A revision mismatch is `RepositoryError::Conflict("sermon changed on another device")` → 409 through the
  central mapping (repository-error-pattern rule). A reorder whose id set differs from the stored set is the
  same 409 (stale set).
- Reads compute `sourceChanged` per item: some snapshot verse differs from the current DB text (or the verse is
  gone). **Obnoviť text** (`PATCH … {refreshSource: true}`) re-snapshots that item, keeps the edits of verses
  whose text is unchanged and clears the rest.
- Changing an item's translation re-snapshots it and clears its edits; the editor confirms first when the item
  has edits ("Úpravy tejto položky sa zrušia").

### 7.6 The sermon UI (units E1–E3)

**Sermon list** (`view=sermons`):

- "Najbližšie" (`service_date ≥ today`, ascending) and "Staršie" (last 20, descending). A row shows weekday +
  date ("ne 19. 10."), title, item and slide counts, and the author when known (`sermon-row`, `data-sermon-id`,
  `data-date`).
- **+ Nová kázeň:** date (default = the next Sunday, today on a Sunday; the server sends `today` and
  `defaultServiceDate` so the phone's clock never decides) and title (default "Kázeň D. M. YYYY").
- The row menu deletes (confirm). "Moje" filters by the Access e-mail when the request carries one.

**Pridať do kázne** (reader selection, search hit, compare row, cross-reference):

- Adds to the device's active sermon (`study.activeSermon`), at the end. Without an active sermon, a sheet
  lists the upcoming sermons plus "Nová kázeň (ne 19. 10.)".
- Toast "Pridané do kázne „Kázeň 19. 10. 2026" · Späť".

**Editor** (`view=sermon`; mockup screen 3):

- Header "Kázeň · ne 19. 10." + **Náhľad**. Each item is a card (`sermon-item`, `data-item-id`,
  `data-label`, `data-translation`): the computed label + translation chip, the tokens (`sermon-token`,
  `data-verse`, `data-index`, `data-state="kept|struck|emphasized"`; struck = grey line-through, emphasized =
  bold capitals), the mode toolbar (`sermon-mode`, `sermon-cut-to-end`, `sermon-cut-from-start`,
  `sermon-undo`), and the item menu (Zmeniť preklad · Obnoviť text, when `sourceChanged` · Odstrániť).
- **+ Pridať verš:** a field with the same resolver; "gal 3 13-14" + Enter adds the item; words fall back to a
  text search to pick from.
- Footer "3 položky · 4 slajdy"; bottom bar **Poradie** · **HOTOVO**.
- **Saving:** the tokens update locally at once through `render_item`. The edit is saved 300 ms after the last
  tap (one `PUT` per item). The header shows `data-save-state="saved|saving|error"` (the E2E settle marker).
  A failed save keeps the local state, shows "Neuložené — skúsiť znova" and retries on the next tap or on
  reconnect; it never drops an edit.
- **409:** the editor reloads the sermon. A rejected edits `PUT` whose item still exists with the same
  translation and snapshot is re-sent once on the new revision (per-item last writer wins) with the toast
  "Zmeny z iného zariadenia načítané · tvoja úprava je uložená". Any other rejected write (add, delete,
  reorder, translation) is not replayed; the toast names it ("Poradie sa medzitým zmenilo — skús znova").
- Deleting an item shows "Odstránené · Späť" for 5 s; Späť re-adds the same item data at the same position.
- **Náhľad** shows the generated slides exactly as stored (`sermon-preview`, `sermon-slide`): main + secondary
  text, labels, the existing `data-role="slide-char-count"` badges (`state::bible::slide_char_count`, the
  same `chars()` rule), and the NLT warning when present.
- **HOTOVO:** everything is already saved. It returns to the list with "Kázeň je uložená · u operátora v
  „Pripravené kazateľom" v deň bohoslužby a na tablete". There is no draft/ready status.
- Other devices with the same sermon open refetch on `BibleSlidesChanged` for its presentation when the
  revision is newer (`use_live_websocket(surface "study", …)`, A+B §6.7).

**Reorder** (Poradie):

- Every item gets a ≥ 44 px drag handle (`sermon-drag-handle`, pointer drag, `touch-action: none` on the
  handle) and ▲ ▼ buttons for keyboard use. Leaving the mode sends the full id list.
- An empty sermon shows "Zatiaľ žiadne položky" and no handles.

**Wide screens:** the open sermon sits in the side pane next to the reader, so the preacher reads on the left
and builds on the right.

## 8. Hand-off

### 8.1 Operator console — "Pripravené kazateľom" (A+B §7.4)

- The console fetches `GET /bible/sermons/today` on mount, on every `bible_presentations_version` bump and when
  the local date changes. Each item becomes a `ConsoleCandidate`:
  - `key` = item id; `label` = "Hebrejom 11:1a · SEB"; `book_code`, `chapter`, `verse_start`, `verse_end` from
    the item; `translation: Some(code)`; `detail: Some(<rendered text, first 80 chars>)`.
  - `origin: CandidateOrigin::Prepared { presentation_id, slide_ids }`.
- **Contract extension:** A+B §7.4 declares `Prepared` as a unit variant. H1 gives it these two fields, so a
  prepared candidate can play its stored slides. Nothing else in the contract changes; the section stays
  hidden while empty (A+B §13-5) and collapses below the result rows while the field has text.
- **Preview:** a tap previews (A+B rule: a tap never shows). For a prepared candidate the preview card shows
  the item's first STORED slide (main, secondary, label; `data-slide-index` / `data-slide-count` over the
  item's slides). No `/bible/resolve` call is made, because that would show the unedited verse.
- **Show:** ZOBRAZIŤ calls `POST /bible/presentations/{id}/slides/{slide_id}/trigger`, the stored-slide path
  the tablet uses. The server sends exactly the prepared text; the A+B `presenter::bible::show` line logs
  `source=prepared`.
- **Next:** after a show, ĎALŠÍ and ◀ / ▶ walk the sermon's slide order across items (next slide of the item,
  then the next item), the order the preacher prepared.
- Several sermons on one date are grouped under their titles inside the section.
- New file `pages/bible_console/prepared.rs` (fetch + mapping); the pure mapping lives in
  `state/bible_console.rs`.

### 8.2 Operator Prepared tab and server guards

- A presentation with `sermon_date` renders through a new read-only component
  `pages/bible_prepared_sermon.rs`: cards without textareas, the badge "Kázeň · ne 19. 10.", and the link
  "Upraviť v Štúdiu" → `/ui/study?view=sermon&id=…`. A card click still triggers, as for any prepared slide.
- Server guard `AppState::refuse_if_sermon(presentation_id)` (`state/sermons/guard.rs`) runs first in
  `append_bible_presentation_slides`, `delete_bible_slide`, `reorder_bible_slides` and `update_bible_slide`:
  409 "Slajdy kázne sa generujú z jej položiek — upravuje sa v /ui/study". The AI tools go through the same
  state methods and get the same refusal. Rename and delete stay allowed (rename = the sermon title).

### 8.3 Tablet

- Sermons are Bible presentations, so they are already in the tablet's list and play through the stored-slide
  trigger.
- The initial load also fetches `/bible/sermons/today`; when it is non-empty, the tablet selects that sermon
  instead of the first presentation by name. A sermon row shows "Kázeň · ne 19. 10.". The selection helper lives
  in `pages/tablet/sermon_select.rs` (`tablet.rs` is at 780 lines).

### 8.4 Part C

`AppState::prepared_references(date) -> Vec<PreparedReference { presentation_id, item_id, order, label,
translation_code, book_code, chapter, verse_start, verse_end, slide_ids }>` (`state/sermons/mod.rs`) is the
"today's prepared references" input of the C draft (§6.3 ASR context, §7.2 ranking, "čítame ďalej" = the next
item). `/bible/sermons/today` exposes the same data over HTTP.

## 9. Access and identity

- **Behind the tunnel:** Cloudflare Access is live on both public hosts (A+B §1, §9). The preacher at home signs
  in once per 720 h. More preachers = one more e-mail in `ops/cloudflare/presenter_access.py`'s config +
  `--apply`.
- **Author label:** `router/access_identity.rs` `access_email(headers, peer) -> Option<String>` reads
  `Cf-Access-Authenticated-User-Email` only from a loopback peer (or with no peer, as in tests). This is the
  exact rule of `extract_client_ip`. The value is trimmed, lowercased, must contain "@" and be ≤ 254 chars.
  It is stored as `bible_sermons.author_email` on create and logged on every write.
- **It is a label, never a permission.** On the LAN and on the church-Wi-Fi bypass there is no e-mail: drafts
  are shared and editable by anyone who can open Presenter, the same trust level as the operator surfaces today.
- `GET /bible/study/identity` → `{ "email": "…" | null }` drives the list's "Moje" filter and the header
  hint.
- **[unverified]** that Access sends the e-mail header through `cloudflared` for an allow-policy session. Check
  on prod in P4: the `presenter::bible::sermon` line shows `author` on a write from outside the church; until
  then the author is simply empty, nothing breaks.

## 10. Licensing (depends on #859)

| Data | Where | Status |
|---|---|---|
| SEB, ROH, SEVP, MIL texts | private repo after #859; read at deploy as today | #851 adds no copy. Sermon snapshots live only in the prod DB. |
| KJV with Strong's | `data/bibles/kjv.usfm.zip` (public domain, stays) | markup derived from it |
| Strong's Hebrew + Greek | `data/study/strongs.zip` | public domain; TWOT numbers and the Hebrew outline not imported (§5.4) |
| OpenBible cross-references | `data/study/openbible-cross-references.zip` | CC BY → `THIRD_PARTY_NOTICES.md` + the footer in §6.7 |
| NLT | on demand only | reader/compare through the existing client; never an item source; a sermon's secondary line stores at most the sermon's verses, as today's append path does |

- #859 is not a technical blocker for #851. The order #859 → D1 avoids two PRs touching the same import code in
  parallel.
- After #859 the Rust `test` job has no SEB/ROH files, so their forms are pinned with synthetic archives (the
  existing `make_mysword_archive` helper). E2E uses the private checkout like every Bible spec.

## 11. API endpoints and WS events

Query parameters are snake_case and JSON is camelCase, like the existing Bible routes. Writes return the full
`SermonDetail`.

**Study (read-only, `router/bible/study.rs`, `routes()`):**

| Method + path | Answer |
|---|---|
| `GET /bible/study/chapter?translation=slk-seb&book_code=HEB&chapter=11` | `{translation, bookCode, bookNumber, book, chapter, verseCount, prev, next, verses: [{verse, text, markup?}]}`; 404 unknown translation/book; NLT errors 502/503 (central mapping) |
| `GET /bible/study/lexicon/{strongs}` | the entry + `occurrenceCount` (KJV); 400 malformed id; 404 unknown |
| `GET /bible/study/strongs/{strongs}?scope=all\|ot\|nt\|<BOOK>&offset=0&limit=100` | `{strongs, total, perBook: [{bookCode, count}], items: [{bookCode, book, chapter, verse, text, markup}]}`; the UI marks the segments carrying the number |
| `GET /bible/study/crossrefs?book_code=&chapter=&verse=&translation=&limit=10` | `{total, items: [{label, bookCode, chapter, verse, chapterEnd, verseEnd, votes, text, fallbackTranslation?}]}` sorted by votes |
| `GET /bible/study/identity` | `{email: string \| null}` |
| `GET /bible/search?…&scope=` | existing endpoint + optional `scope`, phrase support (§6.4) |

The markup and occurrence reads are `AppState` methods in `state/bible_source.rs` (the bible-remote rule:
only that file reads verses from the repository); a remote translation answers no markup.

**Sermons (`router/bible/sermons.rs`, `routes()`):**

| Method + path | Body / answer |
|---|---|
| `GET /bible/sermons` | `{today, defaultServiceDate, sermons: [{id, title, serviceDate, authorEmail, itemCount, slideCount, updatedAt, revision}]}` |
| `POST /bible/sermons` | `{title?, serviceDate?, defaultTranslation?, secondaryTranslation?}` → creates presentation + sermon |
| `GET /bible/sermons/today` | `[SermonDetail]` for the server-local date |
| `GET /bible/sermons/{id}` | `SermonDetail` = `{id, title, serviceDate, authorEmail, defaultTranslation, secondaryTranslation, revision, items: [{id, order, translation, bookCode, bookNumber, book, chapter, verseStart, verseEnd, source, edits, label, renderedText, sourceChanged, slideIds}], slides: [{id, itemId, mainText, mainReference, secondaryText, secondaryReference}], warnings}` |
| `PATCH /bible/sermons/{id}` | `{expectedRevision, title?, serviceDate?, secondaryTranslation?}` (`null` = none) |
| `DELETE /bible/sermons/{id}` | deletes the presentation (cascade) |
| `POST /bible/sermons/{id}/items` | `{expectedRevision, position?, translation, bookCode, chapter, verseStart, verseEnd}`: snapshots the DB text |
| `PUT /bible/sermons/{id}/items/{itemId}/edits` | `{expectedRevision, verses: {"1": {deleted, emphasized}}}` |
| `PATCH /bible/sermons/{id}/items/{itemId}` | `{expectedRevision, translation}` or `{expectedRevision, refreshSource: true}` |
| `DELETE /bible/sermons/{id}/items/{itemId}?expectedRevision=` | — |
| `POST /bible/sermons/{id}/items/reorder` | `{expectedRevision, itemIds}` (full set) |
| `POST /bible/sermons/{id}/regenerate` | `{expectedRevision}`: retries the secondary line after an NLT failure |

**UI shell:** `GET /ui/study`, `/ui/study/manifest.json`, `/ui/study/sw.js` (`router/study_pwa.rs`).

**WS events:** none new. Sermon writes publish the existing `LiveEvent::BibleSlidesChanged { presentation_id }`.

`docs/architecture.md`'s endpoint table and `docs/functional-needs.md` are updated with the units that add
them.

## 12. Error handling

| Case | Server | UI |
|---|---|---|
| Unknown sermon / item / translation / book / Strong's id | `RepositoryError::NotFound` → 404 | inline "Nenašlo sa" + back to the list |
| Verse range without DB rows, > 60 verses, cross-chapter, NLT as item translation, token index out of range, unknown verse key | `TargetNotFound` → 422 or `bad_request_message` → 400 with a Slovak message | toast with the message; local state kept |
| Stale revision, stale reorder set, slide write on a sermon presentation | `Conflict` → 409 | reload + toast (§7.6); Prepared tab hides the edit controls anyway |
| NLT unavailable | reader/compare: 502/503 (existing mapping); sermon writes: slides without the line + `warnings` | inline card with retry; the editor shows the warning and "Obnoviť slajdy" (`/regenerate`) |
| Dictionary / cross-references not imported | 404 / empty list | "Slovník nie je nainštalovaný" / "Žiadne odkazy" |
| Network loss (no offline, Q3) | — | fetch error toast + inline retry; the editor's pending save retries on the next tap or on reconnect |
| Import data error (bad XML, unknown OSIS id, markup invariant broken) | `ingest_bibles` exits non-zero, the deploy step fails (script-failure policy) | — |

No error is silent: every failed request shows a message and logs at WARN with its cause.

## 13. Observability

- `presenter::bible::sermon` INFO per write: `op` (create, add, edit, translation, refresh, delete_item,
  reorder, update, delete, regenerate), `sermon_id`, `item_id`, `revision`, `items`, `slides`, `author`,
  `client_ip`, `ms`, `warning`.
- Import INFO lines: per translation `markup_verses`, `strongs_tags`, `occurrence_rows`, `italic_spans`,
  `notes`, `headings`; per study source `entries` / `rows` and the digest.
- Study reads are covered by the A+B request log (`TraceLayer`, DEBUG, method + path + query + status + ms);
  failures log at WARN.
- `presenter::bible::show` (A+B §8) already covers prepared shows (`source=prepared`).

## 14. Test plan

**Unit tests, presenter-core** (CI `test` job, `rstest` tables):

- `bible/markup.rs`: `plain_text`, `strongs_counts`, `normalize_strongs` (`H0430` → `H430`, `G4102`, `g26`,
  rejects `G`, `X12`, `G12345`).
- `bible/sermon/`: tokenize (punctuation attached, Slovak diacritics); every §7.3 row (end cut → `a` with the
  comma removed, start cut → `b`, `16b-18a`, inner gap `…`, two-sided single verse, no trailing ellipsis after
  `.`, struck verse → `5-7, 9`, all struck → no slides, emphasis "vieru" → "VIERU"); the Heb 11:1 mockup
  example; edit normalization (struck beats emphasized, sorted, deduplicated); index validation.

**presenter-bible:**

- D1: each §5.1 defect on synthetic archives (SEB note → "Tigris." with no stray space; Mk 16:20 shorter ending
  and "[Amen.]" kept; ROH `<TS>` dropped; ROH "[a pôstom]" kept); KJV GEN 2:14 ends "Euphrates."; JHN 3:16
  starts "For God"; `strips_strongs_concordance_markup_from_usfm` stays green.
- D2a/D2b: KJV markup for all 31,102 verses with `plain_text == content` and the content digest equal to
  `testdata/kjv-content.sha256`; 348,884 tags and 310,076 distinct pairs; nested `\+w` inside `\nd`; `\add` →
  italic; footnote → note; MySword `<FI>[…]<Fi>` → note, `<FI>word<Fi>` → italic, `<TS>` → heading.
- D3a/D3b: H430 and G4102 fields exactly as in §5.4; ":--" stripped; `<list>` and `gloss` absent; 8,674 / 5,624
  entries; cross-references: OSIS mapping, ranges, votes ≥ 2 → 328,119 rows, an unknown OSIS id fails.

**presenter-persistence:**

- Markup + occurrences replaced in the import transaction; importing twice leaves no duplicates; deleting the
  translation cascades the occurrences.
- **Re-import safety (T6):** create presentations and a sermon, re-import the translation, assert every
  `bible_slides`, `bible_sermons` and `bible_sermon_items` row unchanged.
- Sermons: create/list/get; `save_sermon_change` with a stale revision → `Conflict`; reorder with a different id
  set → `Conflict`; deleting the presentation cascades the sermon and items; study-source replace is atomic.

**Server** (router + state tests):

- Every §11 endpoint with its status codes; `/bible/search` scope + phrase.
- Generation through the real composer: #828 longer-of rule and lone oversized verse; labels on slides;
  secondary labels with the secondary book; slide ids stable across a regeneration; NLT secondary failure →
  warning (wiremock via `NltClient::for_test`).
- Guards: append / patch / delete / reorder slides of a sermon presentation → 409; rename allowed.
- `/bible/sermons/today` with an injected clock; character-limit change regenerates future sermons only.
- `access_email`: loopback peer + header → e-mail; LAN peer + header → `None`; malformed → `None`.

**UI host tests:** `state/study.rs` (route parse ⇄ URL round trips, history list cap, Slovak date "ne 19. 10.",
default next Sunday), `state/sermon_edit.rs` (toggle, long-press mode swap, parenthesis expansion, cut to
end / from start, undo), `state/bible_console.rs` prepared mapping.

**Playwright** (chromium; phone profiles + desktop from A+B §10; every spec ends with zero console errors and
the T11 request listener):

- `study-reader.spec.ts`: favourites switch translation (`aria-pressed`) and keep the position; chapter
  buttons (`data-loaded-for` settle marker); URL + browser Back; ROH GEN 1 heading "Stvorenie sveta" as
  `study-heading` and verse 1 "Na počiatku…"; KJV italics; SEB note popover; layout checks (T3).
- `study-navigation.spec.ts`: swipe changes chapter (A+B `touchDrag`); verse selection → verse actions;
  history ◀ ▶ and the long-press list; reload restores the last position; text size persists.
- `study-search.spec.ts`: `zid 11 1` → reader at HEB 11:1 highlighted; `viera zárukou` → hits labelled SEB,
  scope chips change the result set; a quoted phrase; `G4102` → Strong's view; `g 26` → Genezis 26 with the
  G26 alternative chip.
- `study-compare.spec.ts`: phone stacked rows for HEB 11:1 incl. NLT (own mock server, as
  `bible-nlt-translation.spec.ts`), next verse; 1024×768 → 3 aligned columns; MAL 4:5 numbering note.
- `study-word-sheet.spec.ts`: tap "faith" in KJV HEB 11:1 → G4102 πίστις; tap the Strong's badge → the same
  sheet (Q1); occurrence count and list; ◀ ▶ word; derivation link opens G3982; swipe down and Back close it.
- `study-crossrefs.spec.ts`: Odkazy sorted by votes; tap navigates; Back returns.
- `sermon-list.spec.ts`: create (default next Sunday from the server's `defaultServiceDate`), list grouping,
  active sermon, add from a search hit, delete with confirm.
- `sermon-prep.spec.ts`: add HEB 11:1 from the reader; strike from "a" to the end →
  `data-label="Hebrejom 11:1a"`; long-press "Viera" → "VIERA"; add GAL 3:13-14 through "+ Pridať verš"; preview
  texts and char badges; the T5 sermon; translation change with confirm; delete + Späť; a second page writes
  first → 409 toast + reload (T7); reload persists.
- `sermon-reorder.spec.ts` (CLAUDE.md drag-drop rule): empty sermon (no handles, hint), drop above the first,
  below the last, in the middle; ▲ ▼; the order persists after reload.
- `sermon-handoff.spec.ts`: a sermon dated today → the console's `console-section[data-section=prepared]`
  lists its items; tap → the preview shows "1. VIERA je zárukou toho, v čo dúfame"; ZOBRAZIŤ → exactly 1
  `POST /bible/presentations/{id}/slides/{sid}/trigger`; `/stage` (second page) shows the text and "Hebrejom
  11:1a (SEB)"; ĎALŠÍ → the next sermon slide; `/ui/tablet` auto-selects the sermon and a tap triggers the
  stored slide; the Prepared tab shows read-only cards with the Štúdium link; `PATCH` on a sermon slide → 409.

**Mutation:** no per-PR gate exists (A+B §4). After P1 and D2a merge, run `/mutation-sweep`; survivors in
`bible/sermon/` or `bible/markup.rs` are that unit's work, not follow-ups.

**Performance:** `ui-perf.yml` (A+B) gains a `/ui/study` cold-open case and a chapter-switch case (T9).

## 15. Rollout — PR-sized units

Each PR: version bump first, ≤ ~600 LoC of code + tests (data files, fixtures and generated files excluded), CI
all green, auto-merge, deploy and post-deploy verification. Migration names share the spec date; each is
appended to `presenter-migration/src/lib.rs`'s `migrations()` list in merge order, which is the order SeaORM
applies. Rule files land with their units:
`.claude/rules/bible-import.md` (D units), `bible-study.md` (S units), `sermon-prep.md` (P and E units), and the
`bible-slide-composer.md` section on `compose_bible_slides_labelled` (P3).

| # | Unit | Content | ≈ LoC (code + tests) | Depends on |
|---|---|---|---|---|
| 1 | **D1 import fixes** | §5.1 fixes in `parsers.rs`, `PARSER_VERSION` 2, `bible-import.md` | 120 + 200 | — (after #859 preferred) |
| 2 | **D2a markup parsing** | `bible/markup.rs`, `presenter-bible/src/markup/{mod,usfm,mysword}.rs`, `with_markup` / `into_parts_with_markup`, KJV content digest fixture, old sanitizers deleted | 300 + 220 | D1 |
| 3 | **D2b markup storage** | migration 000001 (`bible_passage_markup`, `bible_strongs_occurrences`), `repository/bible/markup.rs`, writes in the import transaction, `PARSER_VERSION` 3, cascade tests | 170 + 150 | D2a |
| 4 | **D3a Strong's dictionaries** | `data/study/strongs.zip` + `sources.json`, fetch script + `--check` in `quality`, `lexicon.rs` (quick-xml), migration 000002 (`bible_study_sources`, `bible_lexicon_entries`), `repository/bible/study_sources.rs`, `presenter-importer/src/study_sources.rs` + the `ingest_bibles` pass, notices + ADR 0002 | 320 + 180 | D2b; A1a (notices file) |
| 5 | **D3b cross-references** | `data/study/openbible-cross-references.zip`, `osis.rs`, `crossrefs.rs`, migration 000004 (`bible_cross_references`), its ingest pass | 180 + 120 | D3a |
| 6 | **P1 sermon core** | `bible/sermon/` pure module, `sermon-prep.md` | 300 + 300 | — |
| 7 | **P2 sermon persistence** | migration 000003, entities, `repository/bible/sermons.rs`, `sermon_date` on presentation types | 330 + 230 | P1 |
| 8 | **P3 sermon generation** | `compose_bible_slides_labelled`, `state/sermons/{mod,generate,snapshot}.rs`, stable ids, secondary line, preference-change regeneration | 380 + 240 | P2 |
| 9 | **P4 sermon API** | `router/bible/sermons.rs`, guards, `/today` with clock, `access_identity.rs`, INFO line, architecture doc | 300 + 260 | P3; O1 |
| — | **Milestone M1: walls fixed (D1 on prod), sermon API on prod (P4)** | | | |
| 10 | **D4 study read API** | `router/bible/study.rs`, `bible_source.rs` markup/occurrence/cross-reference reads, `/bible/search` scope + phrase | 380 + 240 | D3b; B2b |
| 11 | **S1 study shell + reader** | `router/study_pwa.rs`, `lib.rs` route, SurfaceNav, `state/study.rs` URL state, `pages/study/{mod,header,reader,markup_view,favourites}.rs`, chapter buttons, `study.css`, `bible-study.md`, `study-reader.spec.ts`, the `ui-perf.yml` study cases | 380 + 180 | D4; B1a; B1c |
| 12 | **S2 study navigation** | swipe, verse selection + `verse_actions.rs`, `history.rs` (◀ ▶, long-press list), last position, text size, `study-navigation.spec.ts` | 300 + 200 | S1 |
| 13 | **S3 study search** | `pages/study/search.rs`, resolver rows via `state/bible_console.rs`, text search + scope + phrase, Strong's occurrences view, `study-search.spec.ts` | 320 + 200 | S2; A1b; B2a |
| 14 | **E1 sermon list + add** | `pages/study/{sermons,add_to_sermon}.rs`, active sermon, create/delete, `sermon-list.spec.ts` | 380 + 200 | S3; P4 |
| 15 | **E2 sermon editor** | `pages/study/{sermon_editor,sermon_item}.rs`, `state/sermon_edit.rs`, local render, debounced save + 409 handling, translation change, Obnoviť text, delete + Späť, `sermon-prep.spec.ts` | 360 + 220 | E1 |
| 16 | **E3 preview, reorder, wide layout** | `pages/study/{sermon_preview,sermon_reorder}.rs`, HOTOVO, multi-device refetch, side-by-side reader + sermon, `sermon-reorder.spec.ts` | 330 + 240 | E2; B1d |
| 17 | **H1 hand-off** | `CandidateOrigin::Prepared` fields, `pages/bible_console/prepared.rs`, preview/show/step through stored slides, `bible_prepared_sermon.rs`, `pages/tablet/sermon_select.rs`, `sermon-handoff.spec.ts` | 360 + 260 | P4; B2d |
| — | **Milestone M2: sermon preparation end to end on prod (T5, T7)** | | | |
| 18 | **S4 compare + cross-references** | `pages/study/{compare,crossrefs}.rs`, `study-compare.spec.ts`, `study-crossrefs.spec.ts` | 400 + 220 | S2; D4 |
| 19 | **S5 KJV word sheet** | `pages/study/word_sheet.rs`, numbers toggle, occurrences link, RTL, `study-word-sheet.spec.ts` | 380 + 220 | S3; D4 |
| — | **Milestone M3: MySword replaced (T2–T4, T9)** | | | |

**Order:** D1 first (the walls improve at once and it needs nothing from #850), then P1–P4 (server only, no #850
dependency), D2a–D4, and the UI units as the #850 units they need land. The sermon path (E1–E3, H1) comes before
compare and the word sheet because the Gmail → Discord hop is the daily pain.

**WASM size:** S, E and H units grow the single bundle. Each such PR measures the raw and br sizes and updates
`scripts/ci/ui_size_budget.json` (A+B §6.6) with the measured numbers in the PR description; T9 guards the
outcome.

## 16. Risks

| Risk | Mitigation |
|---|---|
| D2a changes KJV `content` by accident | content digest fixture (§5.2); the D1 output is the reference |
| A sermon item FK added later cascades on re-import | §5.7 documents the rule; the re-import safety test fails on such a change |
| Database growth (~45 MB) and backup growth | `WITHOUT ROWID`; measured on the dev deploy in D2b/D3a/D3b **[unverified]**; votes ≥ 2 filter |
| Import time after the `PARSER_VERSION` bumps | measured on the dev deploy; the server is stopped during ingest anyway (existing flow) |
| iOS long press opens the text callout instead of emphasis | `user-select: none`, `-webkit-touch-callout: none`, mode toggle as the always-working path; **[unverified on a real iPhone]** |
| Swipe fights vertical scrolling | `touch-action: pan-y pinch-zoom`, 60 px / 30° threshold, chapter buttons always available |
| NLT quota (500 requests/day anonymous) under heavy study | LRU cache; the client logs every request; WARN on quota answers; a registered key raises it to 5,000 (env `PRESENTER_NLT_API_KEY`) |
| Versification differences (KJV-numbered cross-references, Mal 4 vs 3) | same code/chapter/verse + "číslovanie sa môže líšiť" + KJV fallback, the C rule |
| The Hebrew sheet is shorter than MySword's Strong + BDB | Strong's full entry shown; a clean BDB (Open Scriptures, CC BY, unfinished) can be added later as another `bible_study_sources` row |
| Access e-mail header missing behind the tunnel | the author is only a label; checked on prod in P4 (§9) |
| Two devices edit one sermon | revisions + 409 + reload; a rejected item edit is re-sent once, any other rejected write is named in a toast (§7.6) |
| WASM growth pushes the phone cold load over T9 | per-PR size measurement; `@perf-phone` run after S1 and E3 |

## 17. Decided defaults (the owner may override any of them)

1. All study and sermon labels are Slovak; dictionary definitions stay English (Q4 deferred).
2. Favourites default to SEB · ROH · SEVP · KJV · NLT, changeable per device by a long press.
3. A verse cut at both ends, or an inner verse cut at an edge, uses ellipses instead of a letter; letters
   (`a`, `b`) mark only the edges of a run.
4. An end cut removes a trailing `,` `;` `:` or dash; no period is added; the case is never changed.
5. Emphasis is uppercase on the wall and bold capitals in the editor.
6. Sermon slides carry the sermon's secondary line (default: the Bible preference's secondary) as full verses
   with their own plain-number label; "žiadny" turns it off per sermon.
7. The default service date is the next Sunday (today on a Sunday); the default title is "Kázeň D. M. YYYY".
8. Today's sermons show in the console and on the tablet with no draft/ready status; the editor autosaves and
   HOTOVO only closes it.
9. KJV's `¶` is removed; bracketed words that belong to ROH or SEB stay in the text and can be struck per item.
10. Cross-references are imported with ≥ 2 votes; the top 10 show first.
11. The Hebrew definition outline and the TWOT numbers are not imported.
12. The operator's ad-hoc Bible presentation building stays; sermon presentations are read-only there.
13. The NLT is never a sermon item's translation; an item has at most 60 verses.
14. Everyone who can open Presenter can edit every sermon; the Access e-mail is a label.
