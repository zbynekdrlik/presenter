---
paths:
  - "crates/presenter-core/src/bible/sermon/**"
  - "crates/presenter-migration/src/m20261014_000003_bible_sermons.rs"
  - "crates/presenter-persistence/src/entities/sermon.rs"
  - "crates/presenter-persistence/src/repository/bible/sermons.rs"
  - "crates/presenter-persistence/src/repository/bible/sermons/**"
  - "crates/presenter-server/src/state/sermons/**"
  - "crates/presenter-server/src/router/bible/sermons.rs"
  - "crates/presenter-server/src/router/bible/sermons/**"
  - "crates/presenter-server/src/router/access_identity.rs"
---

# Sermon preparation (#851 P and E units)

## The core — `presenter_core::bible::sermon` (P1), shared by the server and the WASM editor

- **Edits are deletions + emphasis only** (ROZHODNUTÉ Q2, no rewording):
  `ItemEdits(BTreeMap<verse, VerseEdits { deleted, emphasized }>)`, token indices into
  `tokenize(source text)` = maximal non-whitespace runs ("nevidíme," is one token). A
  snapshot never changes, so the indices stay valid; only a translation change or a refresh
  re-snapshots (spec §7.5). `normalize_edits` sorts, deduplicates, drops struck-and-emphasized
  emphasis and untouched verses, and refuses an unknown verse / an index past the verse with a
  Slovak `EditError` (the server answers 422 with its text).
- **Everything the wall shows derives from `render_item(source, edits)`.** Once sermons are
  stored (P2b/P3), their slide text is a cache that every sermon write regenerates; never treat
  it as the truth.
- **Label rules (§7.3, decided defaults 3–5):** letters only on the EDGES of a run of
  consecutive kept verses (`b` = start cut on the run's first verse, `a` = end cut on its last;
  `16b-18a`); a cut the label cannot express (an inner verse, the inner edge of a run's first or
  last verse, one verse cut at both ends) reads `… ` / ` …`; an end cut trims a trailing
  `, ; : – —` (also before a trailing ellipsis) and never adds a period; no trailing ellipsis
  after `. ! ?`; a gap inside a verse reads ` … `; a wholly struck verse splits the label
  (`5-7, 9`); every token struck → no verses and the suffix names the whole source range (the
  editor still labels the card); emphasis = Unicode upper case ("vieru" → "VIERU").
- **`translation_short_code`** (core `label.rs`) is the shared "(SEB)" formatter; new code calls
  it, never a copy. Three older copies remain today: the live composer's private
  `translation_short_code` in `state/slides/compose.rs` (P2a switches the composer to the core
  function), an inline `short_code` in `ai/tools/bible.rs`, and `resolume/handlers.rs`'s own.
- **End cuts:** a trailing `, ; : – —` goes even when glued to the word, but a hyphen only as a
  standalone spaced ` - ` token (ROH and SEVP write the dash that way); a hyphen inside or at the
  end of a word ("Bar-Jona", "nad- a podzemné") always stays. A sentence end followed only by
  closing quotes or brackets (`.)`, `.”`, `!“`, `?»`, `.’`, `.]`) gets no trailing ellipsis.
- `MAX_ITEM_VERSES` = 60. `default_service_date(today)` = the next Sunday (today on a Sunday),
  `default_sermon_title(date)` = "Kázeň D. M. YYYY" — computed from the SERVER's date (the
  `state::sermons::local_today` helper arrives in P3), never the phone's.
- Tests are plain `#[test]` fns with inline `assert_eq!` (the quality job's assertion check reads
  `#[test]` bodies; a helper-only body fails it).
- After P1 merges: `/mutation-sweep` (`gh workflow run mutation-full.yml --ref main`); a survivor
  under `bible/sermon/` is P1 work, killed on `dev` with a test, never a follow-up ticket.

## Persistence (P2a, P2b)

- **A sermon IS a Bible presentation**: `bible_sermons.presentation_id` → `bible_presentations(id)`
  ON DELETE CASCADE; the title is the presentation's name; deleting the presentation (Prepared
  tab, AI tool, `/ui/study`) removes the sermon and its items.
- **Deliberately NO foreign key from `bible_sermon_items.translation_code`** (spec §5.7): every
  Bible re-import deletes the translation row and re-inserts it, and a cascading key would
  delete the items. Items keep a text snapshot (`source_json`). The re-import test
  `a_reimport_leaves_slides_sermons_and_items_byte_identical` fails if a key sneaks in.
- Migration `m20261014_000003_bible_sermons.rs` is raw SQL (`IF NOT EXISTS`), like
  `m20260410_000001_separate_bible`, so it reads like the spec; never edit it after it ran on
  SNV/PP — the next change is a new migration.
- Entities live in `entities/sermon.rs` (re-exported from `entities`: `bible_sermon`,
  `bible_sermon_item`) because `entities.rs` sits at 927 production lines. String keys are
  `auto_increment = false` and written with `Entity::insert(..).exec` (database skill #658).
- `BiblePresentationSummary` / `BiblePresentation` carry `sermon_date: Option<NaiveDate>`
  (`skip_serializing_if = "Option::is_none"`): filled by `sermon_dates()` / `sermon_date_of()`.
  A new literal of either struct must name the field (E0063 is CI-only).
