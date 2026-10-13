---
paths:
  - "crates/presenter-ui/src/components/slide_columns.rs"
  - "crates/presenter-ui/src/components/slide_list.rs"
  - "crates/presenter-ui/src/components/slide_list_scroll.rs"
  - "crates/presenter-ui/src/state/slide_columns.rs"
  - "crates/presenter-ui/src/state/session.rs"
  - "crates/presenter-ui/styles/operator.css"
  - "tests/e2e/operator-slide-columns.spec.ts"
  - "tests/e2e/operator-edit-autoscroll.spec.ts"
  - "crates/presenter-ui/src/components/presentation_list.rs"
  - "crates/presenter-ui/src/pages/operator.rs"
  - "crates/presenter-ui/src/components/stage_follow.rs"
  - "crates/presenter-ui/src/components/stage_preview.rs"
---

# Operator slide grids — per-browser slides per row (#832)

- **One setting, every grid.** `OperatorState.slide_columns` (1–8, `None` = no
  choice) is applied by `pages/operator.rs` to `body` as the inherited
  `--operator-slide-columns-choice` (`components::slide_columns::apply_slide_columns`).
  `.operator__slides` uses `repeat(var(--operator-slide-columns-choice,
  var(--operator-slide-columns)), …)`, with `--operator-slide-columns` = 3 and the
  ≤480 px query setting only that DEFAULT to 2 — so an explicit choice wins on a
  phone and no choice keeps the phone at 2. Never set `grid-template-columns`
  directly in a media query again (it would beat the choice).
  `.operator__slides--clipboard` (single column while choosing a paste target)
  comes later in the file and must keep winning.
- **Storage.** The choice is a normal persistent setting
  (`session::get_persistent` / `set_persistent`, key `operatorSlideColumns`).
  Since #832 `state/session.rs` talks to `web_sys::Storage` directly and NEVER
  throws: gloo's `LocalStorage::raw()` threw when storage was blocked (private
  mode) and killed `OperatorState::new` before any default applied. Values stay
  JSON strings (`"\"5\""`) — the format gloo wrote and the E2E specs seed
  (`localStorage.setItem("presenter:operatorSlideColumns", JSON.stringify("5"))`).
- **The stepper follows what the grid shows.** Without a choice it shows and
  steps from `default_slide_columns(viewport width)` (≤480 px → 2, else 3),
  tracked through a `resize` listener removed on unmount, so "+" on a phone goes
  2 → 3, not 3 → 4.
- **Readable at 8.** 6+ per row sets `body[data-slide-columns-dense="true"]`
  (tighter cards, smaller type, `overflow-wrap: anywhere`) and grid children
  have `min-width: 0` — nothing may widen the grid past its container.
- **Anything that assumes 3 columns is now wrong.** The #271 next-row lookahead
  (`slide_list_scroll::columns_per_row`) reads the grid's computed
  `grid-template-columns` (`state::slide_columns::track_count`); the
  5-per-row case is pinned in `operator-slide-scroll.spec.ts`. Grep for a
  hard-coded column count before adding grid logic.
- **E2E.** Count columns only on VISIBLE grids (`offsetParent !== null`): a
  hidden grid's computed `gridTemplateColumns` is the specified `repeat(…)`
  text, not resolved tracks. The control renders in both the worship and the
  Bible toolbar — select it with `:visible`.
- **In Edit mode a stage change never moves the operator's place (#853).**
  AbleSet advances the stage every few seconds during a rehearsal; on SNV an
  operator editing slides on the FOH PC saw "the page refresh and jump to the
  top". Two rules keep Edit mode still:
  - the `slide_list.rs` effect that calls `scroll_slide_into_view` reads
    `ctx.mode` UNTRACKED and scrolls only in Live mode (it still returns the
    current id, so going back to Live never replays an old change);
  - per-card state that a stage event re-renders (the card `class` closure)
    derives everything it shows, `is-focused` included, from signals — never
    poke a class into the DOM, the next stage event rewrites `class`.
- **AbleSet Follow** (`components/stage_follow.rs`, used by the operator page
  and the header's Follow button): stage events switch songs only in Live mode
  (`follow_navigation`; Follow counts as OFF until its status loads); on the
  switch back to Live the operator syncs to the stage at once, so an arrow key
  never sends a slide of the song that was being edited; an explicit Follow ON
  click syncs in any mode (`forced`). An open is recorded in
  `AppContext::follow_pending` (token, the selection it started from, forced,
  entry index) while its song loads, and the selection, `currentPresentationId`,
  `selected_entry_index` and the content are applied TOGETHER only when the song
  arrives and the open is still current. NEVER move `selected_presentation_id`
  before the content: every round-2…4 race came from doing that (a failed fetch
  left the id on a song that was never shown, so Follow never retried). A stale
  open (newer open, stage back on the selected song, operator picked a song,
  fetch failed, Edit/Follow off by then) is simply dropped — the next stage
  change retries. The library context (library id, playlist clear,
  `activeLibraryId`, `context_title`, song list) is applied together after
  `get_library`, only while that song is still selected.
- **Scroll on a newly shown song** (`slide_list_scroll::setup_scroll_on_song_shown`,
  keyed on the SHOWN song, not the selection id): to the top (#271 concern 3),
  or — only while Follow navigates — to the stage's current slide when it is in
  that song. A manual click of the stage song with Follow off opens at the top.
- The song list reads the stage song through a `Memo` of `presentation_id`
  (`presentation_list.rs`), so a stage SLIDE change does not re-run its render
  closure (tachys already patched the rows in place, so this is CPU, not DOM:
  the "same row node" assertion passes on the old code too).
- `operator-edit-autoscroll.spec.ts`: the "editing the song on stage" case
  (1605 → 806), the "Follow on" case (the edited song closed), the "back to
  Live" test, the two network-delay tests (Edit clicked while the stage song
  loads; another song clicked while its library loads — both gated on
  `waitForRequest`/`waitForResponse`, never fixed sleeps), the failed-fetch
  retry, the "selection stays on the shown song while the stage song loads"
  case and the Follow-off manual click (#271) each fail on the code they fix;
  the library / playlist / other-song cases guard against reloads, jumps and
  row rebuilds. A network-failure case fulfils a MALFORMED 200 (`not-json`),
  never a 5xx: Chrome logs every non-2xx and the specs assert a clean console.
