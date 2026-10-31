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

A class in the markup needs a rule, and a rule needs markup: when you delete a CSS
rule, grep `src/pages/settings/` for its class and drop it there too, and never add
a class without CSS. Round 2 removed every CSS-less settings class
(`settings__form-checkbox--inline` / `--block`, `--compact` forms / rows / button,
`settings__card--feature` / `--ableton`, `settings__card-sub`,
`settings__form--ableset`, the bare `settings__form-control`, `settings__host-port`,
`settings__version`) and the rules without markup (`.settings__form-header p`; the
pre-WASM `.settings__form--osc` / `__osc-status` / `__status-line` in operator.css).
No spec selects any of them. The status / badge / source-dot modifiers are built in
code (`format!("settings__status--{}")`, string literals) — grep for the stem
before calling one dead.

List rows, status badges and the inline editor live in `settings_lists.css` (split
so neither file passes 800 lines). Both files must stay listed in
`crates/presenter-ui/index.html`, settings_lists.css right after settings.css. A new
status state needs its `.settings__status--<state>` colour there.

## Row text contrast — at least 4.5:1, measured on the row background

The row background is `#334155`. Secondary text (`.settings__list-aside`,
`.settings__list-meta--muted`) and the row's ghost buttons are `#cbd5e1` at
0.8rem or more (7.0:1). The old `#94a3b8` is only 4.0:1 there. A disabled row is
dimmed with a darker dashed card (`#263346`), never with `opacity`: the old 0.75
faded the secondary text (3.0:1), the warning (3.9:1) and the status badges
(3.6–4.5:1) below 4.5:1, while only the label stayed readable. Before adding a
colour to a row, compute its ratio against `#334155` and `#263346`.

## List cards: `list_card::ListCard` + `host_editor::ListEditor<F>` over an `EditorForm`

