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
- **Everything the wall shows derives from `render_item(source, edits)`.** Stored slide text
  is a cache that every sermon write regenerates; never treat it as the truth.
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
  `#[test]` bodies; a helper-only body fails it). Check new expected strings in a throwaway
  Python mirror first (Tier-0, no local `cargo test`), then port.
- After P1 merges: `/mutation-sweep` (`gh workflow run mutation-full.yml --ref main`); a survivor
  under `bible/sermon/` is P1 work, killed on `dev` with a test, never a follow-up ticket.
