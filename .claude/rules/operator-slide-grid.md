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
  - "crates/presenter-ui/src/components/slide_reorder.rs"
  - "crates/presenter-ui/src/components/slide_selection.rs"
  - "crates/presenter-ui/src/state/song_load.rs"
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
  - the `slide_list.rs` effect that follows stage slide changes with
    `scroll_slide_into_view` reads `ctx.mode` UNTRACKED and scrolls only in
    Live mode (it still returns the current id, so going back to Live never
    replays an old change); the only Edit-mode scroll to the stage slide is a
    song the operator's own Follow ON click opened;
  - per-card state that a stage event re-renders (the card `class` closure)
    derives everything it shows, `is-focused` included, from signals — never
    poke a class into the DOM, the next stage event rewrites `class`.
- **AbleSet Follow** (`components/stage_follow.rs`; the operator page feeds
  every stage snapshot to `on_stage_snapshot`, the header's Follow ON click
  calls `sync_to_stage(.., forced)`): stage events switch songs only in Live
  mode (`follow_navigation`; Follow counts as OFF until its status loads); on
  the switch back to Live the operator syncs to the stage at once, so an arrow
  key never sends a slide of the song that was being edited; the Follow ON
  click also opens in Edit mode, but nothing opens once Follow is off. An open
  is recorded in `AppContext::follow_pending` (token, request number, the
  selection and the library/playlist context it started from, forced, entry
  index, last request time) while its song loads; the selection, `currentPresentationId`,
  `selected_entry_index` and the content are applied TOGETHER only when the
  song arrives, the token is current and the selection unchanged. NEVER move
  `selected_presentation_id` before the content: every round-2…4 race came
  from doing that (a failed fetch left the id on a song that was never shown,
  so Follow never retried). A stale open is simply dropped: a newer open, the
  stage leaving that song (in ANY mode — `on_stage_snapshot`, so a Follow ON
  open in Edit mode cannot land after the stage moved on), the operator
  picking a song (a click or a search result clears `follow_pending`, so even
  a failed click that returns the selection cannot revive it), a failed fetch
  of the LATEST request, Follow off / Edit by then. In Live mode the
  next stage change opens again (after an Edit drop the switch back to Live
  does); an open still waiting after `STALE_OPEN_MS` (5 s) is requested AGAIN
  under the SAME token with the next request number, so the first SUCCESSFUL
  answer opens the song and only a failure of the latest request ends the
  open (a new token would discard a slow but live first answer).
  songplayer pushes to an API layout (a stage frame with NO song,
  `is_api_stage_layout`) are not stage changes for Follow: `on_stage_snapshot`
  skips them, and the catch-up paths (back to Live, the Follow ON click, the
  landing scroll) read `AppContext::follow_stage` — the last stage frame that
  is not such a push, seeded at page load from the camera-crew snapshot
  (which always carries the stage song) unless a live frame came first —
  never `ctx.stage_snapshot`, which during songplayer playback holds the
  push. On an API layout a presenter trigger publishes ONLY the camera-crew
  frame.
  A stage frame with no song opens nothing and keeps the operator's picked
  occurrence. An open whose song is already SHOWN when its answer arrives
  (it stayed on screen while a click or search of another song loaded, or a
  failed load moved the selection back to it) moves the selection, the
  occurrence and the library context there, but NEVER the content — its
  answer may predate an edit (#515). The decisions are pure functions with host unit tests: `sync_step`,
  `may_navigate`, `songless_api_frame`, `stage_left_open`,
  `answer_ends_open`.
  The library context (library id, playlist clear, `activeLibraryId`,
  `activePlaylistId` removed, `context_title`, song list) is applied together
  after `get_library`, only if the song is still selected and the library /
  playlist are unchanged since the OPEN STARTED (a pick while the song loads
  counts); Follow's own library step moves a still-pending open's
  `context_at_start` along, so it is never taken for an operator pick. Accepted: during that `/libraries/summary` round trip the trigger
  context still carries the old playlist id (clearing it earlier would flash
  the old list), and if `get_library` fails the old context stays.
- **No late answer may replace a newer song.** A song-list click, a search
  result and the session restore load through
  `state::song_load::show_when_loaded` (the #515 seq guard, the #556 F6
  retry, `is_still_selected`); a song that cannot be shown never stays
  selected (`OnFailure::SelectShown` puts a click back on the shown song AND
  its playlist occurrence — `AppContext::shown_entry_index`: when a song is
  shown it records the occurrence selected at that moment (the click's, or a
  stage mirror that came during the load), and the stage mirror updates it
  only while its song is shown — never the occurrence of a click still
  loading, #496; `ClearSelection` empties the session
  restore's selection but keeps its session key, so Follow can open it; a
  load gives up only while no other load of the same song is in flight — a
  double-click loads twice and either failing must not undo the other
  (`IN_FLIGHT`) — and never for a re-click of the song already shown (its
  clicked occurrence stays); a Follow open that started while the failing
  song was selected is re-pointed at the new selection — the move is not an
  operator pick);
  every structural edit (insert / duplicate / delete /
  paste via `slide_save::apply_structural_slides`, reorder and the reconcile
  refetch via `is_shown`) applies only while its song is still SHOWN.
  Otherwise the shown and selected songs disagree, or one song's slides land
  in another, and an arrow key sends the wrong slide.
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
- `operator-edit-autoscroll.spec.ts`: each case fails on the code it fixes
  (cases marked `// Guard` cover fixes that shipped without one) — Edit-mode
  scroll / focus / Follow-in-Edit, back to Live (also on an API layout after
  a songplayer push), the network-race cases (Edit or a song click while the
  stage song or its library loads, a failed / hung / slow re-requested stage
  song, an old request failing after a re-request, the stage bouncing back,
  a late manual / search / session-restore answer vs a Follow open, Follow
  ON→OFF, a stale Follow ON open in Edit, a click cancelling a Follow ON
  open, a playlist picked while the song or its library loads, Follow's own
  library switch vs the next open, a songplayer push during an open, a
  Follow open started while a clicked song fails), the reload after a Follow
  library switch, structural-edit answers after a song switch (duplicate,
  reorder, the raced-duplicate reconcile refetch), failed manual / search /
  session loads (incl. the playlist occurrence after a failed click, with a
  click still loading, mirrored during a load, and after a songless
  catch-up, #496), a double-click whose first or second load fails, a failed re-click of the shown song, Follow back to the shown stage song from a playlist that lacks it, an old Follow ON
  answer for the song that ends up shown, Follow ON right after a reload on
  an API layout, and the manual-open-starts-at-the-top cases. Idioms: hold a request with
  `gateRoute` (+ `waitForRequest`) and release it only after the competing
  event is CONFIRMED in the DOM — never a fixed sleep; before a "nothing
  changed" check wait for the late answer's BODY (`landed()` =
  `response.finished()`) and `settle`; a network failure is a MALFORMED 200
  (`not-json`), never a 5xx (Chrome logs every non-2xx and the specs assert a
  clean console).
