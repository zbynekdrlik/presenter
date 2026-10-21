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
  fields; `NameplateStore::update` applies the saved plate locally first, then calls `done`,
  and each draft is dropped only if it is still exactly what was sent (refused = back to
  the saved text). Never commit on `change` as well as `blur`: that saved twice.
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
- Settle signals: a typed field keeps its draft, so prove the refresh landed through a field
  with no draft (the role) before asserting the typed one. A drag needs a tall viewport
  (1280 × 1200) so every row is on screen.
- `tests/e2e/companion-socket.ts` = the real Companion socket helper (`enableCompanionSocket`,
  `createCompanionSocket`), shared with `stream-companion.spec.ts`.
- `stream-editor-tabs.spec.ts` covers the same list inside the editor (add, Prehrať, Skryť);
  `stream-editor-output-select.spec.ts` the switch clearing the old plates (list GET held).
