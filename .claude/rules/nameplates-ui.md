---
paths:
  - "crates/presenter-ui/src/components/nameplates/**"
  - "crates/presenter-ui/src/components/stream_editor/editor_nameplates.rs"
  - "crates/presenter-ui/src/components/stream_editor/output_switch.rs"
  - "crates/presenter-ui/styles/nameplates.css"
  - "tests/e2e/operator-nameplates.spec.ts"
  - "tests/e2e/stream-editor-tabs.spec.ts"
  - "tests/e2e/stream-editor-output-select.spec.ts"
  - "tests/e2e/companion-socket.ts"
---

# Nameplate ("Menovky") plate list: one component, two places (#871)

## Where things live
- `components/nameplates/mod.rs` — `NameplateStore` (`Copy`): one output's plate list, the
  plate on air, the live song, and EVERY plate REST call (list / add / PATCH / DELETE /
  reorder / show / hide) plus the request DTOs and paths. Never a second copy elsewhere:
  the editor's `output_paths.rs` no longer has plate paths.
- `components/nameplates/plate_list.rs` — the shared views: `NameplateSongRow`,
  `NameplateRows` (keyed `<For>` by plate id, owns the drag state), `NameplateAddForm`.
  `RowOptions` carries the per-page extras as `fn` pointers (`preview`, `after_hide`) +
  `numbered`, so it stays `Copy`.
- `components/nameplates/order.rs` — pure order logic: ↑/↓ step, drop slot, drop marker,
  local `apply_order`. `drag.rs` — the DOM side of the drag (pointer capture, row boxes).
- `components/nameplates/focus.rs` — `keep_focus` (the list keeps its focus across a row
  move), `rows_moving` (a move is under way) and the pure `blur_kind` (moved / focus away /
  left).
- The plate saves, three files (one owner each, see "Saves" below):
  `save_control.rs` — the pure, host-tested `SaveControl` (tests in `save_control/tests.rs`):
  per plate the desired texts, the PATCHes on their way (each with a token), the confirmed
  texts, the failure streak and a waiting Zobraziť; plus `SaveOutcome` (how a save ended),
  `TextLeft` (where its typed text is now), the toast texts and `retry_delay_ms`.
  `saving.rs` — the store's side (`impl NameplateStore`): runs every controller `Step` (a
  PATCH, a retry timer, a toast, a show, a list reload) and keeps `save_views`, the
  `PlateSave` each row reads (`unsaved`, `settled`). `row_save.rs` — the row's side:
  `PlateText` (one field's draft, its base from the pure `base_text`), `Why`, `RowSave`
  (commit, `nothing_to_commit`, Zobraziť, blur, leave).
- `api/mod.rs::start_patch_keepalive` + `finish_json` — the plate PATCH as a `keepalive`
  fetch started before the call returns (the `ndi_video::dispatch_delete` pattern), aborted
  after `timeout_ms` (an `AbortController` + a timer: `AbortSignal.timeout()` is missing on
  Safari < 16, an iPad Air 2, and its call would throw into the WASM); only the plate saves
  use it.
- `components/nameplates/numbering.rs` — pure Stream Deck numbering (#870: button 1 = song,
  person at list index i = button i + 2, grid 4 × 3 = 12 buttons). Host-tested.
- `components/nameplates/operator_view.rs` — the operator's Menovky view
  (`/ui/operator/menovky`, tab after AI in `header.rs`), always the `stream` output.
- `stream_editor/editor_nameplates.rs` — the editor's Menovky tab = the shared list + its
  extras: Prehrať (`preview_nameplate`) in the tab's own preview frame, Skryť also clears that
  frame, "Vytvoriť menovkovú vrstvu".
- `utils/dom_attr.rs::bool_attr` — the `"true"`/`"false"` attribute helper, shared by the
  editor and the plate list (the shared component imports nothing from `stream_editor`).
- CSS: the list is `nameplates__*` in `styles/nameplates.css` (linked after
  `stream_editor.css` in `index.html`); the operator view's touch sizes (44 px controls,
  16 px inputs) are scoped to `.nameplates-view`, so the editor keeps its dense look.

