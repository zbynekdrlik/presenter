---
paths:
  - "crates/presenter-ui/src/pages/stream_editor.rs"
  - "crates/presenter-ui/src/components/stream_editor/**"
  - "tests/e2e/stream-editor*.spec.ts"
---

# Stream-graphics WASM operator editor (`/ui/stream`, #713, epic #718)

The editor is a standalone Leptos page (NOT an operator.rs tab). Built layers so
far: #713 v1 skeleton (scene columns + overlay row + activation + add/remove/
rename/reorder). Element CRUD + property panel = #714; preview iframe + assets =
#715. The companion OUTPUT page `/stream/{slug}` is a SEPARATE lane
(`components/stream/**`, `pages/stream_output.rs`, `ws/stream.rs`) — do NOT edit
those from an editor ticket.

## def + events reconciliation (the client model)
Two signals: `def: RwSignal<Option<StreamOutputDef>>` (full config) and
`active: RwSignal<StreamShowState>` (live show-state). Rules:
- `LiveEvent::StreamState{output=="stream", ..}` → apply DIRECTLY to `active`
  (activation does NOT bump `config_revision`, so no def refetch).
- `LiveEvent::StreamConfigChanged{output=="stream", config_revision}` → refetch
  the def ONLY when `config_revision` advances past the local one.
- Own ACTIVATION writes (`active-scene` / `overlays` / `clear`) return
  `StreamShowState` → apply directly. Own CONFIG writes (create/rename/delete/
  reorder) → refetch the def (they bump `config_revision`).
This is why the client refetch-on-config vs apply-activation split matters —
mirrors `.claude/rules/stream-graphics.md`'s server-side note. Do not refetch the
def on every change; it defeats the lightweight `StreamState` event.

## Reuse the GENERIC ws + api layers — no bespoke client
- WS: `crate::ws::use_live_websocket("stream")` gives
  `(_state, last_event: ReadSignal<Option<LiveEvent>>)`. Do NOT hand-roll a
  stream ws hook (`ws/stream.rs` belongs to the output-page lane).
- REST: `crate::api::{get_json, put_json, post_json, patch_json, put_no_content,
  delete}`. No `api/stream.rs` module — the server write DTOs are `pub(super)` in
  `router/stream.rs`, so define small client-side request structs locally
  (`#[serde(rename_all = "camelCase")]`): active-scene body `{sceneId: Option<i64>}`
  (null clears base), overlay `{active: bool}`, create `{name, kind: SceneKind}`
  (kind serialises snake_case), reorder `{ids: Vec<i64>}`, rename `{name}`.

## Reorder wants the FULL id set (base ++ overlay), 422 on a partial set
`PUT /stream/api/outputs/{slug}/scenes/order` (`set_scene_order`) requires the
EXACT set of ALL the output's scene ids (no dupes, no missing) or it returns
`Invalid` (422). It reassigns positions PER KIND by list order. So a
one-step up/down move = read the def, split into base ids + overlay ids in
current order, swap the one pair within its kind, send `base ++ overlay`.

## Read signals UNTRACKED inside event handlers
`on:click`/`on:submit` handlers read signals with `.get_untracked()` (matches
`pages/settings/android.rs`), never a tracked `.get()` — a handler is not a
reactive scope and an accidental subscription there is a latent bug. The
reactive reads (class/`data-active`/text) stay tracked in the view.

## Active highlight via `data-active`, not a `class:` toggle
Set `data-active=move || if is_active() {"true"} else {"false"}` and style
`.stream-editor__scene[data-active="true"]` in CSS. One attribute serves both the
CSS highlight and the E2E assertion; avoids the `class:name--active` double-hyphen
directive question entirely.

## CI does NOT clippy `presenter-ui` — but it MUST fmt + compile
`presenter-ui` is a workspace `exclude`, so CI's `cargo clippy --workspace` never
lints it. It IS gated by: `cargo fmt --check` (run separately in
`crates/presenter-ui`), the wasm `trunk build`, and host `cargo test --lib`. So a
clippy-only nit won't red CI here, but a compile error or fmt drift will. Local
(Tier-0): only `cargo fmt --manifest-path crates/presenter-ui/Cargo.toml -- --check`
plus the size gates — never a local build.

## `#[component]` fns are EXEMPT from the function-length gate
`scripts/dev/fn_length_check.py` skips Leptos `#[component]` fns (view! DSL), so
a large `view!` block is fine. Non-component helpers/methods still cap at 120 —
keep the ctx action methods small.