The list cards are thin (Resolume, Android; from #852 C1b-6 the listening
microphones). Each implements `CardItem` for its DTO (id, key, label, `address()`,
`values()` for the editor, optional `timestamps()`, status, and the `list` /
`save` / `delete` API calls as `async fn`s), gives a `static CardText<Form>` (ALL
copy — buttons, delete question, tooltips, editor texts — plus the form's config),
and builds the card-specific row parts (`RowParts`: status badge + aside, meta
lines, warning, extra buttons). What the editor edits is the card's `EditorForm`
(#852 C1b-1): `connection_form::ConnectionForm` (label / host / port
/ "Enabled", config `ConnectionSpec`) for the connection cards. A new kind of item
gets its own `EditorForm` (values, `validate`, `render_fields`), never fields
bolted onto `DraftValues`; `host_editor.rs` holds only the shared editor (state machine,
`render_editor`, focus), and form-specific copy lives with its form (the connection cards' lock
tooltips are in `connection_form.rs`).
Everything else is shared: put a new list behaviour or guard in `list_card.rs` /
`host_editor.rs`, never in one card. A failed save / delete line is
`failure_text` (the server's own message for `ApiError::Refused` / `NotFound`).

- **Edit** renders the editor IN the row; "+ Add …" renders it as the first `<li>`
  (`<role>-new-item`). `EditTarget` = Closed / New / Item(id). The draft lives at
  CARD level (`ListEditor`), outside the keyed rows, so a poll can never reset typing.
- What a stage TV opens is never asked for (#879): the server launches the Presenter Stage
  app, `launchComponent` is optional in the API (create = ours, update = the stored one stays).
- Field `data-role`s (`host-label`, `android-label`, …) and the message ids
  (`resolume-form-status`, `android-form-status`) stay unique ONLY because one
  editor per card is open. Keep it that way, or the #459 aria-describedby specs
  break.
- Every list fetch goes through `ListCard::fetch`, the only writer of the private
  `items` signal; cards read it through `items()` (a `ReadSignal`).
  - It is numbered by `list_sync::ResponseOrder`, so a poll sent before a save and
    landing after the save's reload is dropped instead of reverting the row.
  - It calls `forget_missing`. When the open editor's row was deleted elsewhere,
    the editor closes, the toast `CardText.removed_elsewhere` ("This connection
    was removed elsewhere.") says why, and focus goes to "+ Add" (once the save
    settles, if one of that row is in flight).
- Save flow (`ListCard::save`): `begin_save` (no editor / busy / invalid → `None`),
  `mark_saving`, request, on success reload the list, THEN `finish_save`. A failed
  request goes straight to `finish_save`, with no reload; a row deleted elsewhere is
  closed by the next list fetch's `forget_missing` (normally the 5 s poll), save in
  flight or not. `finish_save` re-enables Save last (a held Enter must not POST
  twice) and closes, or on failure shows the error, only if the save's ticket still
  matches the editor: same open generation AND same target (`save_is_current`,
  unit-tested); otherwise the card toasts the error.

## Listening microphones card (#852 C1b-6): whole list behind per-mic save / delete

`settings/listen.rs` is a thin `ListCard<MicConfig>` with Slovak copy in its
`CardText` (incl. `unsaved_title` / `saving_title`); its `EditorForm` lives in
`settings/listen_form.rs` (`MicForm`: name, VBAN stream, optional sender IP,
channel / language / role `<select>`s with `prop:selected` per option, the #827
rule; split out so `listen.rs` can grow the health / switch lines under 800).

- The API is the WHOLE list (`GET|PUT /listen/mics`). `CardItem::save` / `delete`
  read the list, change one mic (`apply_draft`) and `PUT` it back. Two tabs saving
  within that one round trip can lose one edit: accepted for one Settings card and
  ≤ 4 mics — do not add per-mic endpoints or ETags for it. A save of a mic deleted
  meanwhile is refused with `ApiError::NotFound("Tento mikrofón medzitým niekto
  odstránil.")`, never re-added.
- The server validates the whole list; its 422 text shows in the editor line
  (`put_json_refusal` → `ApiError::Refused` → `failure_text`). The card checks only
  name and stream itself.
- A new mic's id is `mic_id_for_role` (`guest` / `interp` / `preacher`, then `-2` …),
  never derived from the name; it never changes on edit (it names the recordings)
  and the row's aside ends with "id <id>". The editor shows it too (`listen-id`,
  `id_line`): an existing mic's own id, a new mic's `mic_id_for_role` over the listed
  ids, recomputed from the role select — so the server's refusal ("Mikrofón „guest-2“:
  …") names an id the operator has seen. The listed ids reach `MicForm::new` through
  the `TakenIds` context `ListenCard` provides (a form cannot see the list): a
  `Signal` DERIVED from the list, never an `RwSignal` mirrored by an `Effect` (that
  lags a tick behind the list). The form is built inside `ListCard::new`, before the
  list exists, so the signal reads a cell `ListenCard` fills right after the card;
  the save still assigns the id from the list it just read.
- `GET /listen/mics` is a quiet GET (`request_log.rs`): the card's 5 s list poll.
- E2E `settings-listen.spec.ts` drives the three selects with real `selectOption`
  (a broken `on_change` must fail it), reads the stored list back through the API
  (a whole-list PUT must leave the other mic unchanged) and measures `.settings-layout`
  at 360 px with the editor open. The two tests that provoke a refused save
  count-assert its `Failed to load resource … 422` line (one each), then
  `toEqual([])` on the rest; the edit test has none and asserts `toEqual([])`.
  Every test resets the mics and the recorder through `request.put` first
  (order-independent, no serial mode), and the server records into a temp folder
  (`PRESENTER_LISTEN_RECORDINGS_DIR` set around `startTestServer`).

Health, socket and recorder (#852 C1b-7):
- One `GET /listen/status` poll every 2 s (`start_status_poll`), numbered by
  `ResponseOrder`. `tick(visible, holds_status)` decides each tick (pure, unit-tested):
  the card is on screen (`offsetParent`) → fetch; hidden → fetch NOTHING and drop the
  status it held (the operator keeps the Settings panel mounted under `display: none`,
  so an hours-old "prijíma" or a checked switch would greet whoever opens Settings;
  the badges read "—" until the first visible tick). Dropping also numbers a request
  nobody sends, so an answer still in flight is discarded. After 2 failed polls in a
  row the status is `None` too (`video_sources::is_stale`, one console warn when it
  goes stale): every badge shows "—" and the lines are empty rather than old numbers.
  The status never enters a row's key; a row reads its own mic through `Memo`s over
  the one signal.
- The recorder switch "Nahrávať dnes" is a checkbox bound to a `Memo<bool>` of
  `recorder.enabled` ONLY (`prop:checked`), so a poll that changes bytes or files
  never re-renders it. It PUTs `/listen/recorder`; once the PUT is answered the box
  shows what the server stored (`enabled_for.is_some()`), a failed PUT restores the
  state from before the click, and the next fetch confirms. It is disabled with no
  status, and when it is off and no mic exists ("Najprv pridaj mikrofón") — an ON
  switch always stays switchable off. With no status it is `indeterminate`
  (`prop:indeterminate` on a `Memo<bool>`) and the line says "Stav nahrávania nie je
  známy." — a disabled, unchecked box must never read as "not recording". The
  retention in its text is core's `RECORDING_RETENTION_DAYS`, not a copy.
- Badges reuse the existing `.settings__status--<state>` modifiers (`live`,
  `not-broadcasting`, `error`, `checking`, `unknown`); add none for the card.
- The E2E plays a real tone with `scripts/dev/listen/vban_send.py` (a
  `child_process` with its own guest stream, `--to 127.0.0.1:<port>`, the port from
  `/listen/status` `socket.localAddr`; stopped with SIGTERM and awaited in a
  `finally`, `--seconds` only bounds a leaked process) into the test server's
  loopback socket, waits until the status shows > 16 000 recorded bytes BEFORE it
  switches off, then reads the WAV header from the temp folder. A stall on a loaded
  runner can split the recording: assert ONE day folder and ≥ 1 file, check every
  header against its file, and compare the status's last `file` / `bytes` with THAT
  file's data size — never `toHaveLength(1)` on the files. Waits use the config's
  expect timeout, no per-assertion `timeout` overrides.
- Two more tests cover the poll. The failing poll serves a MALFORMED 200
  (`body: "not-json"`), never a mocked non-2xx — Chrome logs its own
  `Failed to load resource` line for those (ui skill #598) — and sees the badge,
  lines and switch fall back to unknown / indeterminate, then recover; exactly ONE
  console line may match the stale warn (`listen status poll failed 2x in a row`),
  then `toEqual([])` on the rest. The operator test opens `/ui/operator/settings`,
  switches to Worship (the card is hidden, its badge becomes `unknown`, no
  `/listen/status` request over more than one 2 s tick), then back to Settings with
  the first answer held by `page.route`: the card still claims nothing until it
  arrives.

## The editor's guards: `trigger_lock` (pure, unit-tested)

Every row's Edit and "+ Add" read `ListEditor::trigger_lock(target)` through a
`Memo`. It sets `prop:disabled`, `data-lock` and the tooltip. `open()` checks the
same lock, so a locked trigger can never open the editor:

- **`saving`** — a save is in flight (`busy`). Every trigger is locked, Cancel is
  disabled and Escape is ignored (`ListEditor::cancel`). The late result can then
  never land on a different editor.
- **`unsaved`** — the draft differs from what the editor was opened with (the
  `dirty` Memo: the form's current `Values` != `loaded`, every field). Every OTHER trigger is locked, with the card's
  `EditorSpec.unsaved_title` ("Save or cancel the open editor first" on the
  connection cards). Typing the stored value back unlocks again.
- **`open`** — the trigger's own editor is already open ("+ Add" while adding).
- A clean editor locks nothing: Edit on another row switches in one click.

Focus:
- Label is focused once per open (`take_first_focus`, `takes_focus`). The editor
  remounts when its row is re-keyed, for example by an edit in another tab, and a
  remount must not pull the caret back to Label.
- On close, focus returns to the row's Edit button or to "+ Add" via
  `focus_on_close`, but only when it fell back to `<body>`: the save's close is
  async, and the operator may already be typing elsewhere.
- `focus_on_close` takes the request only while `trigger_lock(target)` is `None`
  (a disabled button ignores `focus()`). The lock is tracked, so the Effect re-runs
  when a save settles: a row deleted elsewhere mid-save gets "+ Add" focused then.
- It defers `el.focus()` with `spawn_local`, so the focus runs after the render
  effects the same change already queued (the button's `disabled`, the editor's
  removal). A newly woken Effect is NOT ordered against them. Keep both the gate
  and the deferral; neither is redundant.

## Row `<For>` key: id + the fields the row shows or edits — never status, never `updated_at`

Status, latency, warnings and Updated/Created are read through per-row `Memo`s
(`ListCard::render_row`). Do NOT key on `updated_at` for Resolume: the #564
port-drift writer (`update_resolume_host_active_port`) bumps it in the background,
which rebuilt the row mid-edit. `resolume.rs` / `android.rs` keep `row_key` (used
by `CardItem::key`) with unit tests for this.

## E2E: race guards are tested with `page.route`

`settings-inline-edit.spec.ts`:
- To test "slow response lands last", hold one GET: call `route.fetch()` at once
  (that captures the pre-save body), wait for a test promise, then
  `route.fulfill({ response })`. A `MutationObserver` on the list records any
  transient old label. Without it, a stale list that the next poll corrects would
  pass.
- Hold the save's PUT the same way (`route.continue()` after the checks) to assert
  the `saving` locks.
- Fulfil or continue with the REAL answer, never a mocked non-2xx: Chrome logs
  `Failed to load resource` for it and the zero-console assertion fails (ui skill
  #598). When a real non-2xx IS the behaviour under test (the mid-save delete spec:
  the held PUT reaches a server without the row and gets a 404), keep it and
  count-assert exactly that one `Failed to load resource … 404\b` line, then
  `toEqual([])` on the rest (ui skill #718).
- Delete through `page.request`, which is not routed and does not log to the console.

## Measuring layout (E2E and live checks)

- `.settings-layout` is the page's scroll container (`height: 100%;
  overflow-y: auto`, so overflow-x computes to `auto`). It absorbs every overflow:
  `document.documentElement.scrollWidth` stays at the viewport width even when the
  page scrolls sideways. Measure `.settings-layout`'s `scrollWidth` vs
  `clientWidth`.
- The bundled Inter font uses `font-display: swap`. Read every rect you compare in
  ONE `page.evaluate` after `await document.fonts.ready`. Two `boundingBox()`
  calls can straddle the swap and mix two layouts (a false red measured: gap 6 px,
  centres 3 px apart).
- Phone widths (the standalone page, `operator-settings-native.spec.ts` checks
  360 and 320 px): a flex row with a text input needs `width: 0; min-width: 0`
  on the input (`min-width: 0` alone is not enough). Its ~20-character intrinsic
  width otherwise counts towards the card's minimum width (the NDI name input made
  the page 404 px on a 360 px phone). The clip-name legend stacks to one column
  at ≤ 840 px (its 160 px name column alone made the page 393 px).
- The header has `gap: 16px`, a minimum title–nav gap at every width. At ≤ 480 px
  only its side padding drops to 20 px (`padding-inline`); the 24 px top / bottom
  padding stays. The header nav is `flex-wrap: wrap` + `white-space: nowrap`.
- Every row text that can carry a long unbroken token needs `overflow-wrap:
  anywhere` (`.settings__list-meta`, `-aside`, `.settings__host-addr`). The Android
  warning names `PRESENTER_ANDROID_STAGE_URL` (27 chars). That alone set the row's
  min-content to 248 px, and the standalone page measured 328 px on a 320 px phone.
  It reproduces only with seeded rows, as in CI, never with an empty DB.
- Video Sources rows (`.settings__source-item`) wrap: `flex-wrap: wrap`, and the
  info column is `flex: 1 1 160px; min-width: 0`. A not-found row on ONE line
  (name, hint, the nowrap badge "Not found on the network", Activate, Delete) is
  about 510 px. Measured on prod v0.4.304: SNV 589/320, PP 427/320. CI seeds no
  not-found row, so the guard for it is in `ndi-source-status.spec.ts` (synthetic
  NDI lane). After a release, also check the standalone page on BOTH prod sites at
  320 px. Real data catches what seeded data cannot.
- To find what sets a minimum width, set `width: min-content` on each card and
  compare. Overflow checks on a forced narrow width miss shrinkable content.

## Checking layout

- For CSS, inject the candidate stylesheet into the LIVE dev page with your own
  isolated chromium (deploy skill), then measure. Delete only the rules whose
  `selectorText` contains `.settings`, recursing into `@media`. Deleting every rule
  that mentions "settings" also removes the operator's
  `[data-view="settings"]` panel rule, and every card then measures 0 px.
- **Run the CI-built binary.** It is faster than a mock and exact for markup.
  `gh run download <run> -n build-artifacts` gives the binary, built with
  `mock-integrations` + `test-helpers`. It hard-binds `127.0.0.1:8091`, which the
  deployed `presenter-dev` holds, so start it in its own network namespace:
  `sudo unshare -n bash -c "ip link set lo up; sudo -u newlevel <script>"`. The
  script starts the binary (`PRESENTER_PORT=18399`, throwaway
  `PRESENTER_DB_URL=sqlite://<tmp>/t.db?mode=rwc`) and then the node Playwright
  probe against `127.0.0.1:18399`. `unshare -rn` (an unprivileged user namespace)
  is blocked on dev2.
- For new markup, use a static HTML mock with the same classes and both
  stylesheets, screenshotted at 1600 px and 600 px.
- The E2E layout guard (`settings-inline-edit.spec.ts`) checks that the Companion
  card is under 300 px, no form-row label is over 90 px, and a list row is under
  160 px.

## Toast timing (#875)

Every toast's timer restarts on each message: the operator `Toast` owns its own, settings and the
stream editor call `toast::hide_after_last_message` once where the signals are made. Never a
`Timeout…forget()` per call. On prod, "Scan" on the video sources card is a read-only way to raise a
settings toast (it only GETs the status).