## Rules
- **The DOM contract is the editor's (#779):** `stream-nameplate-song`, `stream-nameplate`
  rows (`data-nameplate-id`, `data-active`), `stream-nameplate-name` / `-role` inputs,
  `-show` / `-preview` / `-hide` / `-up` / `-down` / `-delete`, the `stream-nameplate-add`
  form. #871 adds `stream-nameplate-drag` (handle), `data-dragging` / `data-drop` on rows;
  operator-only: `stream-nameplate-button` (number) and `data-beyond-grid` (only when
  `numbered`), scoped under `[data-role="operator-nameplates"]`.
- **A row's number follows its POSITION, not its key.** The keyed `<For>` reuses a row across
  a reorder or a delete elsewhere, so the number is a `Memo` over the list by id (#496 rule),
  never captured in `children`.
- **Half-typed text is never overwritten** (`PlateText` in `row_save.rs`): each field shows
  its DRAFT (set on `input`) else its BASE text (the saved text, or the plate's newest
  committed text while a save is pending: see "stands on" below), all `Memo`s. tachys
  re-writes a `String` `prop:value` on every run with no comparison, so a plain closure over
  the whole list would wipe typing on ANY list refresh. Never commit on `change` as well as
  `blur`.
- **The saved-text `Memo` KEEPS its last value when the plate is missing from the list**
  (`Memo::new(|previous| … .or_else(|| previous.cloned()))`). The editor's output switch
  `clear()`s the list BEFORE the rows' `on_cleanup` runs; reading `""` there wiped the role
  of the commit made as the row leaves.
- **Saves: ONE controller per list, the row only owns its drafts** (#871 consolidation, after
  five review rounds of races between row-level and store-level save state matched by TEXTS).
  - The row COMMITS its trimmed texts (`RowSave::commit(Why)` → `NameplateStore::save`) on
    leaving a field, the window or the page, on Zobraziť, and when an editor output switch
    takes the row away. It never sends, retries or tracks a save.
  - `SaveControl` per plate: `desired` (newest commit), `in_flight` (`Patch { token, texts }`),
    `confirmed: Option<Texts>` (the newest answer saved, else the row's saved texts when the
    plate got busy; `None` after a failed save of OTHER texts, see failures below),
    `failure` (count + the number of the newest failure), `show` (a Zobraziť ticket). A
    plate with nothing of that left has NO entry (idle). `send` / `advance` on a plate with
    no entry do nothing (`send` returns `Option<Patch>`: never a PATCH with empty texts).
  - **Strictly one PATCH per plate on its way.** A commit meanwhile only replaces `desired`
    (log `save waits for the one on its way`; the same texts as the PATCH on its way:
    `save of these texts is on its way (…)`). When it is back and nothing else is on its way,
    `desired` goes unless `confirmed == Some(desired)`. Answers are matched by TOKEN.
  - **The one exception: an urgent commit** (`visibilitychange` → hidden, or `pagehide`:
    the page may never run again) sends `desired` NOW beside the PATCH on its way when no
    PATCH on its way carries it (log `save sent now beside the one on its way`). Those two may
    be applied out of order: accepted for this case. The rule above repairs what the page
    sees: an older answer coming last leaves `desired ≠ confirmed`, so it goes once more.
    A save waiting whose ROW HAS LEFT (an editor output switch took it while a PATCH was on
    its way) has no row to commit it: `NameplateRows` listens to the same two events once
    per list and calls `store.send_waiting()` → `SaveControl::send_waiting(has_row)`, which
    sends, beside the PATCH on its way, every plate whose `desired` waits and that is not in
    the shown list (log `… (the page went hidden; its row has left the list)`). Listed
    plates are left to their rows' own urgent commit, so nothing goes twice. That includes a
    plate whose row was REBUILT (the editor switched there and back while the save was on
    its way: the new row has no drafts, the old row's texts wait in the controller).
    **A row with nothing to commit** (no drafts, OR only a name being retyped: name draft
    "", no role draft, `kept as typed, not saved`) calls `RowSave::nothing_to_commit(why)`,
    never a bare `return`: on `PageHidden` `store.send_waiting_of(id)` →
    `send_waiting(|other| other != id)`, only that plate (log `… (the page went hidden; its
    row was rebuilt)`); for EVERY `why` `store.retry_of(id, why.reason())` →
    `SaveControl::retry(id)`, a no-op unless the plate failed with nothing on its way (log
    `failed save sent again (the page went hidden | the field was left | the focus went
    away)`). The two are exclusive (a save on its way or not). Once one sends, its texts
    are on their way, so hidden then `pagehide` send once (both `send_waiting*` go through
    `send_waiting_where`; a second `retry` finds the PATCH on its way). The list-level
    hidden / `pagehide` handler never retries (only `online` / visible do, via
    `retry_failed` → `retry_of`), so nothing goes twice. Zobraziť on a draft-less failed
    row therefore retries through its `commit(FieldLeft)` (logged „the field was left"),
    and `show_saved` then only takes a ticket. Before #871's last round both early returns
    skipped the store: a name retyped in a rebuilt row lost the waiting texts on a tab
    close, and a rebuilt row's failed save waited for the timer (the toast's „keď pole
    opustíte" was false for it). The shown ids
    are read BEFORE `saves.try_update_value` and handed in as plain data, so the closure
    stays pure (style). The real hazard is re-entering `saves` itself inside one of its
    own closures (ui skill: reactive_graph 0.1.8 runs the closure under the value's own
    lock, so a nested `try_*` on the same `StoredValue` silently returns `None` and the
    update is dropped, and `with_value` / `get_value` panic with a misleading "disposed";
    other signals and StoredValues are safe to read there).
  - **A row drops a draft only when the store SETTLES that text** (`PlateSave.settled`, a
    numbered `Settle`, applied once per number by an `Effect` in `RowSave`): a field is
    settled when the confirmed text is the desired one and no PATCH on its way carries
    another; `Settle::Gone` (404) drops both. A draft typed on since (≠ the settled text,
    compared trimmed) stays. Commits of the saved texts on an idle plate send nothing and
    settle the drafts (`Commit::Unchanged`, no list reload).
  - **A field with no draft stands on the controller's `desired` while the plate is busy,
    never the list** (`PlateText::base`). `PlateSave.desired` = `SaveControl::desired(id)`
    (set by `update_view` after every step, `None` when idle); the row reads it through
    `store.pending_texts(id)` (tracked). `base` = that field's pending text (even ""), else
    `saved` (the list's, kept once the plate leaves the list): the pure
    `row_save::base_text(pending, saved)`, unit-tested in `row_save.rs`; `shown` = draft
    else base, and
    `PlateText::value()` (what a commit sends) = draft else base. Why: the list can be
    OLDER than a commit still waiting. A row rebuilt by an editor output switch there and
    back while a save was on its way stood on the list, so a role typed there committed
    the list's older name with it and the newer name was silently put back (#871 review).
    `saved` stays the list's text: an idle plate's commit compares with it, and `leave`'s
    deleted-plate check compares the row's texts with it. Going idle, `finish_save` puts
    the saved answer into the list BEFORE `run` clears `desired`, so the field does not
    flicker. Drafts, the settle rule, „neuložené" and focus are unaffected (a draft wins
    over any base change; no new commit path, so no extra PATCH).
- **An empty name is handled in the row, before anything is sent.** Leaving the field
  (`Why::FieldLeft`: Tab, a click, Enter, Zobraziť) toasts „Meno nesmie byť prázdne." and
  the old name stays (the newest committed one while the plate is busy, kept as the draft;
  else the saved one); a typed role still goes with it. `FocusAway` / `PageHidden` (the
  operator is retyping it) keep the empty name as typed: a typed role goes with the NEWEST
  name (`store.desired`, else saved; log `… the role sent with the newest name (…)`), else
  the row commits nothing (`… kept as typed, not saved (…)`), but its plate's waiting or
  failed save still goes (`nothing_to_commit`, above).
- **A failed save keeps the desired texts and is retried** (`SaveOutcome::of_error`): a
  network error (Wi-Fi asleep, the 15 s abort), a server fault (5xx, 408, 429, an unreadable
  2xx body) or an expired login (401 / 403) is `Failed`; 404 is `Gone` (`DELETED_UNSAVED`,
  every desired / waiting state of the plate dropped); other 4xx `Refused` (the fields show
  the saved texts again). While `failure` is set the row shows `data-unsaved="true"` +
  „neuložené" (`stream-nameplate-unsaved`). A save answered as saved ends the streak.
  Retries send `desired`, never half-typed text: a timer (2 s / 5 s / 15 s for failure 1 / 2
  / 3 of a streak, then none; each timer is armed for ONE failure number and does nothing
  once a save, a commit or another failure came since), the window `online` event and
  `visibilitychange` → visible (both listened to ONCE per list, in `NameplateRows`),
  Zobraziť, any commit, and a row with nothing to commit when its field, the window or the
  page is left (`retry_of`, above). A commit that came while the failed PATCH was on its
  way goes at once. Toasts: „Uloženie zlyhalo: … Text ostal v poli ako neuložený — uloží sa znova sám o
  chvíľu, po obnovení siete alebo návrate na stránku, po stlačení Zobraziť alebo keď pole
  opustíte." (+ the login hint for 401 / 403); „sám o chvíľu, " only while a retry timer is
  armed (`message(…, timer)` = `step.retry_in.is_some()`; none from the 4th failure on, so
  the toast promises none); prefix „Menovka sa nezobrazila. " when a waiting Zobraziť was
  dropped. A failure whose plate is no longer in the shown list (an editor output switch
  took the row) gives up with `LOST_UNSAVED` „Zmena menovky sa neuložila — skúste ju zadať
  znova.", but only once no other PATCH of the plate is on its way (`Failed if !held &&
  slot.in_flight.is_empty()`): with a save sent beside it as the page hid still out, the
  failure falls through to the on-its-way step, is not reported, and that save decides
  (saved = idle, no toast; failed = the give-up toast). Before #871's last round it gave up
  at once, dropped that save and told the operator to retype text the server had.
- **A failed PATCH may have been applied** (a lost answer, the 15 s abort after the server
  wrote it, an unreadable 2xx body; #871 review). A Failed answer whose texts are not the
  `confirmed` ones sets `confirmed = None`, so once nothing else is on its way `desired` is
  sent again even when it equals the texts the page last knew (the PATCH is idempotent):
  the hidden-page side save (B saved, then the older A failing) sends B once more, and a
  waiting Zobraziť fires only after that; typed back to the saved texts after a failure is
  re-sent. A failure answered while a PATCH of the plate is on its way after it (still out,
  or started now: `Step::on_its_way`) is logged (`failure not reported; a save of the plate
  is on its way`), not toasted (`SaveOutcome::is_reported(on_its_way)`, whether or not a
  row still holds the text): that PATCH's answer decides and reports. A saved answer while
  an earlier PATCH of the plate is still out logs `saved; a save of the plate is still on
  its way` (the E2E settle signal below). The row stays marked „neuložené" meanwhile. Accepted residual risk: an older
  failure answered BEFORE the newer save's saved answer is not repaired (the saved answer
  sets `confirmed` again), the same out-of-order risk as the urgent side save.
- **Zobraziť** (`RowSave::show`) first commits the row's drafts (iOS Safari may not blur the
  field on a button tap), then `store.show_saved`: an idle plate goes on air at once; a busy
  one gets a ticket (and a failed save goes again) and goes on air once the plate is idle
  with its desired texts confirmed. Every show / hide of THIS page and every editor output
  switch (`set_active` / `clear()` → `drop_waiting_shows` → `note_show`) makes an older
  ticket stale; a Companion / REST show does not. A waiting show also fires only while its
  plate is in the shown list (`is_listed`, the current output's list): `show_person` PUTs to
  the output shown NOW and the server does not check which output owns a plate. A Failed
  (no retry pending) or Gone end drops the ticket with the prefixed toast.
- **A blur commits only when the field was LEFT** (`RowSave::commit_on_leave`). The blur
  handler reads `focus::rows_moving()` AT ONCE; the decision waits in a `spawn_local` task,
  queued after the focus restore below (`focus::blur_kind`). Only a blur DURING a move whose
  field holds the focus again (active element AND `document.hasFocus()`) was a move (the
  browser fires `blur` when `insertBefore` moves the focused node) and commits nothing. A
  field still active outside a move lost the WINDOW's focus (`FocusAway`: alt-tab, OBS,
  Companion, a locked phone), whatever `hasFocus()` says (Safari / Firefox may still say
  true); a field no longer active was left (`FieldLeft`). Never decide synchronously in the
  blur handler: the half-typed text was saved early on every reorder from elsewhere.
- **A row leaving the list** (`RowSave::leave`, in `on_cleanup`, before its signals are
  disposed): the store's slug ≠ the row's (an editor output switch) → the drafts are
  committed (`FocusAway`; the PATCH is id-scoped, the store carries on without the row);
  else the plate was deleted elsewhere → typed text the server does not have toasts
  `DELETED_UNSAVED` and `store.drop_saves` forgets the plate (a later 404 stays silent).
  Per row two window listeners: `visibilitychange` → hidden and `pagehide` commit urgently.
- **Every plate PATCH is a `keepalive` fetch STARTED before the call returns, with a 15 s
  abort** (`saving.rs::send_patch` → `api::start_patch_keepalive`, `SAVE_TIMEOUT_MS`). Chrome
  cancels a plain fetch when the page unloads (probe: a plain fetch queued from `pagehide`
  never reached the server, every keepalive one did). The abort guarantees a save that never
  answers turns into a failure (the plate is never held forever); an aborted fetch logs
  nothing in Chrome. `page.route` intercepts keepalive requests, and a `route.abort()` logs
  ONE `Failed to load resource: net::ERR_FAILED` line. Residual risk: iOS Safari may freeze
  or kill a backgrounded tab before its handlers finish; Safari before 13 ignores
  `keepalive`.
- **A reorder keeps the focus** (`focus::keep_focus`). The keyed `<For>` moves a row with
  `insertBefore` (a one-step swap re-inserts BOTH rows, tachys `view/keyed.rs`), which drops
  the focus to `<body>`: ↑ / ↓ on the drag handle stopped after one step, and a reorder from
  elsewhere pulled the operator out of a half-typed name. Every list change that can move
  rows (`reorder`, an applied list response) runs inside `keep_focus`: it notes the focused
  element of a plate list (an input also its `selectionStart` / `End`), and after the change
  a `spawn_local` task (queued behind the `<For>`'s render effect, still before the next
  input event) focuses it again and puts the caret back, only while the focus fell back to
  `<body>` (`utils::window::focus_is_free`) and the element is still on the page. The row
  DOM is reused across the move (key = plate id), so the noted element is the one to focus.
  Not `requestAnimationFrame`: a second key press can arrive before the frame and go to
  `<body>`.
- **List responses are sequenced.** `list_gen` is bumped by every list request AND every local
  change (reorder, saved PATCH, `clear`); a response applies only while its number is the
  newest and no reorder PUT is pending. A saved PATCH bumps it only when the controller knows
  the save (`finish_save` asks the controller first): the late 2xx of a forgotten save (after
  a 404 `forget`) would drop the reload that forget started, leaving the deleted plate listed. Reorders show at once (`apply_order`) and their PUTs
  are serialized in `send_order` (only the newest waiting order is sent next), so two quick
  ↑ taps move two places and the server ends on the last chosen order.
- **Drag = pointer events on a handle** (`touch-action: none`), the repo's touch-capable
  pattern (`interpreter/tips.rs`, the canvas overlay); HTML5 DnD never fires on a phone. The
  drop sends the FULL id set through the same `reorder` path as ↑/↓; a drop on the row's own
  place sends nothing; a move with no button held ends the drag. The handle also takes ↑/↓
  keys. No auto-scroll while dragging: a far move on a long phone list uses ↑/↓.
  `PlateDrag` holds the id and the slot only: the drop line reads the dragged plate's
  place from the list as it is NOW, so a plate deleted (or moved) elsewhere mid-drag leaves
  no line, and an `Effect` in `NameplateRows` ends a drag whose plate is gone (its handle is
  gone, so no release or cancel would ever reach it).
- **The store owns the slug.** It holds the editor's `output_slug` signal itself, so an output
  switch reloads the right list; `output_switch.rs` calls `plates.clear()` first (the old
  output's rows and plate on air go at once), then `reload_list()` / `reload_active()`; a
  response for a slug that is no longer current is dropped.
- **Live updates go through `NameplateStore::apply_event`** (`StreamNameplate`,
  `StreamNameplatesChanged`, `Stage`). Both pages forward their socket's events to it; the
  operator view also refetches the `stream` def on a newer `StreamConfigChanged` (the
  no-layer hint, a stale def dropped) and reloads everything after a socket reconnect.
- **The song line** (`song_line`): "title · library", "—" with no song; since #871 a song with
  no library shows its title alone (the pre-#871 editor printed "title · ").
- **The toast is a `Callback<(String, &'static str)>`** built before the page context exists
  (`show_editor_toast` / `state::show_toast_on` take only the signals, because a
  `Send + Sync` callback cannot capture the whole `AppContext`).
- **The operator view mounts lazily** (a latching `Memo` on `ctx.view`): no request until the
  view is first opened, then it stays mounted like the other views.
- The operator header overflows at phone width (pre-existing). Phone E2E for this view checks
  the PANEL (`[data-view-panel="menovky"]`), not `assertNoHorizontalOverflow` on the page.
- Picking a view in the phone menu closes the menu (`set_view` resets `mobile_nav_open`);
  before #871 the overlay stayed open over the picked view.
- Sizes after the save consolidation (production lines, 800 = warning): `mod.rs` ~460,
  `plate_list.rs` ~400, `save_control.rs` ~620, `row_save.rs` ~330, `saving.rs` ~350;
  `save_control/tests.rs` ~780 (exempt as `tests.rs`, but next to 800: new controller
  tests go in a submodule under `save_control/tests/`, like `rebuilt_row.rs`).

## E2E
- `tests/e2e/operator-nameplates.spec.ts`: tab order + URL, add / rename (`fill` + `blur`) /
  reorder / show / hide / delete (confirm) against the REST list, the song row, live changes
  from REST, the no-layer hint, typing that survives refreshes (new plate, another plate
  renamed, the same plate renamed) then saves, mouse drags (none / one plate / above the
  first / below the last / middle, full id set per PUT), two quick ↑ taps, the Companion slot
  labels following a rename and a reorder, the button-13 grid note, and a `PHONE_390` context
  (menu closes, finger drag via `touchDrag`, panel fits, 44 px / 16 px). Tests reset the
  `stream` plates themselves (`resetPlates`), except the first (fresh DB, no-layer hint).
- The #871 review tests: ↑ ↑ / ↓ ↓ on a handle focused ONCE (`page.keyboard.press`; a
  `locator.press` focuses the handle again before every key and hides a lost focus); a
  REST reorder (`putOrder`) while typing keeps the focus, the caret (`setSelectionRange`,
  then `keyboard.type`) and the text, with no PATCH until the blur; leaving the page saves
  once (`document.hasFocus = () => false` + a dispatched `FocusEvent("blur")` that leaves
  the focus on the field; `visibilityState` overridden to "hidden" + a `visibilitychange`
  dispatched with `bubbles: true`, as the browser does, or a window listener never hears
  it — helpers `windowBlur` / `setPageHidden`; the window blur and THEN the page hidden =
  one PATCH, run in TWO evaluates with the blur's PATCH held by `page.route` until the
  page logs its hidden decision: inside one evaluate no task runs between the dispatches,
  so the hidden save would go first); a window blur where `hasFocus()` still says true
  saves (Safari / Firefox timing); a plate deleted by REST while half-typed shows the toast
  and sends no PATCH; two saves of one plate with the first PATCH held by `page.route`
  until the page logs `save waits for the one on its way` (raced against a second PATCH,
  which is let through first via `route.fetch` and fails the test), never a timer; the
  server ends on the later text; a plate deleted by REST mid-drag takes its drop line with
  it.
- The keepalive / failure tests: an `addInitScript` wrapper around `window.fetch` records
  each plate PATCH's `Request.keepalive` (wasm-bindgen calls the page's `fetch`, both the
  gloo binding and `Window::fetch_with_request`); `page.reload()` right after `fill` must
  leave the typed text on the server; a save waiting behind a held PATCH goes out once the
  page hides (every PATCH held in arrival order, released one by one with `route.fetch` +
  `fulfill`); a first PATCH `route.abort()`ed keeps the typed text + the "Text ostal v poli"
  toast, then the next blur (or the page visible again, for a save that failed while
  hidden) saves; a 404 (plate deleted by REST while the page's list GETs are held, so the
  row stays) toasts the deleted-plate text; an emptied name on `windowBlur` /
  `setPageHidden` logs `name is empty; kept as typed` (the settle signal) with no toast and
  no PATCH, while leaving the field still toasts „Meno nesmie byť prázdne.". Specs that
  fail a fetch on purpose split the expected Chrome console line from the rest
  (`splitConsole`) and assert both. A negative check after a settle signal reads the
  attribute or text ONCE (`getAttribute`, `textContent()`), never
  `toHaveAttribute("data-visible", "false")` / `not.toContainText(…)`, which retry until a
  wrong toast is replaced or hidden. The keepalive reload test holds the NEW page's list
  GET (`page.route`) until REST has the saved text: that GET could otherwise reach the
  server before the keepalive PATCH, and the new page's socket may connect after the
  save's change event, so nothing would refresh it.
- The last-round tests (helpers `holdPatches` = every PATCH of a plate held in arrival
  order, `failFirstPatches(n)` = the first n `route.abort()`ed, `failFirstPatchOnCue` = the
  first held until `failFirst()` aborts it): a failed save marks its row (`data-unsaved` +
  „neuložené") and the timer retries it (fails twice, the third try saves, ~7 s, poll timeout
  20 s); the window `online` event retries at once; Zobraziť on a failed row puts the SAVED
  text on air; a save sent beside another as the page hid keeps the next one waiting; an
  older save answered after the newer one makes the page send the newest texts once more; a
  404 drops the waiting save (one 404 line, the decision proven by "a list GET or a second
  PATCH, whichever comes"); an emptied name + a failed role on a hidden page sends the role
  with the newest name. `stream-editor-output-select.spec.ts` adds `switchWhileSaving` (first
  PATCH held across `selectOption`; `finishFirst` returns once the page GETs the new
  output's list again, which comes only after the plate's last save): one PATCH with the
  role kept, and an aborted one toasting `LOST_UNSAVED`. Its own `setPageHidden` /
  `holdFirstPatch` helpers (specs share no module for them).
- The consolidation tests: a commit while a save is on its way goes after it, once, with the
  newest texts, and Zobraziť meanwhile waits (`GET …/nameplates/active` stays null until the
  newest save is back); Zobraziť while the page repairs an older answer that came last waits
  for the repair (the newest role on air); typed, changed and typed back while the save is on
  its way, the save failing: sent once more and nothing after (settle = the page's line
  `saved; nothing left to send` OR a third PATCH, whichever comes); an empty name left while
  the role saves: refused before anything is sent, the role marked and retried; a name
  cleared while its new text saves: the role goes with that newest name on hide; a PATCH
  that never answers: given up after 15 s, marked, sent again (~17 s, poll timeout 25 s);
  Zobraziť dispatched with no blur (`locator.dispatchEvent("click")`, the field keeps the
  focus): the typed text is saved first and goes on air.
- The "a failed save may be applied" round: `holdPatches` also gives `held[n].lose()` =
  `route.fetch()` (the server APPLIES it) then `route.abort()` (the page sees a network
  error, one `ERR_FAILED` line). An older save lost after the newer one was saved leaves the
  server on the OLDER role (asserted), then the page sends the newest texts a third time,
  no failure toast (`data-visible` read once). A late saved answer of a deleted plate's
  save: the second PATCH's route does `route.fetch()` at once and holds only the
  `fulfill`, so it is applied before the REST delete and answered after the first PATCH's
  404 (settle = the page's `answer of a forgotten save` line); once the held list GETs go,
  the row must be gone. `stream-editor-output-select.spec.ts`: a Zobraziť waiting for its
  held save, then an output switch: the race "a PUT to the NEW output's
  `…/nameplates/active`" vs "the idle reload of the new output's list" must end on the
  reload (the old code issued the PUT first), and that output's active stays null; a role
  typed, the output switched while the name's PATCH is held (`holdFirstPatch`), the page
  hidden: a second PATCH with both texts goes before the held one is released, then (held
  one applied last) a third repairs the server. The same with the output switched there
  AND BACK before the tab closes (`closePage` = hidden + `pagehide` dispatched in ONE
  evaluate, as a real close: no answer is handled between them): the plate's rebuilt row
  sends the waiting texts, logged `sent now beside …` exactly once (that log tells it from
  the held save's 15 s timeout retry, which sends the same texts; no `save not done` line
  may appear), then the repair. Page logs there are read from a `logs` array with
  `expect.poll`, so the old code fails on the count, not on a `waitForEvent` timeout. The
  rebuilt row shows the WAITING texts (asserted), not the list's. And the base case: name
  saved (held), output switched there and back, only the role typed in the rebuilt row
  and left (settle `save waits …`), the held save released (settle `saved; nothing left
  to send`): REST must hold BOTH the newer name and the new role, and the second PATCH
  carries both (the old code sent the older name back with the role). A row with nothing
  to commit: name saved (held), role typed, output switched there and back, the name
  CLEARED in the rebuilt row (no role typed there), `closePage`: exactly one `sent now
  beside … (the page went hidden; its row was rebuilt)` carrying the waiting texts, the
  empty name kept; and (looped `close` | `leave`) name saved (held), switched there and
  back, the held save aborted with the spec's own `onUnsavedMark(page, id, "close" |
  "leave")` installed (closes the tab, or `focus()` + `blur()` on the name field, in the
  observer pass): the only `failed save sent again` line carries THAT trigger, never `the
  retry pause ran out` (the old code waited for the 2 s timer, so it fails on the line,
  not a timeout). Controller side: `save_control/tests/rebuilt_row.rs` (a submodule of
  `tests.rs`, which is at ~780 lines) = the CONTROLLER CONTRACT the rebuilt row relies on
  (busy → `desired` = the newest committed texts, idle → `None`; a failed save with
  nothing on its way is skipped by `send_waiting` and sent once by `retry`). Those pass on
  any code that keeps the contract, so they are never a [red]; the row's base CHOICE is
  `row_save::base_text`, unit-tested in `row_save.rs`.
- **Releasing the next held save waits until the page HANDLED the previous answer**
  (`savedWhileAnotherIsOut(page, id)` in both specs = `waitForEvent` on the page's
  `saved; a save of the plate is still on its way`, set up before the release). `held[n]
  .answered` / `waitForResponse` only mean the network delivered it; the page handles it two
  awaits later (`api::finish_json`), so on a CPU-starved runner the next released answer
  could be handled first, down the documented not-repaired path (a timeout). Used by every
  test that releases two held saves of one plate in a row (newer first or in order).
- **A test that acts on a FAILED row acts from a MutationObserver** (`onUnsavedMark(page,
  id, "online" | "hide" | "show")`, installed BEFORE the action that fails the save): the
  observer fires the window `online` event, the page going hidden, or a click on Zobraziť
  in the same microtask pass as the row's `data-unsaved` turning "true", so the page's own
  2 s retry timer can never act first. `unsavedMarkSeen(page, id)` tells whether it fired
  (on code that never marks, a poll on it fails on the assertion, not a timeout).
- Settle signals that fire on the OLD code too (the RED must fail on the assertion, not a
  timeout): a decision log matched by BOTH its outcomes (`save sent` OR `save waits …`;
  `name is empty; kept as typed` + either ending), then the exact line asserted; "the page
  dealt with save 1's answer" = a typed name ending in a space shows trimmed (the save
  trims, the settle drops the draft, the field shows the saved text); for "nothing more is
  sent", a promise resolved by EITHER the new code's line or the old code's extra request
  (never a `waitForEvent` that the old code never satisfies).
- Settle signals: a typed field keeps its draft, so prove the refresh landed through a field
  with no draft (the role) before asserting the typed one. A drag needs a tall viewport
  (1280 × 1200) so every row is on screen. A page log line is a fair settle signal for a
  decision with no DOM trace (`page.waitForEvent("console", { predicate, timeout })`, set
  up before the action).
- `tests/e2e/companion-socket.ts` = the real Companion socket helper (`enableCompanionSocket`,
  `createCompanionSocket`), shared with `stream-companion.spec.ts`.
- `stream-editor-tabs.spec.ts` covers the same list inside the editor (add, Prehrať, Skryť);
  `stream-editor-output-select.spec.ts` the switch clearing the old plates (list GET held)
  and a text whose save failed (first PATCH `route.abort()`ed) being saved with the role it
  had, not toasted as deleted, when the editor switches output.
- RED proof on Tier-0: run the new spec against the CI build of the base commit
  (`gh run download <run> -n build-artifacts`, `target/release/presenter-server` in the
  worktree) inside `sudo unshare -n` (the binary binds ports the deployed dev service
  holds), `PRESENTER_DB_URL=sqlite://<tmp>/red.db?mode=rwc` and a temp copy of the spec
  without `refreshDevData` (the private Bibles are not on dev2). Run each test in its own
  Playwright invocation: after a failure the worker restarts, and the next server cannot
  bind the Companion port the previous one still holds. Delete the binary and the copies.
