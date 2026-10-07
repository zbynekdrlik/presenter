---
paths:
  - "crates/presenter-ui/src/pages/settings/**"
  - "crates/presenter-ui/styles/settings.css"
  - "crates/presenter-ui/styles/settings_lists.css"
  - "tests/e2e/settings*.spec.ts"
  - "tests/e2e/operator-settings-native.spec.ts"
---

# Settings tab — form CSS system + inline list editor (#819)

## One form system — never restyle `.settings__form-row` for one card

`settings.css` has exactly ONE `.settings__form-row`: a caption above its input (6 px),
fields in `grid-template-columns: repeat(auto-fit, minmax(180px, 1fr))`, 12 px gaps,
`align-items: end`. Modifiers: `--connection` (Label | Host | narrow 110 px Port),
`--port-middle` (Host | Port | Library), `--inline` / `--single` (flex, auto-width
items: a checkbox, a `--tiny` number input, buttons).

The #819 "huge gaps" bug was a SECOND, global `.settings__form-row { flex-direction:
column }` added for the Video Sources form: every label's `flex: 1 1 220px` became a
220 px HEIGHT (Companion card 720 px, tab 8452 px). A card that needs a different
field layout gets its OWN class (Video Sources: `.settings__ndi-field`), never an
override of the shared one.

List rows, status badges and the inline editor live in `settings_lists.css` (split
so neither file passes 800 lines). Both files must stay listed in
`crates/presenter-ui/index.html`, settings_lists.css right after settings.css. A new
status state needs its `.settings__status--<state>` colour there.

## Resolume / Android lists: `host_editor::ListEditor`, one inline editor per card

- **Edit** renders the editor IN the row; "+ Add …" renders it as the first `<li>`
  (`*-new-item`). `EditTarget` = Closed / New / Item(id). The draft lives at CARD
  level (`ListEditor`), outside the keyed rows, so a poll can never reset typing.
- Field `data-role`s (`host-label`, `android-component`, …) and the message ids
  (`resolume-form-status`, `android-form-status`) stay unique ONLY because one
  editor per card is open. Keep it that way, or the #459 aria-describedby specs
  break.
- Save flow: `begin_save` (no editor / busy / invalid → `None`), `mark_saving`,
  request, reload the list, THEN `finish_save` — it re-enables Save last (a held
  Enter must not POST twice) and closes only if the open-generation counter still
  matches (the operator may have re-opened an editor, even the same row).
- Every list fetch goes through the card's `fetch_hosts` / `fetch_displays`.
  - It is numbered by `list_sync::ResponseOrder`, so a poll sent before a save and
    landing after the save's reload is dropped instead of reverting the row.
  - It calls `forget_missing`, so a row deleted in another tab closes its editor.
  - Never call `hosts.set(list)` directly.
- Focus returns to the row's Edit button / "+ Add" via `focus_on_close`, but only
  when focus fell back to `<body>`. The save's close is async, and the operator may
  already be typing elsewhere. The late-save check is the pure `save_is_current`,
  which is unit-tested.

## Row `<For>` key: id + the fields the row shows or edits — never status, never `updated_at`

Status, latency, warnings and Updated/Created are read through per-row `Memo`s. Do NOT
key on `updated_at` for Resolume: the #564 port-drift writer
(`update_resolume_host_active_port`) bumps it in the background, which rebuilt the
row mid-edit. `resolume.rs` / `android.rs` have `row_key` unit tests for this.

## Checking layout on Tier-0 (no local WASM build)

- For CSS, inject the candidate stylesheet into the LIVE dev page with your own
  isolated chromium (deploy skill), then measure. Delete only the rules whose
  `selectorText` contains `.settings`, recursing into `@media`. Deleting every rule
  that mentions "settings" also removes the operator's
  `[data-view="settings"]` panel rule, and every card then measures 0 px.
- For new markup, use a static HTML mock with the same classes and both
  stylesheets, screenshotted at 1600 px and 600 px.
- The E2E layout guard (`settings-inline-edit.spec.ts`) checks that the Companion
  card is under 300 px, no form-row label is over 90 px, and a list row is under
  160 px.