## Property panel (#714): ONE draft `RwSignal<StreamElementProps>` + field accessors
The element property form edits a SINGLE working copy (`draft`), not a wall of
per-field signals. `components/stream_editor/props_access.rs` holds the accessors:
`read_frame`/`with_frame_mut` (one `|`-pattern across all 4 kinds), the six
`TextStyle` slots via a `TsSlot` selector (`read_ts`/`with_ts_mut`) so the shared
`TextStyleForm` component works for every kind, `read_transition`/
`with_transition_mut`, `split_color`/`join_color` (`#rrggbb` + alpha byte ⇄
`#rrggbb[aa]`), and `default_element_props(kind)`. The seed Effect tracks
`selected_element` and reads `def` UNTRACKED, so a live `StreamConfigChanged`
refetch never clobbers unsaved edits; Save is EXPLICIT (PATCH the raw props enum).
Element create/patch bodies are the RAW `StreamElementProps` JSON (serde
snake_case, `kind` tag) — POST/PATCH `&props` directly, no camelCase wrapper DTO.

## Surface a 422/409 body inline with `crate::api::*_detail`
The plain api helpers only carry the HTTP status TEXT. To render a server refusal
message inline (props-validation 422; referenced-asset 409 naming the scenes), use
`post_json_detail` / `patch_json_detail` / `delete_detail` — they read the
`ErrorBody { message }` from the response. `ctx.prop_error` (set by `save_props`)
holds the panel's inline error; the asset picker has its own `asset_error`.

## Element reorder needs the FULL element-id set (like scene reorder)
`PUT /stream/api/scenes/{scene_id}/elements/order {ids}` requires the EXACT set of
the scene's element ids (422 on a partial/dup set); it reassigns z_order by list
order. `update_stream_element` PRESERVES z_order (props-only), so this endpoint is
the ONLY way to change element order. Repo `validate_order_set` is shared by scene
+ element reorder.

## Countdown `timer_id`: fixed 2-timer dropdown, conventional ids 1/2
`TimersOverview` has exactly two timers (`countdown_to_start`, `preach_timer`) and
NO id registry. The countdown form offers a fixed dropdown mapped
1=countdown_to_start / 2=preach_timer (passes `validate_ref > 0`). **Since #785 the
OUTPUT renderer (`element_countdown`) HONOURS the id** (1 → count-down, 2 →
count-up via `format_elapsed`); it is no longer forward-looking / always-
countdown_to_start. Keep the dropdown ids at 1/2.

## Output switcher — design ANY output, `ctx.output_slug` not `DEFAULT_OUTPUT_SLUG` (#785)
The editor is no longer hard-wired to the single `stream` output. `StreamEditorCtx`
gains `output_slug: RwSignal<String>` + `outputs: RwSignal<Vec<StreamOutputSummary>>`,
and a new sibling module `components/stream_editor/output_paths.rs` holds:
- the SLUG-AWARE output-scoped path builders (`def_path(slug)`, `output_path`,
  `scenes_path`, `scenes_order_path`, `active_scene_path`, `overlay_path`,
  `nameplates{,_order,_active}_path`) — moved OUT of `mod.rs` purely to keep it
  under the file-size gate. The ID-scoped builders (`/stream/api/scenes/{id}`,
  `/elements/{id}`, `/nameplates/{id}`, `scenes/{id}/elements`) stay in their
  module — they are not output-scoped.
- `OutputSelect` (the header `<select data-role="stream-output-select">`, options
  from `GET /stream/api/outputs`),
- selected-output persistence: `localStorage` (`stream-editor-output`) + the
  `?output=` URL param (`initial_output_slug()` reads URL → storage → default on
  load; `switch_output` writes both via `persist_output_slug` +
  `mirror_output_to_url`).

Rules that MUST hold when touching the editor:
- EVERY output-scoped call reads `ctx.output_slug.get_untracked()` (captured at the
  top of the method, before `spawn_local`) — grep for a bare `DEFAULT_OUTPUT_SLUG`
  before pushing; it should survive ONLY as the default value + `initial_output_slug`'s
  fallback.
- The page's WS-event filters compare `output == ctx.output_slug.get_untracked()`
  (not the constant), so a switched editor reflects the RIGHT output's live events.
