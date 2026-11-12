---
paths:
  - "crates/presenter-ui/src/pages/study/**"
  - "crates/presenter-ui/src/state/study.rs"
  - "crates/presenter-ui/src/state/study_history.rs"
  - "crates/presenter-ui/src/state/study_word.rs"
  - "crates/presenter-ui/src/state/study_field.rs"
  - "crates/presenter-ui/src/api/study.rs"
  - "crates/presenter-ui/styles/study.css"
  - "crates/presenter-server/src/router/study_pwa.rs"
  - "crates/presenter-server/src/ui/study_sw.js"
  - "tests/e2e/study-*.ts"
  - "crates/presenter-server/src/router/bible/study.rs"
  - "crates/presenter-server/src/state/bible_source/study.rs"
  - "crates/presenter-ui/src/pages/study/bookmarks.rs"
  - "crates/presenter-ui/src/state/study_bookmarks.rs"
  - "crates/presenter-ui/src/api/bookmarks.rs"
  - "crates/presenter-server/src/router/bible/bookmarks.rs"
  - "crates/presenter-server/src/state/study_bookmarks.rs"
  - "crates/presenter-persistence/src/repository/bible/study_bookmarks.rs"
---

# The study page `/ui/study` (#851 S1–S5)

The preacher's reader, replacing MySword on his phone and tablet: reading, searching and sermon
preparation (the sermon views are in `sermon-prep.md`).

- **Nothing here reaches a wall (T11).** No trigger, clear or stage call; every test in the study
  specs ends with `finish()` (`study-support.ts`), which asserts that and the clean console. The test
  that passes through the operator empties `wallRequests` once it is back: the operator polls the
  stage.
