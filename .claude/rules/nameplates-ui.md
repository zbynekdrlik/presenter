---
paths:
  - "crates/presenter-ui/src/components/nameplates/**"
  - "crates/presenter-ui/src/components/stream_editor/editor_nameplates.rs"
  - "crates/presenter-ui/src/components/stream_editor/output_switch.rs"
  - "crates/presenter-ui/styles/nameplates.css"
  - "tests/e2e/operator-nameplates.spec.ts"
  - "tests/e2e/stream-editor-tabs.spec.ts"
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
  left). `save_queue.rs` — pure `SaveQueue`: one PATCH per plate on its way, the newest
  waiting save replaces an older one, `take_waiting` for a page that hides; `SaveOutcome`
  (how a save ended) + its toast texts. Host-tested.
- `api/mod.rs::start_patch_keepalive` + `finish_json` — the plate PATCH as a `keepalive`
  fetch started before the call returns (the `ndi_video::dispatch_delete` pattern); only the
  plate saves use it.
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
- **Half-typed text is never overwritten** (`PlateText` in `plate_list.rs`): each field shows
  its DRAFT (set on `input`) else the saved text, both `Memo`s. tachys re-writes a `String`
  `prop:value` on every run with no comparison, so a plain closure over the whole list would
  wipe typing on ANY list refresh. A row saves on BLUR (Enter blurs), one PATCH carrying both
  fields; `NameplateStore::update` applies the saved plate locally first, then calls
  `done(SaveOutcome)`. Saved or refused (an empty name on leaving the field, a 4xx): each
  draft is dropped only if it is still exactly what was sent (refused = back to the saved
  text). Never commit on `change` as well as `blur`: that saved twice.
- **A failed save keeps the typed text** (`SaveOutcome::of_error`, host-tested). A network
  error (a phone's Wi-Fi asleep after a lock, a cancelled fetch) or a server fault (5xx, 408,
  429, an unreadable 2xx body) is `Failed`: the drafts STAY, the toast says the text stayed
  („Uloženie zlyhalo: … Text ostal v poli — uloží sa znova, keď pole opustíte."), and the
  next save sends it again: leaving the field, Enter, or the page back in view
  (`RowSave.retry`, read by the `visibilitychange` → visible path). A 404 is `Gone`: the
  store toasts `DELETED_UNSAVED` (the deleted-plate text) and the row drops its drafts, so
  the row leaving the list says nothing more. `RowSave::settle` only WRITES signals: the
  `done` of a save can run after its row was disposed (a write to a disposed signal is a
  no-op in reactive_graph 0.1.8, a read panics).
- **A blur saves only when the field was LEFT** (`RowSave::commit_on_leave`). The blur
  handler reads `focus::rows_moving()` AT ONCE; the decision waits in a `spawn_local` task,
  queued after the focus restore below (`focus::blur_kind`). Only a blur DURING a move whose
  field holds the focus again (active element AND `document.hasFocus()`) was a move (the
  browser fires `blur` when `insertBefore` moves the focused node) and saves nothing. Any
  other blur saves: a field still active outside a move lost the WINDOW's focus (alt-tab,
  OBS, Companion, another tab, a locked phone), whatever `hasFocus()` says when the task
  runs (Chrome already says false, Safari / Firefox are not confirmed; the pre-marker
  `hasFocus` heuristic would have skipped that save there); a field no longer active was
  left. `keep_focus` holds the marker from before the list change until its restore task
  has run: the move, its render effect and the restore run in one go of queued tasks, so no
  key, click or window event can blur in between. Never decide synchronously in the blur
  handler: the half-typed text was saved early on every reorder from elsewhere.
