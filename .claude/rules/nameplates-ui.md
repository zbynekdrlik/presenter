---
paths:
  - "crates/presenter-ui/src/components/nameplates/**"
  - "crates/presenter-ui/src/components/stream_editor/editor_nameplates.rs"
  - "crates/presenter-ui/styles/nameplates.css"
  - "tests/e2e/operator-nameplates.spec.ts"
  - "tests/e2e/stream-editor-tabs.spec.ts"
---

# Nameplate ("Menovky") plate list: one component, two places (#871)

## Where things live
- `components/nameplates/mod.rs` — `NameplateStore` (`Copy`): one output's plate list, the
  plate on air, the live song, and EVERY plate REST call (list / add / PATCH / DELETE /
  reorder / show / hide) plus the request DTOs and paths. Never a second copy elsewhere:
  the editor's `output_paths.rs` no longer has plate paths.
- `components/nameplates/plate_list.rs` — the shared views: `NameplateSongRow`,
  `NameplateRows` (keyed `<For>` by plate id), `NameplateAddForm`. `RowOptions` carries the
  per-page extras as `fn` pointers (`preview`, `after_hide`) + `numbered`, so it stays `Copy`.
- `components/nameplates/numbering.rs` — pure Stream Deck numbering (#870: button 1 = song,
  person at list index i = button i + 2, grid 4 × 3 = 12 buttons). Host-tested.
- `components/nameplates/operator_view.rs` — the operator's Menovky view
  (`/ui/operator/menovky`, tab after AI in `header.rs`), always the `stream` output.
- `stream_editor/editor_nameplates.rs` — the editor's Menovky tab = the shared list + its
  extras: Prehrať (`preview_nameplate`) in the tab's own preview frame, Skryť also clears that
  frame, "Vytvoriť menovkovú vrstvu".
- CSS: the list is `nameplates__*` in `styles/nameplates.css` (linked after
  `stream_editor.css` in `index.html`); the operator view's touch sizes (44 px controls,
  16 px inputs) are scoped to `.nameplates-view`, so the editor keeps its dense look.

## Rules
- **The DOM contract is the editor's (#779):** `stream-nameplate-song`, `stream-nameplate`
  rows (`data-nameplate-id`, `data-active`), `stream-nameplate-name` / `-role` inputs,
  `-show` / `-preview` / `-hide` / `-up` / `-down` / `-delete`, the `stream-nameplate-add`
  form. Operator-only: `stream-nameplate-button` (number) and `data-beyond-grid` (only when
  `numbered`), scoped under `[data-role="operator-nameplates"]`.
- **A row's number follows its POSITION, not its key.** The keyed `<For>` reuses a row across
  a reorder or a delete elsewhere, so the number is a `Memo` over the list by id (#496 rule),
  never captured in `children`.
- **The store owns the slug.** It holds the editor's `output_slug` signal itself, so an output
  switch reloads the right list (`output_switch.rs` calls `plates.reload_list()` /
  `reload_active()`), and a response for a slug that is no longer current is dropped.
- **Live updates go through `NameplateStore::apply_event`** (`StreamNameplate`,
  `StreamNameplatesChanged`, `Stage`). Both pages forward their socket's events to it; the
  operator view also refetches the `stream` def on a newer `StreamConfigChanged` (the
  no-layer hint) and reloads everything after a socket reconnect.
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
- `tests/e2e/operator-nameplates.spec.ts`: tab order + URL, add / rename (`fill` + `blur`,
  the inputs commit on `change`) / reorder / show / hide / delete (confirm) against the REST
  list, the song row, live changes from REST, the no-layer hint following the def, the
  button-13 grid note, and a `PHONE_390` context (menu closes, panel fits, 44 px / 16 px).
- `stream-editor-tabs.spec.ts` covers the same list inside the editor (add, Prehrať, Skryť).