- **Entry from the operator = the Stránky menu item "Štúdium"** (#875 H1, same tab), never a link in the
  Obrazovky surface list (those open a new tab). The page is also entered from `/`.
- **A chapter comes from `GET /bible/study/chapter`, in ONE function** (`api/study.rs::load_chapter`).
  An empty `verses` = this translation lacks it; specs gate it with a URL regex, it has no body.
- **Every view leads back to the verses without the history:** a tab reopened on a sermon or the
  sermon list has no ◀ and no field, so the header adds `study-bible` "Biblia" on those views
  (`sermon_pane::reading_route` = the last reading place, its saved verse) beside Kázne (owner,
  2026-11-06). A new view without the field needs the same way out.
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
- **A tap on a verse's text highlights it** (`reader::on_text_tap`, `v=<n>` by `nav::replace`, no
  history entry; a second tap lets go; with the verse sheet open it grows the selection like the
  number; with Čísla on a KJV word tap highlights AND opens the word sheet, `VerseBody`'s `on_tap` runs first; with Čísla off a word is plain text, #913).
  Owner 2026-11-06: "aby bolo jasné, na ktorý sa fokusujem".
- **The header names the chapter on screen** (`study-place`, #913): `Chapter::Ready` in the reader, else "Štúdium". A long book gives way, its chapter never.
- **The field and the chips share one row** (`.study-tools`, #913). Up to 932 px the field leaves room while it has no focus (✕ and ▾ hidden, so the whole field is one target: at 360 px ▾ reached its middle and `tap()` hung to the 180 s limit on CI's slightly wider chips only), and with focus takes the row (`data-focused` + a sibling rule, not `:focus-within`: a tapped chip takes the focus too). `.study-suggest` still starts at 100 px.
- **Every installed translation is one tap behind ▾** (#921, owner 2026-11-09: "chcem mať tieto novo
  pridané preklady aj v štúdiu"): the favourites row = the favourites that fit
  (`state::bible_translations::study_chip_cap`: 4 under 420 px — `Study::narrow`, one `watch_media`
  at page start, so the field keeps ~94 px at 360 — else 5) + `study-translations-open` ▾, which opens
  the `s=preklady` sheet (`StudySheet::Translations`, `components::translation_list`, grouped by
  language, short code + full name). A pick is the chip's `switch()` (keeps the verse in place) and
  never changes the favourites; the open translation outside the shown chips takes the LAST chip's
  place (`favourite_row`, the chip keeps that slot, so a long press there still sets the slot). Long
  press keeps customising slots. Compare's column `<select>` already lists every translation. E2E:
  `pickTranslation` (`study-support.ts`) always goes through ▾ (a chip is tapped with `favourite`); the NLT, the fifth
  favourite, is in ▾ on a 360/390 phone. `study-translations.spec.ts` routes SNV's four extra
  translations in from the KJV (`tests/e2e/extra-translations.ts`).
- **A translation switch keeps the eye's verse at its screen offset** (`ScrollPlan::VerseAt`): the
  highlighted verse while it shows, else `verse_in_view`. `verse_in_view` reads 2 px below the
  landing gap (`SLACK_PX`): a landed verse sits at the gap and the verse before ends there plus a
  fraction of a pixel, so without the slack every switch stepped one verse back (20 → 16 in seven).
  `land` rounds its whole-pixel scroll step DOWN (`scroll_step`): rounding 412.53 up left the verse
  0.47 px above the reader's top and failed the `>= 0` E2E bound on CI only.
  `verse_offset` rounds DOWN too (`screen_offset`): read to the nearest, a verse landed at 172.8 read
  173 and every switch walked it a pixel down the screen. Read and landing must agree.
- **Its own app:** manifest + icons under `/ui/study/` (open book, `study_icons/`, not the tablet's
  "B"), `<link rel=manifest crossorigin="use-credentials">`: a cookie-less manifest fetch behind
  Cloudflare Access (`prsnv.newlevel.media`) gets the login page and Android offers no install. Dev2
  and the church sit on the Access bypass IP, so only a phone off-site shows that case.
- **A verse number works only on the route's chapter** (`reader::chapter_settled`): during a load
  the URL has moved on and the old text is on screen, so numbers are disabled and the sheet reads
  nothing from a chapter the route does not name.
- **Back from a hit keeps the list:** `Search::Ready` stays for its key, nothing is asked again, and the
  offset by history index restores the scroll. Back with no offset: the route's verse, else the top.
- **A failed `/bible/books` is not silent** (`structures_failed`): inline card + retry, a neutral bar
  (never Začiatok/Koniec), and `suggest_unresolved`: a reference gets a hint, only words naming no
  book may search, or the reader gets a false "Nič sa nenašlo".
- **`touch-action: pan-y pinch-zoom`** on the reader: pan-y alone turned pinch-zoom off.
- **A verse number is a control (T3), the first thing on its verse's line** (#932, owner 2026-11-11:
  "zbytočne veľké odsadenie zľava a sprava"): `VerseBody`'s `lead` puts the button first in
  `p.study-line`. Phone shell: 10 px edges, the lines after the first run to that edge, the first
  starts at 40 px (`text-indent`, reset on the line's children); the button is a real 44 × 44 box,
  absolutely placed on the first line's middle from the screen's edge (`assertMinTapTarget`). Never
  a float with negative margins there: Chrome keeps it as an exclusion over the SECOND line too
  (measured: line 2 at 31 px). Specs read the glyph lines with a `Range`, never the `p` box.
  Desktop hangs it as a float in the 44 px gutter (negative margin, zero net width). Its click stops propagation: a number tap is never also a text tap. The
  reader has no chapter title (#932): the header's `study-place` names it, the chip the translation.
- **A translation lacking a place** answers no slides: `Chapter::Missing`, and `step_chapter` walks to
  its nearest chapter by the canonical book number, so the reader never gets stuck.
- **The field keeps an opened reference** (`Field::opened`): while that chapter is read, in any
  translation, the typed text stays so "2kor 5" is fixed to "1kor 5" in place (owner, 2026-11-06);
  another chapter, view or Back empties it (not while the field has focus). One `FieldPlace` effect
  owns the field's text for both this and the search query.
- **✕ and ▾ in the field** (owner, 2026-11-06): ✕ clears and keeps the focus; ▾
  (`field_recent.rs`) lists this browser's earlier searches (`state::study_recent`, localStorage
  `studyRecent`, 12, newest first, case/space duplicates move to the top). What is kept: the typed
  text on Enter, a row's label, "Book N" for a chapter cell, the Strong's number, the search words.
  A pick only FILLS the field (Enter opens it); typing, a blur or Escape closes the list.
- **A tap on a † is the note's** (`tapped_note`), never the verse's. **A chip tapped during a load**
  keeps the waiting `ScrollPlan`: what is on screen is the chapter being left. **`nav::replace` keeps
  `listed`** unless a sheet closes: a highlight on a closed sheet's step is no second row. **Whatever
  opens a passage leaves its text in the field** (`Field::open`), a row's label or "Book N" too.
- **The field shares the console's row builder** (`console_view`) and adds only the chapter grid and
  the text search (`state::study::suggest`; a word naming a book offers both, Enter opens the book). `Suggestions` shows while the field has focus and
  prevents `mousedown`'s default, or the blur removes a row before its `click`.
- **No dead buttons.** New actions are one `VerseAction` + its arm in `run` (`verse_actions.rs`).
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
  Resolume binds 8091). Run `bash scripts/build-ui.sh` FIRST: the server's `build.rs` never runs
  trunk, it only embeds `presenter-ui/dist` (re-run when `dist/index.html` changes), so a dist from
  build-ui.sh is patched and `finish()` sees no `integrity` warnings (#936, 2026-11-12: 159 study /
  sermon tests clean locally). A bare `trunk build` dist still gives those warnings.
- **Local UI builds need nightly and `--release`** (`.cargo/config.toml` has `build-std`):
  `RUSTUP_TOOLCHAIN=nightly trunk build --release`; on stable, wasm-bindgen fails with
  `__wbindgen_externref_table_alloc`, and a debug build floods the console with reactive warnings.
- **`refresh-dev-data` refuses as stale:** `cargo build --release -p presenter-importer --bin
  import_propresenter --bin ingest_bibles`.

## Named bookmarks (#915, `pages/study/bookmarks.rs`, `/bible/study/bookmarks*`)

- **Server-side, one list per reader e-mail** (`AccessEmail`, `.claude/rules/access-identity.md`): a verified Access
  login, or the device cookie that login left (so the same phone keeps its list through the church's Access bypass,
  ROZHODNUTÉ 1, 2026-11-09). NULL = the shared list: the LAN address, a device that never logged in. An id of
  another list is 404. The e-mail is a convenience scope, never a protection: bookmarks are no secret.
- **Rules live in `state/study_bookmarks.rs`** (`BookmarkRefusal` → 422, Slovak): trimmed name 1–80 chars, ≤ 100 per
  list, a place the translation really has (`bible_book_chapter_summaries`, so the NLT is checked against its
  structure with no request); the refusal names the book as the translation does. The row stores `book_name`.
- **"Pridať záložku" adds in ONE tap** (#932, owner 2026-11-11: "Načo je to dobré?!" about the naming sheet;
  `bookmark-save` was pressed unchanged 5 of 6 times): `bookmarks::add_here` posts the place name
  (`offered_name`, "Hebrejom 11:4"), closes the verse sheet, notice "Záložka pridaná: …". It is the verse
  sheet's FIRST action; the rest follow the SNV phone counters. No `s=zalozka` any more (an old link opens
  the verse, no sheet); naming = Premenovať in `s=zalozky` (the header icon, hidden on the sermon views for
  the 360 px header).
- **The list sheet starts every visit in `Loading`** and carries `data-state=loading|ready|error` (the spec's settle
  marker); its keyed `<For>` sits OUTSIDE the state match, so a reload after Späť keeps an open rename field.
- **Späť after Zmazať** re-creates at the deleted `index` (POST `index`), the repository renumbers 0..n in one
  transaction. Lookups and `study.last` never touch bookmarks.

## Compare and cross-references (#851 S4, `pages/study/{compare,crossrefs}.rs`)

- **Compare is a view, its ◀ ▶ a `nav::replace`:** Back leaves it at once, the verses are no history
  rows. Stacked = phone shell; columns = `CompareLayout` (2, 3 at ≥ 1024 px), the spec's aligned grid.
- **Compare's bar is `◀ Čítať ▶` (#915):** Back alone cannot leave compare opened with no history (a
  reopened app), and the usage counters showed the bar's place text pressed as an exit. `Čítať` opens
  the lead translation at the verse; the place lives in the header (`compare::header_place`).
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

## Word sheet and Strong's view (#851 S5, `pages/study/{markup_view,word_sheet,strongs_view}.rs`)

- **`VerseBody` is the ONE verse renderer** of the reader and compare. A tap is read once per verse by
  delegation (`tapped_word`), not one closure per word; the chip re-renders a verse, one closure each.
  Badges are rendered only while on: a hidden badge would still be `textContent` and break
  `toContainText("Through faith")`. `text` stays what a copy or a sermon takes.
- **`w`/`wv` is the place, `w` may be foreign:** `wv` must name a tagged word on the page (`find_word`),
  `w` may be a derivation link's number. `word_sheet::install` drops a place with no word once the text
  answered (`Gone`, never while `Waiting`), so a bad URL leaves no sheet and a reload keeps a good one.
  Compare's page = the KJV column, on a phone only the verse shown: ◀ ▶ stay in it.
- **Links add entries, ◀ ▶ and tabs replace.** `popstate` uses `only_sheet_differs`, which also sees a
  change of word under the same sheet, or Back from a link scrolls. ✕ and the backdrop walk the whole run
  of word entries (`close_word_sheet`), only Back walks one. A list starting on the sheet: the walk ends on
  its first entry and `AfterPop::CloseSheet` closes it there, one ✕. "Všetky výskyty" walks the links first
  (`leave_word_sheet`), or a later Back reopens one; a `popstate` finishes what a walk left (`after_pop`).
- **A failed read keeps the word:** `TextFailed`, never `Gone` (only a Ready text with no such word or a
  chapter the translation lacks, or no KJV column in compare). The sheet has its own retry: a phone's
  backdrop covers the text's.
- **The modal word sheet reserves no space at ≥ 1024 px** (`:not([data-role="word-sheet"])`): §6.6, the
  reading position never moves. The other docked sheets still do.
- **A click ending a mouse selection is no tap** (`text_selected`).
- **No KJV word has two numbers in today's data** (0 of all verses): the tab test mocks a chapter.
- **The Strong's list stays in `Study::occurrences`:** Back asks nothing. A page lands only on the list
  that asked for it, right after what it holds (`land_page`). A scope chip is a `nav::replace`. The chips
  read `Study::book_counts`, so they outlive a scope change and a failed read. On a phone they are one
  sideways row (`assertNoHorizontalOverflow` flags such a row, so the spec checks the page itself).
- **`study-*` local E2E pace:** every test fails `finish()` on the local `integrity` line, so each one
  starts a new worker and re-imports: ~30 s per test.

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

## Sermon specs

- **A spec opens its own sermon by id,** never `.first()`: the server lives through the whole file and the list runs by service date, so an earlier test's sermon comes first. Only the retry's fresh database hid this.
- **Repeating a spec locally:** the local-only `integrity` warning fails every test at `finish`, so each repeat restarts the server (~20 s). Filter it in a throwaway edit first. A `[pageerror]` line now carries the stack: read it before calling a failure a scroll or timing problem.
- **Search scope** lives in the URL (`scope=`, left out for `all`) and in the settle marker (`slk-seb|HEB:viera`, the scope before the query: a query holds any character); the book chip is the chapter still held behind the search, so a search opened from a bare URL has none. A quoted query matches the server's phrase rule, curly quotes included: `mark_matches` marks the whole run.

## No `on:scroll` on a view's scroller (#851)

Leptos 0.7 drops an `on:` closure with its view but leaves the listener on the element; the browser still delivers a scroll queued for that `<main>` (the reader landing on a verse, then Back in the same frame) → page error "closure invoked recursively or after being dropped" (intermittent CI flake in `study-compare`). The reader uses `use:save_on_scroll=(s, ticket)` (`scroll.rs`): its own `Closure`, removed in `on_cleanup` (cleanups run before the owner's stored values go). Spec: dispatch `scroll` on the detached element after Back.