- `switch_output` asks about unsaved element edits first (#787 reopen; it returns `false`
  when declined), then resets the per-output selection/draft (`close_panel`) and
  refetches def + nameplates + active-nameplate for the new slug. Fonts are global
  (not per-output) — do NOT refetch them on switch.
- `editor_preview.rs` builds the iframe `src` from `ctx.output_slug`, so the
  preview follows the switch.

## Preview iframe + assets (#715) are RUNTIME-coupled to parallel lanes
`editor_preview.rs` embeds `<iframe src="/stream/{slug}?preview=1&scene=<id>">`
(reactive `src`; a "live" toggle drops the `scene` param). `editor_assets.rs`
uploads via `web_sys::FormData` + `crate::api::post_form_data` to `POST
/stream/assets` (field `file`), lists `GET /stream/api/assets`, thumbnails
`/stream/assets/{id}`. The output page (`/stream/{slug}`, #709) and asset routes
(#708) are SEPARATE lanes — E2E that drives them (`stream-preview.spec.ts` beyond
the src-string checks) only passes in INTEGRATED CI, not an isolated editor
worktree. Capture iframe console too (`page.on("console")` covers child frames);
never mock a non-2xx in a zero-console spec (#598 — use the real 409 / a
malformed-200).

## Inline `move ||` is fine for attributes / `<Show when>` — only `<For each>` needs a `let`
See the ui skill's view!-macro note (corrected in #714): do NOT hoist every
`data-x=move ||` / `prop:value=move ||` / `<Show when=move ||>`; that churn is
unnecessary. Only `<For each=…>` needs a named closure.

## Live preview = a draft-override `postMessage` channel, ONE shared module (#777)
The editor previews UNSAVED edits by pushing them into the REAL output iframe, so
the preview IS the real renderer (fonts/transitions/timers) — never a second
in-page render. The wire shape lives in ONE place, `components/stream/draft_preview.rs`
(`DraftMessage {type:"presenter-stream-draft", elementId, props}`, serde of the
existing `StreamElementProps`), shared by editor (serialize + `iframe.content_window()
.post_message`) and output page (listen). Rules that MUST hold:
- The output page installs the listener + provides the `StreamDraftOverride` context
  ONLY when `?preview=1`, and verifies `event.origin == window.location.origin`.
  A production output installs nothing → `scene_render`'s `use_context` is `None` →
  each element's `Memo` returns stored props ONCE and never re-fires (zero cost).
- `scene_render` resolves each element through a per-element `Memo` so ONLY the edited
  element re-renders on a draft change; needs `StreamElementProps: PartialEq` (it is).
- The editor pushes on EVERY draft change (Effect tracking draft + draft_element_id)
  AND on iframe `on:load` (fresh listener). In the "live" toggle push a CLEAR (no
  override) so an unsaved draft never leaks onto the live view (#777 review fix).
- The draft is the SHARED `ctx.draft: RwSignal<StreamElementProps>` (+ `draft_element_id`),
  kept a bare props signal (NOT `Option`) so `TextStyleForm`/`ImageFields`/… prop types
  are unchanged — the form + the canvas overlay + the preview push all read/write it.

## Forced-scene preview renders only a BASE scene (`scene=` param)
`pages/stream_output.rs` applies the preview `scene=<id>` as the BASE (`forced_scene`);
an OVERLAY scene forced via `scene=` matches no base → the iframe renders nothing for
it. The interaction overlay still works on overlay scenes (it draws from def+draft,
independent of the iframe), so canvas editing of an overlay scene is fine — only its
LIVE iframe render is empty. (Enhancement candidate: force an overlay scene into
`overlay_ids` in preview mode.)

## Canvas overlay: pointer capture on the container, `page.mouse` emits real pointer events
`components/stream_editor/canvas_overlay.rs` — set `set_pointer_capture` on the overlay
container in pointerdown; handle `pointermove`/`pointerup` on the CONTAINER (events bubble
from the captured child) + `keydown` for arrow-nudge; container needs `tabindex=0` +
CSS `touch-action:none`. The iframe below is `pointer-events:none` while editing. In the
E2E, Playwright's `page.mouse` (down/move-in-steps/up) DOES dispatch real pointer events
in Chromium, so a real-pointer drag test works. All geometry is the pure host-tested
`frame_math.rs` (move/resize per 8 handles, min-size, snap, clamp to core + in-canvas).

## Buffered frame numeric field MUST commit on `input`, not only `change`/`blur` (#777)
`components/stream_editor/number_field.rs` — a `<input type=number>` bound to a parsed
signal fights the caret on intermediate text (`"-"`, `"1."`). The fix keeps a local
String buffer for the DISPLAY (while focused) but commits the parsed+CLAMPED value on
EVERY `on:input` — because (a) Playwright `fill()` fires `input` but NOT `change`/`blur`,
so a change-only commit is invisible to `fill`-based tests, and (b) live two-way sync
(field ↔ canvas) needs per-keystroke commit. Clamping on commit is what makes a save
impossible to 422 on the frame — so a deliberate-422 E2E must use a NON-frame field
(e.g. text `size_pct` beyond 0..=100), NOT a frame field.

## Canvas gestures = the pure `gesture.rs` state machine; a drag must never outlive the button (#787)
`components/stream_editor/gesture.rs` (host-tested) decides everything; `canvas_overlay.rs`
only feeds it events. Invariants that MUST hold when touching the overlay:
- pointerdown → `Pending` (nothing moves); a drag starts only after `DRAG_THRESHOLD_PX`
  (4 px) of travel, then applies the FULL delta since pointerdown. A click never edits the frame.
- The gesture ends on `pointerup`, `pointercancel`, `lostpointercapture`, window `blur`, AND
  any `pointermove` with `ev.buttons() == 0`. The last one matters most: the dirty-guard
  `confirm()` inside pointerdown (and alt-tab / an OS gesture) can swallow the pointerup, so
  the overlay never hears the release. Without the buttons check the element then follows
  every plain mouse move ("stuck drag").
- Escape during a gesture restores the frame from pointerdown; Escape when idle, or a
  pointerdown whose target IS the overlay container (`data-role` check — do not use
  `currentTarget` under leptos event delegation), calls `ctx.deselect_element()`, which asks
  the same „Zahodiť neuložené zmeny prvku?" question as `select_element`.
- The window blur listener is `window_event_listener_untyped` + `on_cleanup(handle.remove())`.
  Do NOT `forget()` it: `WindowListenerHandle` is `Send` (the host build accepts it), and the
  overlay remounts on every scene open, so a leaked closure would read disposed `StoredValue`s.
- Outlines render in ascending `z_order` (`ids_bottom_to_top`), so the top-most element is
  last in the DOM and the browser's own hit test picks it. No `elementsFromPoint` is needed.
  A scene with a full-canvas element (e.g. the SNV timer's 0/0/100/100 image) has NO empty
  canvas, so in that scene only Escape deselects.
- Only the primary button (`ev.button() == 0`) selects, starts a gesture, or deselects.
  A right-click must not drag anything.
- Activation responses (`activate_base` / `toggle_overlay`) and `reload_def` capture the slug
  before `spawn_local` and discard a response once the output has changed. `reload_def`
  returns `bool`, so a follow-up such as `add_element`'s selection runs only after the def
  was actually installed.
- E2E: simulate a lost pointerup with `overlay.releasePointerCapture(pid)` (read the pid from
  a capturing `pointerdown` listener) and then `mouse.up()` outside the overlay. Simulate
  alt-tab with `window.dispatchEvent(new Event("blur"))`.

## Output switch clears `def` first; `reload_def` drops a stale slug's response (#787)
`switch_output` sets `def = None` (and resets `active`) BEFORE the refetch, so `EditorScenes`
shows its „Načítavam…" fallback and every def-reading action (`move_scene`, …) no-ops until
the new def lands. `reload_def` re-reads `output_slug` after the await and discards a
response for a slug that is no longer selected. E2E: hold the page's def response with a
`page.route` gate. `page.request` is NOT intercepted by `page.route`, so read expected ids
through it.

## A late async response must never overwrite a newer local edit — `SelectionIntent` (#787 reopen)
This was the root cause of the first-attempt reds in `stream-editor-canvas.spec.ts:277/:498`
(CI run 36428243568: the saved frame was `[10,60]` instead of `[60,60]`). `add_element` kept the
PREVIOUS element selected, with its form editable, for the whole POST + `reload_def` round-trip.
It then set `selected_element` to the new id without any check, so `element_form.rs`'s seed Effect
re-seeded `draft` over the first field typed in between. It was NOT a def-refetch clobber and NOT
a `switch_output` clobber. Rules:
- An action that SELECTS something when its response lands must release the current selection
  BEFORE it awaits. It does this through `confirm_discard_draft`, the one shared dirty guard;
  declining aborts the action. That way no stale form is editable while the request is in flight.
- It first captures the ticket from `bump_selection()`, and applies its selection only if
  `selection_is_current(ticket)` still holds.
- Every selection change bumps the counter: `select_scene`, `close_panel`, `select_element`,
  `deselect_element`, `delete_element` of the selected element, and `add_element`. The counter is
  the pure, host-tested `selection_intent::SelectionIntent`, held in `ctx.selection`
  (`StoredValue`). Any NEW selection-changing path MUST bump it too.
- E2E settle signal after adding an element: wait for `overlayEl(newId)` to have
  `data-selected="true"`. It reads `draft_element_id`, so it means the element is selected AND
  seeded. Never wait only for "the server def has it": `page.request` polls see the server, not
  the page.
- To reproduce the window, gate the page's `POST /stream/api/scenes/{id}/elements` with
  `page.route`.
- The local def must match the operator's own writes (`def_sync.rs`, pure + host-tested).
  - `save_props` applies the element returned by the PATCH to `def` at once
    (`apply_saved_element`), before its own refresh. Without that, the page keeps the OLD def
    until the refresh lands, so the draft reads as "dirty". Any guarded action in that gap
    (add / select) then asks about edits that are already saved, and Playwright auto-dismisses
    the question, which silently aborts the action.
  - `apply_saved_element` also raises the local `config_revision` by 1. The PATCH moved the
    server to at least N+1, so a GET that started before the PATCH (still at N) can no longer
    roll the save back. The save's own refetch (N+1) still installs, and the WS
    `StreamConfigChanged(N+1)` no longer triggers a second refetch.
  - `reload_def` drops a def for the same output whose `config_revision` is older than the one
    installed (`should_install`). A different slug or output `id` always installs, which covers
    an output that was recreated and whose revision reset to 0. `reload_def` still returns `true`
    when it drops, because a def at least that new is already installed.
- E2E settle signal for "the add decided its selection": the „Prvok pridaný." toast. The row
  count is not enough, because a live-event refetch can install the def before `add_element`
  decides.
- E2E settle signal after a SAVE: wait for the Save button to have `data-dirty="false"`
  (`saveFrame`). `waitForSavedFrame` alone polls the SERVER, not the page.
- While a create is in flight nothing is selected, so Escape or an empty-canvas click is a no-op
  and does not cancel the late auto-select. That is harmless, because nothing was visibly
  selected. An explicit pick (`select_element`) DOES cancel it.

## EVERY path that drops the element draft asks first; save errors are tagged (#787 reopen, gaps 2+3)
- The selection-changing ctx methods live in `selection_intent.rs`, next to the one guard
  `confirm_discard_draft`. Guarded: `select_scene` (scene „Upraviť"), `request_close_panel`
  (the panel's „Zavrieť"), `select_element`, `deselect_element`, `add_element`, and
  `switch_output`. Declining keeps the element and its edits. A NEW UI path that drops the draft
  MUST go through the guard, and must also bump the selection ticket. Re-clicking „Upraviť" on
  the scene that is already open also asks, because it resets the element selection. These
  guarded methods decide synchronously inside the click handler, and the E2E "declined, nothing
  changed" checks rely on that.
- `close_panel()` is the RAW reset, with no question. Call it only after the caller has already
  decided: `switch_output` after its guard, and `delete_scene` after the delete was confirmed.
  Never wire it to a button.
- A declined output switch leaves `output_slug` unchanged, so no option's `prop:selected` re-fires
  and the `<select>` would keep showing the rejected option. `on_output_change` (in
  `output_paths.rs`) sets it back to `ctx.slug()` by hand.
- `ctx.prop_error` is `RwSignal<Option<PropError>>`. `PropError { element_id, message }` is in
  `prop_error.rs` (pure + host-tested). `save_props` tags a failed save with the element it saved
  (`set_prop_error`), and the form shows it only when `message_for(err, selected_element)`
  matches. A successful save clears only its own element's error (`clear_prop_error_of`), so a
  late success of A never wipes B's error.
- `set_prop_error` / `clear_prop_error_of` live in `prop_error.rs` (an `impl StreamEditorCtx`
  block next to the pure helpers). `set_prop_error` logs
  `stream editor: save of element {id} failed: …` AFTER storing the error. That is a console.log,
  which `attachEditorConsoleCollector` ignores.
- E2E for "something is ABSENT after a late response": the settle signal must be one that fires on
  the buggy code too. Otherwise the RED fails on a timeout instead of on the real assertion. The
  late-error test polls "the log line was seen OR an error box appeared". It then forces a render
  flush with a positive reactive wait (fill a field, then expect `data-dirty="true"`), and only
  after that asserts that the error box has a count of 0.
- E2E for a real 422 on a non-frame field: a text `size_pct` of 5000 (countdown
  `[data-role="stream-ts-countdown"] [data-role="stream-ts-size"]`). Hold the PATCH with
  `page.route`. Chrome logs the non-2xx itself, so strip exactly one 422 console line.
- Dialog handling in E2E: an un-handled `confirm()` is auto-DISMISSED by Playwright, which
  silently aborts a guarded action. Arm `answerNextDialog(page, accept)` before every click that
  can meet a dirty draft, and assert the question text.

## A `<select>` fed by an async list: `prop:selected` on each OPTION, never `prop:value` (#827)
`prop:value` on the `<select>` runs once at construction. The options come later
(`GET /stream/api/outputs` lands after the def), so that value matched nothing. When the options
rendered, the browser picked the FIRST one, while the editor edited another output (PP:
"Moderátor" shown, "stream" scenes edited). `OutputSelect` now sets
`prop:selected = (slug == ctx.output_slug)` on every option. Each option is created already in the
right state and follows every later switch, whatever order the list and the def arrive in. A slug
missing from the list (a deleted output, a stale bookmark) switches to the first listed output
through `switch_output`. `output_paths::fallback_output` (pure, host-tested) decides that.
E2E: gate the page's `**/stream/api/outputs` with `page.route` so the def lands first. The glob
matches only that exact path, not `/outputs/{slug}/…`.

## Scény | Menovky | Písma tabs (#829): panels stay MOUNTED, URL params are merged
- `editor_tabs.rs` owns `EditorTab` (ids `scenes|nameplates|fonts`), `initial_tab()` (`?tab=` →
  localStorage `stream-editor-tab` → Scény), `ctx.select_tab()`, `EditorTabs` and `TabPanel`.
  `TabPanel` only toggles `data-active`, and CSS `display:none` hides the inactive panel. Never
  unmount a panel: the element draft and a half-typed nameplate must survive a switch, and a
  switch must never touch the dirty-draft guard or the selection ticket.
- Any editor URL param goes through `crate::utils::window::replace_url_param(name, value)`. It
  sets one param and keeps the others plus the hash (`query_with_param` is pure and host-tested).
  Before #829, `mirror_output_to_url` replaced the WHOLE query, which would drop `?tab=`.
- Raw localStorage for the editor: `output_paths::{read_stored, write_stored}` (no `presenter:`
  prefix, no JSON). Don't add a third copy of the `local_storage()` helper.
- E2E: the scene body `[data-role="stream-editor"]` is VISIBLE only on the Scény tab. A spec that
  reloads into another tab must wait for `[data-role="stream-editor-tabs"]` instead. Every test
  gets a fresh context, so localStorage never leaks a tab between tests. The font upload panel
  sits behind `[data-role="stream-editor-tab"][data-tab="fonts"]` (`stream-fonts.spec.ts`).
- Known gap: the Menovky „Prehrať" buttons post into the preview iframe, which lives in the
  (hidden) Scény tab. The animation plays where the operator cannot see it.
- Phone width: the header and the tab bar `flex-wrap`, the font `<input type=file>` gets
  `max-width:100%` (its ~300 px intrinsic width overflowed 320 px), and the ≤480 px gutters sit in
  ONE `@media` block at the END of `stream_editor.css`. It must stay last: same-specificity base
  rules after it would win.

## Content-transition control = three radios (#834)
`TransitionFields` offers Strih / Prelínať (crossfade) / Prelínať cez prázdno. The radios are
`stream-transition-cut`, `stream-transition-fade` and `stream-transition-fade-through`, and they
share one `name`, which is safe because only one property form is mounted. The crossfade radio
kept the old checkbox's `data-role`, so `.check()` in older specs still works; `.uncheck()` on a
radio would throw. `transition_choice.rs` (pure) carries the duration between the two fades. A
cut starts at `STREAM_DEFAULT_FADE_MS`.