- **The row's save logic is one `Copy` struct, `RowSave`** (`plate_list.rs`): `commit(Why)`,
  `settle`, `commit_on_leave`, `page_changed`, `report_lost`. `Why` = `FieldLeft` (Tab, a
  click, Enter) / `FocusAway` (the window lost the focus) / `PageHidden` (`visibilitychange`
  to hidden, or `pagehide`) / `PageBack` (visible again after a failed save). Only
  `FieldLeft` refuses an empty name with „Meno nesmie byť prázdne." (and the field shows the
  saved name again); the other three skip the save and KEEP the draft (the operator cleared
  the name to retype it, then alt-tabbed or locked the phone). Each branch of `commit` logs
  its decision after making it (sent / waits / sent now / on its way / empty kept), never a
  "saved" line ahead of it. Per row, two window listeners removed in `on_cleanup`:
  `visibilitychange` (hidden → save NOW, a phone may kill a hidden tab with no blur; visible
  → retry a failed save) and `pagehide` (a reload or a closed tab; Chrome fires it BEFORE
  `visibilitychange`, Safari may skip `visibilitychange` on unload). `RowSave.sending` holds
  the texts of the row's save on its way, so the same texts are never sent twice (the
  window blur and the page hidden right after = one PATCH). A row whose plate was deleted
  elsewhere with typed, unsent text toasts `DELETED_UNSAVED` from its `on_cleanup`
  (`report_lost`; cleanups run before the row's signals are disposed); the field saves
  nothing then. A row that leaves because the EDITOR switched output (its remembered
  `RowSave.slug` ≠ the store's slug; only kept drafts of a failed save get there, the select
  click blurs a field first) sends that text instead: the plate still exists and the PATCH
  is id-scoped.
- **Saves of one plate go one at a time** (`SaveQueue` in `store.saves`, a
  `StoredValue::new_local`: a waiting save holds its `done` closure). Tab name → role and
  leaving the role are two PATCHes within one round trip; sent at once, a server applying
  them out of order kept the EARLIER text. While one is on its way the newest save waits
  (it carries both fields as they are then) and goes next; a replaced waiting save is
  dropped WITHOUT its `done` (the newer one settles the drafts). The list is reloaded once
  nothing of that plate is left to send. A page that hides yields that order: `commit`
  with `PageHidden` (and `update(.., urgent = true, ..)`) takes the waiting save out
  (`SaveQueue::take_waiting`, `NameplateStore::flush_save`) and sends it NOW beside the one
  on its way: a waiting save on a frozen or closed page never went out. The server applies
  the last PATCH it gets, and that one, sent last, carries the newest texts.
- **Every plate PATCH is a `keepalive` fetch STARTED before `update` returns**
  (`start_save` → `api::start_patch_keepalive`, then `finish_json` in the task). Chrome
  cancels a plain fetch when the page unloads, and the old save started on the first poll
  of a `spawn_local` task: a reload right after typing lost the text (probe: a plain fetch
  queued in a microtask from `pagehide` / `visibilitychange` never reached the server, every
  keepalive one did). `page.route` still intercepts keepalive requests, and a
  `route.abort()` logs ONE `Failed to load resource: net::ERR_FAILED` console line.
  Residual risk, not solvable in the page: iOS Safari may FREEZE or kill a backgrounded tab
  without running `visibilitychange` / `pagehide` handlers to the end (killed from the app
  switcher, memory pressure); Safari before 13 ignores `keepalive`. Text typed then is lost;
  a save that fails because the Wi-Fi slept goes again when the page is back in view.
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
  newest and no reorder PUT is pending. Reorders show at once (`apply_order`) and their PUTs
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
  attribute ONCE (`getAttribute`), never `toHaveAttribute("data-visible", "false")`, which
  retries until the 3.5 s toast timer hides a wrong toast.
- Settle signals: a typed field keeps its draft, so prove the refresh landed through a field
  with no draft (the role) before asserting the typed one. A drag needs a tall viewport
  (1280 × 1200) so every row is on screen. A page log line is a fair settle signal for a
  decision with no DOM trace (`page.waitForEvent("console", { predicate, timeout })`, set
  up before the action).
- `tests/e2e/companion-socket.ts` = the real Companion socket helper (`enableCompanionSocket`,
  `createCompanionSocket`), shared with `stream-companion.spec.ts`.
- `stream-editor-tabs.spec.ts` covers the same list inside the editor (add, Prehrať, Skryť);
  `stream-editor-output-select.spec.ts` the switch clearing the old plates (list GET held)
  and a text whose save failed (first PATCH `route.abort()`ed) being saved, not toasted as
  deleted, when the editor switches output.
- RED proof on Tier-0: run the new spec against the CI build of the base commit
  (`gh run download <run> -n build-artifacts`, `target/release/presenter-server` in the
  worktree) inside `sudo unshare -n` (the binary binds ports the deployed dev service
  holds), `PRESENTER_DB_URL=sqlite://<tmp>/red.db?mode=rwc` and a temp copy of the spec
  without `refreshDevData` (the private Bibles are not on dev2). Run each test in its own
  Playwright invocation: after a failure the worker restarts, and the next server cannot
  bind the Companion port the previous one still holds. Delete the binary and the copies.
