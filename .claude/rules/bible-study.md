---
paths:
  - "crates/presenter-ui/src/pages/study/**"
  - "crates/presenter-ui/src/state/study.rs"
  - "crates/presenter-ui/src/state/study_history.rs"
  - "crates/presenter-ui/src/api/study.rs"
  - "crates/presenter-ui/styles/study.css"
  - "crates/presenter-server/src/router/study_pwa.rs"
  - "crates/presenter-server/src/ui/study_sw.js"
  - "tests/e2e/study-*.ts"
  - "crates/presenter-server/src/router/bible/study.rs"
  - "crates/presenter-server/src/state/bible_source/study.rs"
---

# The study page `/ui/study` (#851 S1–S3)

The preacher's reader, replacing MySword on his phone and tablet: reading, searching and sermon
preparation (the sermon views are in `sermon-prep.md`).

- **Nothing here reaches a wall (T11).** No trigger, clear or stage call; every test in the study
  specs ends with `finish()` (`study-support.ts`), which asserts that and the clean console. The test
  that passes through the operator empties `wallRequests` once it is back: the operator polls the
  stage.
- **No pill in the operator's surface nav.** That row is absolutely positioned at the header's limit:
  a seventh pill slid the AI chip under the stage preview at 1280 px. The page is entered from `/`.
- **A chapter comes from `GET /bible/study/chapter`, in ONE function** (`api/study.rs::load_chapter`).
  An empty `verses` = this translation lacks it; specs gate it with a URL regex, it has no body.
- **The URL is the state.** `state::study::StudyRoute::{parse,to_url}`; every view AND sheet has a
  URL (`s=akcie|historia|pismo|prekladN`), so the system Back closes a sheet. `v=3-12` = the
  selected verses. Moves go through `pages/study/nav.rs`, never a direct `route.set`.
- **`StudyHistory` mirrors the browser's list:** `history.state` = `<generation>:<index>`, the list's
  own id and the entry's index. A state of no list or of another one (the tab left the page and came
  back, or Back reached an older list) starts a NEW list at that entry (`nav::start_list`), so ◀ and
  a sheet's ✕, which only walk the list, can never leave the page. A sheet entry is `listed: false`:
  a step for Back, never a row of the long-press list, never a reason to light ▶. `nav::go` closes a
  sheet in place first and the entry stays unlisted, so Back never lands on a sheet and the place
  is no second row. The list keeps 50; `base` is the browser index of `entries[0]`, and a push after
  going below it rebases there.
- **Settle markers, never timing:** `study-chapter[data-loaded-for="slk-seb:HEB.11"]` +
  `data-state=loading|ready|missing|error`, `search-results[data-loaded-for="slk-seb:viera"]`. A
  chapter on its way leaves the old text on screen; only the newest (`Study::pending`) may land.
- **The scroll is a plan** (`scroll.rs`): the owner of a move sets `ScrollPlan`, `settle` runs it
  in a `requestAnimationFrame` once that chapter (or that search) is on screen. `study.lastVerse` holds
  `<chapter key>|<verse>`, written for the chapter ON SCREEN and read only for that chapter: a saved
  verse is ignored anywhere else, and a link's `v=` wins over it (a reload keeps the place).
- **A verse number works only on the route's chapter** (`reader::chapter_settled`): during a load
  the URL has moved on and the old text is on screen, so numbers are disabled and the sheet reads
  nothing from a chapter the route does not name.
- **Back from a hit keeps the list:** `Search::Ready` stays for its key, nothing is asked again, and the
  offset by history index restores the scroll. Back with no offset: the route's verse, else the top.
- **A failed `/bible/books` is not silent** (`structures_failed`): inline card + retry, a neutral bar
  (never Začiatok/Koniec), and `suggest_unresolved`: a reference gets a hint, only words naming no
  book may search, or the reader gets a false "Nič sa nenašlo".
- **`touch-action: pan-y pinch-zoom`** on the reader: pan-y alone turned pinch-zoom off.
- **Verse numbers are a 44 px gutter, not superscripts:** a number is a control (T3).
- **A translation lacking a place** answers no slides: `Chapter::Missing`, and `step_chapter` walks to
  its nearest chapter by the canonical book number, so the reader never gets stuck.
- **The field shares the console's row builder** (`console_view`) and adds only the chapter grid and
  the text search (`state::study::suggest`; a word naming a book offers both, Enter opens the book). `Suggestions` shows while the field has focus and
  prevents `mousedown`'s default, or the blur removes a row before its `click`.
- **No dead buttons.** New actions are one `VerseAction` + its arm in `run` (`verse_actions.rs`); S5 adds
  the word sheet.
  A new view is a `StudyView` variant. Scope chips and the quoted phrase are the server's (`scope`, D4):
  never filter on the client.
- **CSS:** `study.css` carries its own tokens on `.study` (`position: fixed` over the body). No `inset`
  shorthand: the bundle targets Safari 12. The phone test is `body[data-shell]`, the landscape row
  keys on `(orientation: landscape) and (min-width: 560px)` like the rail. ≥ 1024 px docks the
  sheets as the side pane.
