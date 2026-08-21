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
