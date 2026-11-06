---
paths:
  - "crates/presenter-server/src/state/slides/compose.rs"
  - "crates/presenter-server/src/state/slides/tests.rs"
  - "crates/presenter-server/src/state/bible.rs"
  - "crates/presenter-server/src/state/bible_labels.rs"
  - "crates/presenter-server/src/state/bible/legacy_wall_tests.rs"
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
- Legacy `/bible/trigger` path (Companion, the AI `trigger_bible_verse` tool):
  `trigger_secondary_text` returns `TriggerSecondary { text, translation_code,
  book }`; `state/bible_labels.rs::legacy_slide_output` builds the ONE
  `BibleSlideOutput` from it — the secondary book swapped into the main
  reference ("1 John 1:1-3 (KJV)", falling back to the main name), and NO
  secondary reference without a secondary text (an empty `translationText`
  with a stored secondary translation used to still send "… (KJV)").
- **One shape of output for every path (#850 A2b, spec §5.10).**
  `trigger_bible_passage` and `trigger_bible_slide_output` both end in
  `AppState::show_bible_on_wall` (show log, `slide_output` + `broadcast`,
  `Bible` + `BibleSlide`, `BibleUpdate::from_slide_output`). There is no
  legacy Resolume branch any more: `BibleUpdate` carries only `slide_output`
  (`None` = clear). The legacy trigger still returns and publishes its passage
  broadcast (translation name, code, number) — the `/bible/trigger` and
  Companion `apply_bible` contract — while the slide path publishes the
  metadata broadcast (`slide_broadcast`). Each caller passes a
  `BibleShowOrigin` (`trigger` with the client IP, `companion` / `ai` via
  `BibleShowOrigin::internal`, client IP `-`).
- A legacy reference may carry ONLY the main-language name ("1 Ján", no or a
  blank `book_code`): look the secondary translation up by the canonical code
  (`secondary_book_code` → `bible_source::canonical_book`), never by the main
  name — eng-kjv has no "1 Ján" rows, so the secondary text was silently empty
  before.
- The edited-text branch reads the book name from the translation's STRUCTURE
  source (`bible_remote::structure_source`): for the NLT that is eng-kjv, so it
  never waits on an API request before going on air.
- Build a `BibleUpdate` only through `BibleUpdate::from_slide_output`. Legacy-path
  wall tests live in `state/bible/legacy_wall_tests.rs`: real host workers
  (`resolume_registry.set_hosts`) against `resolume::tests::mount_full_composition`
  (`pub(crate)`, the 16-clip Arena with every Bible lane clip), compared push for
  push with the slide path. Driver-level tests reuse `tests::setup_bible_driver`
  and stay out of the over-long `resolume/tests.rs` (#487).

## Caller labels: `compose_bible_slides_labelled` (#851 P2a)

`compose_bible_slides` is a thin wrapper: it builds today's integer-range labels and calls
`compose_bible_slides_labelled(main, secondary, passages, lookup, limit, SlideLabels { main,
translation })`, which holds the whole #828 loop. A sermon item passes its computed label
("Hebrejom 11:1a (SEB)") and the secondary book's plain-number label; the line format stays
`"{verse}. {text}"` (the integer verse prefixes the line, the label carries the letter). A new
caller with its own labels calls the labelled form; never fork the loop. "(SEB)" comes from
`presenter_core::bible::sermon::translation_short_code` (one copy for the composer and the
sermon labels).