- **The service worker has no `fetch` handler:** network-only, and a page under test stays
  `page.route`-able (Playwright does not see requests a worker makes).
- **The backdrop closes only on a press that started on it:** the finger of the long press that opened a
  sheet lifts over the backdrop, and that click would close the sheet at once.
- **The verse sheet covers the chapter bar on a phone:** the swipe on the text still turns the chapter.
- **E2E:** `study-support.ts`; a late answer is `gateRoute` + `landed()` + `settled()`, never
  `networkidle`; "only the newest lands" is proven to fail with the `pending` guard removed; `body` has no height under the fixed `.study`, so wait for
  `data-wasm-ready` with `state: "attached"`; a long press = `touchDrag` from a point to itself with the assertion
  in `whileHeld`; the deliberate NLT 503 is filtered by status, the failure paths are malformed 200s.
- **Local E2E of a UI change:** the server EMBEDS `presenter-ui/dist` (`include_dir!`), so a CI or
  `/opt/presenter-dev` binary runs the OLD UI. Build it (`cargo build --release -p presenter-server
  --features mock-integrations,test-helpers`, ~11 min) and stop `presenter-dev` for the run (the mock
  Resolume binds 8091). That cargo build re-runs trunk unpatched: the `integrity` console warnings
  in `finish()` are this local dist only.
- **Local UI builds need nightly** (`.cargo/config.toml` has `build-std`): `RUSTUP_TOOLCHAIN=nightly
  trunk build`; on stable, wasm-bindgen fails with `__wbindgen_externref_table_alloc`.

## Compare and cross-references (#851 S4, `pages/study/{compare,crossrefs}.rs`)

- **Compare is a view, its ◀ ▶ a `nav::replace`:** Back leaves it at once, the verses are no history
  rows. Stacked = phone shell; columns = `CompareLayout` (2, 3 at ≥ 1024 px), the spec's aligned grid.
- **One `CompareCell` per chapter:** a chapter left behind drops its cell, so a late answer has nowhere
  to land; only what is on screen is asked (the NLT's 500 requests). A step inside a chapter asks nothing.
- **`study-compare[data-loaded-for="HEB.11.1"]`** = every shown translation answered or failed.
- **A chip writes the WHOLE list back (`listed`), never the fitted one** (`shown`): a narrow tablet must
  not shorten what a wider screen shows. `data-columns` = the columns on screen, not the layout's room:
  a grid with an empty track runs the verses diagonally.
- **Landing is keyed on the place AND the layout** (a rotation lands, a retry does not); Back's offset
  wins. Compare keeps `pane` in its route without showing it, so a tap on a row returns to the sermon.
- **A sheet in the way of a sheet:** "Odkazy" replaces the verse sheet, or `nav::go` closes only the top
  entry and a second Back lands on a sheet. `popstate`'s `sheet_only` needs the sheets to DIFFER: another
  verse of the same chapter is a move and scrolls.
- **Cross-references come once, whole (`limit=500`)** when the verse sheet opens, so the button can say
  `Odkazy (N)`; ten show first, the rest is local. URL: `v=16&s=odkazy`, not the spec's `x=`: a sheet
  over the selection like the others.
- **A row's tap and add use the translation of the text it shows** (`fallbackTranslation`): Joel 2:28
  opens the KJV from the SEB, which has no such verse. Only the NLT stays itself. A reference across
  chapters has no add button: a sermon passage stays in one chapter.

## The read API (#851 D4, `router/bible/study.rs`, `state/bible_source/study.rs`)

- **Counts are verses, not words.** `occurrenceCount`, `total` and `perBook` count rows of
  `bible_strongs_occurrences`, the same rows the list shows. `perBook` always covers the whole Bible,
  `total` follows the scope.
- **Cross-references never call a remote API.** The NLT reads the KJV's text and says so with
  `fallbackTranslation`: ten rows would spend ten of 500 daily requests and fail whole with the API.
- **A verse a translation lacks comes from the KJV, never clamped:** Joel 2:28 is 3:1 in the SEB, whose
  chapter 2 ends at 27, so a clamp shows a wrong verse under a true-looking label (1,337 rows in SEB and
  SEVP). Only the KJV's own gap keeps a place inside the chapter (3 John 15 → 14). Only a range's END is
  clamped. A range leaving the book reads to the book's end; only the label shows the other book.
- **A chapter the translation lacks answers 200 with no verses**, `prev`/`next` still move on (Joel 4
  exists in the SEB and not in the KJV, so the KJV is no judge of what exists); 404 = unknown translation
  or book. `BookScope` is the one scope parser (`all|ot|nt|<BOOK>`).
- **An exact phrase searches `content` only:** the FTS table also indexes the book name (`"job"` = 1,074
  hits, 54 real). A phrase of no words is an empty answer, never `%%`.
- **A study read has a book code, not a name.** `idx_bible_passages_reference` starts with the name, so
  `idx_bible_passages_place` (translation, code, chapter, verse) serves these lookups; without it each
  one scans the translation.
