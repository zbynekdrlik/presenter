---
paths:
  - "crates/presenter-ui/src/pages/bible.rs"
  - "crates/presenter-ui/src/state/bible.rs"
  - "tests/e2e/wasm-bible.spec.ts"
  - "crates/presenter-ui/src/pages/bible_slides.rs"
  - "crates/presenter-ui/src/pages/bible_controls.rs"
  - "crates/presenter-ui/styles/bible.css"
  - "crates/presenter-ui/src/pages/bible_prepared.rs"
  - "crates/presenter-ui/src/pages/bible_prepared_list.rs"
  - "crates/presenter-persistence/src/repository/bible/presentation_order.rs"
  - "tests/e2e/bible-prepared-order.spec.ts"
---

# Bible page (`/ui/operator/bible`) — DOM contract & E2E determinism

## The Live tab IS the verse console (#850 B2c)

`BibleLiveTab` mounts only `<BibleConsole/>` (rules in `bible-console.md`), on a phone and a
desktop alike. Gone in B2c, never bring them back: the book filter / book list, the chapter and
verse fields (#825 range hint, `state/bible_range.rs`, `pages/bible_reference.rs`), the load
button, the 300 ms auto-load, the history list, the two translation `<select>`s (the console's
translation bar replaced them), the header verse search and its results branch
(`AppContext.bible_search_*`). `BibleState` keeps only what the console, the grid and the Prepared
tab read: no selected book / chapter / verse, no `classic_loads` / `console_takes` /
`console_owns_grid` / `translation_echo`.

- **Settle marker:** the books effect publishes `books_translation` as its LAST write (after a
  stale-answer check against the selected translation); `[data-role="bible-console"]` carries it as
  `data-books-translation`. E2E awaits that attribute after any main translation change, never a
  timing guess (#727 rule: a bigger poll timeout cannot fix a predicate).
- **E2E entry:** `selectPassage(page, query)` from `support.ts` types into the console field and
  waits for `data-preview-ready="true"` plus a slide card. A translation is picked with the bar's
  buttons: `[data-role="console-translation"][data-kind="main|secondary"][data-translation=<code>]`
  (`""` = žiadny), never a select. NLT is never a main button. Since #921 a row shows only the
  preferred chips that fit (phone 3, desktop 4): a translation outside them is picked through ▾
  (`[data-role="console-translation-menu"][data-kind=…]`, then
  `[data-role="translation-option"][data-kind=…][data-translation=<code>]`).
- **Desktop layout = mockup v3 section 3** (`docs/mockups/operator-redesign-v3.html`, owner
  complaint 2026-11-06 on #875): sub-tabs 34 px segmented, then the on-air strip (spec section
  7.1 keeps Zhasnúť reachable; one 48 px line, transparent until a verse is live, then a red left
  edge), the field 52 px / 18 px text / 2 px border (blue while focused), "Hlavný preklad" + a row
  of 36 px buttons (on = filled #3b82f6), "Druhý preklad" + its row led by "žiadny" (CSS `order`,
  on = #1e3a5f + blue border), then ◀ Zobraziť ▶ on a 48 px / 1fr / 48 px grid (the step labels
  are `display: none`, their text stays for screen readers and `toHaveText`). Rows, hints, the text
  search and the prepared sermons come after the bar (CSS `order`, DOM order stays the phone's);
  the phone's preview card is not drawn on a desktop (the grid on the right is the preview; its
  data attributes stay for the specs). All in the `body.operator:not([data-shell="phone"])` block
  at the end of `styles/bible_console.css`; `bible-console.spec.ts` pins sizes and order at
  1280×720 and 1920×1080.
- **The page frame = the Piesne one** (#875, `bible.css` end): the left column 360 px + a 1 px
  `border-right` at x = 0, no card border / radius / padding (the tabs get 8 px like the mockup's
  `.lists`), the work area flush beside it (`.operator__slides-column` padding 0), the verse on the
  wall at the bottom 360 × 202.5 (stage_preview.css; under 1025 px the column is 301 px and the
  preview 300 × 168.75, the Piesne mirror's rule). Pinned in
  `operator-stage-preview-layout.spec.ts`.
- **The work area = the Piesne one** (#875 mockup section 3, `operator-mockup-v3.spec.ts`
  "Biblia work area"): the toolbar is an `.operator__worktop operator__worktop--bible` — the
  title `[data-role=bible-worktop-title]` (Live: `state::bible_grid::passage_title`, "Ján 3:16-18
  · SEB + KJV", the reference every live slide carries minus its "(SEB)", the secondary code only
  when a slide has its text; Prepared: the open presentation's name), then "+ Snímka"
  (`add-empty-slide`, `.operator__worktop-add`; desktop = Prepared tab only, the phone keeps its
  "+"), the selection controls, the stepper. The cards share the Piesne rules in
  `operator_frame.css` (selectors list `.operator__slide-card--bible` next to `--worship`), plus a
  Biblia block there: the select zone (text top left, #828 counts floated top right) over the
  trigger zone drawn as the caption `[data-role=slide-caption]` "1 · Ján 3:16"
  (`slide_caption`, verses from the metadata) + `slide-live-tag` "NA STENE". The phone keeps
  "▶ Ján 3:16-18 (SEB)" (`.operator__slide-trigger-ref`; the caption and tag are hidden there). The grid shows on a phone on the
  Pripravené tab only: `BibleSlidesColumn` is always mounted, CSS hides it on the Live tab.
  On the wall = `is_on_wall` (same main text + reference as `AppContext::active_bible_slide`) →
  class `is-on-wall` + `data-on-wall`, never `is-active` (operator.css paints that blue on the
  phone). The Live grid hides the reference footer on a desktop; a prepared card keeps it.
- **Multi-select + add to a presentation** (`SelectionControls`) sit in the title row
  (`bible_slides.rs`, `[data-role="bible-selection"]`), desktop Live tab only (spec section 7.1),
  and only while the grid holds slides; their buttons take the "+ Snímka" look.
- **Reproducing on the live dev server:** `http://10.77.8.134:8080/ui/operator/bible`,
  `/bible/translations`, `/bible/books?translation=<code>`. Book codes are canonical and shared
  across full translations; `slk-mil` is partial (Gospels only).

## Prepared tab: order + archive (#874)

- `GET /bible/presentations` = ACTIVE only, by `position` (tablet, verse console, AI tools never see
  the archive); `?archived=true` = the archive. A new consumer that should see archived ones asks for
  them explicitly. `sermon_ids_on` (today's hand-off) leaves archived sermons out; `/ui/study` lists all.
- New, sermon-created and restored presentations go to the TOP (`move_to_top`). E2E that picked "the
  last card = the new one" broke; pick a card by name. The collapsed archive also carries
  `data-presentation-id`: scope selectors to `[data-role="presentation-card"]`.
- Every write transaction starts with its WRITE (sermon-prep rule): create = INSERT then one UPDATE
  with a subquery; reorder = position UPDATEs, THEN the active-set check (rollback = drop the txn);
  archive / restore = one conditional UPDATE. Concurrency test uses a FILE DB (`file_repo`), the
  in-memory one shares a cache and never shows SQLITE_BUSY.
- UI (`bible_prepared_list.rs`): keyed `<For>` (key = id + every shown field), the moved card's
  handle refocused after a move, drag ended by window `pointerup`/`pointercancel`, reorder PUTs
  serialized (newest waiting order only), list answers gated by `presentation_list_gen` and
  `presentation_order_sending`; a 409 just reloads (no toast). `BibleState` is `Copy`.
