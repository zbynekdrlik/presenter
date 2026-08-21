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
NO id registry; per #709's contract `Countdown.timer_id` is forward-looking (the
output page always binds `countdown_to_start`). The countdown form offers a fixed
dropdown mapped 1=countdown_to_start / 2=preach_timer (passes `validate_ref > 0`).

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
