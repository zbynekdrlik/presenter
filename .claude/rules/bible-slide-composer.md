---
paths:
  - "crates/presenter-server/src/state/slides/compose.rs"
  - "crates/presenter-server/src/state/slides/tests.rs"
  - "crates/presenter-server/src/state/bible.rs"
  - "crates/presenter-server/src/resolume/legacy_reference.rs"
  - "crates/presenter-server/src/resolume/handlers.rs"
  - "crates/presenter-ui/src/pages/bible_slides.rs"
---

# Bible slide composer + the legacy trigger path (#824, #828)

## Character limit = the LONGER text, in characters (#828)

`compose_bible_slides` adds a verse to the current slide only if NEITHER the
main NOR the translation accumulator would exceed `character_limit`
(`SlideDraft::would_exceed`). Count with `chars().count()`, never `.len()`:
`.len()` is UTF-8 bytes, so every Slovak diacritic used to count twice and an
English secondary line was never measured at all. A lone verse longer than the
limit still stays ONE whole slide (#434): the split happens only when the draft
is non-empty. The slide construction lives in `BibleSlideFrame::push_slide`;
keep `compose_bible_slides` a thin loop (it sat at 115/120 lines before #828).

The AI item composer (`VerseAccumulator`, main text only — AI slides carry no
translation) and the bible validator's length rule count `chars()` too; move
them TOGETHER, or the composer packs a slide the validator rejects and the
agent loops (#784).

The operator card shows the same count: `slide_body_view` renders a
`data-role="slide-char-count"` badge per text (`data-field="main|translation"`,
`data-over="true"` above `BibleState.character_limit`), computed by the
host-tested `state::bible::slide_char_count` — the same `chars()` rule as the
server, so the badge and the split always agree.

## The secondary reference names the SECONDARY book (#824)

- Live path: the composer takes the secondary label's book name from the
  secondary passages (`secondary_book_name`); no secondary passage → no label.
- Legacy `/bible/trigger` path (Companion, the AI `trigger_bible` tool):
  `trigger_secondary_text` returns `TriggerSecondary { text, translation_code,
  book }`; `BibleUpdate.secondary_book` carries the book to the Resolume
  workers, and `legacy_reference::legacy_translation_reference` swaps it into
  the main reference ("1 John 1:1-3 (KJV)"), falling back to the main name.
- A legacy reference may carry ONLY the main-language name ("1 Ján", no or a
  blank `book_code`): look the secondary translation up by the canonical code
  (`secondary_book_code` → `bible_source::canonical_book`), never by the main
  name — eng-kjv has no "1 Ján" rows, so the secondary text was silently empty
  before.
- The edited-text branch reads the book name from the translation's STRUCTURE
  source (`bible_remote::structure_source`): for the NLT that is eng-kjv, so it
  never waits on an API request before going on air.
- New `BibleUpdate` field? Grep every `BibleUpdate {` literal (`resolume/tests.rs`,
  `bible_clear_tests.rs`, `legacy_bible_tests.rs`, `state/bible.rs`) — E0063 is
  CI-only on this Tier-0 box. New legacy-path tests go to
  `resolume/legacy_bible_tests.rs`, not the over-long `resolume/tests.rs` (#487);
  reuse `tests::setup_bible_driver` (`pub(super)`, the 16-clip Arena with every
  Bible lane clip) instead of copying the arena/driver helpers.
