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
---

# The study page `/ui/study` (#851 S1–S3)

The preacher's reader, replacing MySword on his phone and tablet. Reading and searching only.

- **Nothing here reaches a wall (T11).** No trigger, clear or stage call; every test in the study
  specs ends with `finish()` (`study-support.ts`), which asserts that and the clean console. The test
  that passes through the operator empties `wallRequests` once it is back: the operator polls the
  stage.
- **No pill in the operator's surface nav.** That row is absolutely positioned at the header's limit:
  a seventh pill slid the AI chip under the stage preview at 1280 px. The page is entered from `/`.
- **A chapter comes from `POST /bible/resolve` with `characterLimit: 1`, in ONE function**
  (`api/study.rs::load_chapter`), until D4's `/bible/study/chapter` replaces it. `/bible/passage`
  cannot: it matches a row whose verse span EQUALS the request and rows are single verses, so a
  chapter is `null`. Resolve goes through `bible_source` (NLT included), stores nothing, and a lone
  verse is never split (#434), so each slide is one verse written `"{verse}. {text}"`.
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
- **No dead buttons.** New actions are one `VerseAction` + its arm in `run` (`verse_actions.rs`); E1
  adds "Pridať do kázne" there and "Kázne" in the header, S4 "Porovnať" and "Odkazy", S5 the word sheet.
  A new view is a `StudyView` variant. Scope chips and the quoted phrase wait for D4's server
  support: never filter on the client.
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
- **Local UI builds need nightly** (`.cargo/config.toml` has `build-std`): `RUSTUP_TOOLCHAIN=nightly
  trunk build`; on stable, wasm-bindgen fails with `__wbindgen_externref_table_alloc`.
