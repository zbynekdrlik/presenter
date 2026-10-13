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
  - "crates/presenter-ui/src/components/search.rs"
  - "crates/presenter-ui/src/components/slide_save.rs"
  - "crates/presenter-ui/src/state/mod.rs"
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
  never sends a slide of the song that was being edited; the Follow ON click
  (`forced`) also opens in Edit mode, but nothing opens once Follow is off. An
  open is recorded in `AppContext::follow_pending` (token, the selection it
  started from, forced, entry index, start time) while its song loads; the
  selection, `currentPresentationId`, `selected_entry_index` and the content
  are applied TOGETHER only when the song arrives, the open is still current
  and the selection has not changed. NEVER move `selected_presentation_id`
  before the content: every round-2…4 race came from doing that (a failed
  fetch left the id on a song that was never shown, so Follow never retried).
  A stale open (newer open, stage back on the selected song or cleared,
  operator picked a song, fetch failed, Follow off / Edit by then) is simply
  dropped; the next stage change opens again (after an Edit drop the switch
  back to Live does), and an open still waiting after `STALE_OPEN_MS` (5 s, a
  hung request) is re-opened. The decision is the pure `sync_step` /
  `may_navigate` (host unit tests). The library context (library id,
  playlist clear, `activeLibraryId`, `activePlaylistId` removed,
  `context_title`, song list) is applied together after `get_library`, only
  if the song is still selected and the operator has not switched library or
  playlist since the song was shown. Accepted: during that one
  `/libraries/summary` round trip the trigger context still carries the old
  playlist id (clearing it earlier would flash the old list).
- **Every manual open checks the song is still selected before showing it**
  (`state::is_still_selected` in `presentation_list.rs` select, `search.rs`
  result click, the `pages/operator.rs` session restore; the `slide_save.rs`
  reconcile refetch checks the song is still SHOWN): a late answer for a
  song the operator or Follow has since replaced must not be shown, or the
  shown and selected songs disagree and an arrow key sends the shown song's
  slide.
- **Scroll on a newly shown song** (`slide_list_scroll::setup_scroll_on_song_shown`,
  keyed on the SHOWN song, not the selection id): a song a Follow open just
  showed (`AppContext::follow_landing`, set right before `selected_presentation`
  and consumed on every run of the effect) lands on the stage's current slide
  when it is in that song; every other newly shown song (a click, search, a
  playlist, a reload) opens at the top (#271 concern 3), Follow on or off.
- The song list reads the stage song through a `Memo` of `presentation_id`
  (`presentation_list.rs`), so a stage SLIDE change does not re-run its render
  closure (tachys already patched the rows in place, so this is CPU, not DOM:
  the "same row node" assertion passes on the old code too).
- `operator-edit-autoscroll.spec.ts`: each case fails on the code it fixes —
  the "editing the song on stage" case (1605 → 806), the "Follow on" case
  (the edited song closed), "back to Live", the two network-delay tests (Edit
  clicked while the stage song loads; a song clicked while the stage song's
  library loads, started from the playlist), the failed-fetch retry, the
  hung-request retry, the "selection stays on the shown song" + stage bounce
  case, a late manual answer vs a Follow open, Follow ON→OFF before the song
  arrives, a playlist opened while the library loads, the reload after a
  Follow library switch, the reconcile refetch after a raced duplicate, and
  the manual-open-starts-at-the-top cases (Follow off and on); the library / playlist / other-song cases guard against
  reloads, jumps and row rebuilds. Gate a delayed request with `gateRoute` +
  `waitForRequest`/`waitForResponse` and release it after the competing event
  is CONFIRMED in the DOM — never a fixed sleep. A network-failure case
  fulfils a MALFORMED 200 (`not-json`), never a 5xx: Chrome logs every non-2xx
  and the specs assert a clean console.
