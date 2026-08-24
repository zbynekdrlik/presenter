---
paths:
  - "crates/presenter-core/src/stream.rs"
  - "crates/presenter-persistence/src/repository/stream*.rs"
  - "crates/presenter-server/src/**/stream*.rs"
  - "crates/presenter-ui/src/**/stream*.rs"
  - "crates/presenter-ui/src/components/stream/**"
  - "crates/presenter-migration/src/*stream*.rs"
---

# Stream-graphics subsystem (Resolume replacement, epic #718)

A nameable transparent WASM page (`/stream/{slug}`, an OBS browser source) renders one exclusive
BASE scene + a set of OVERLAY scenes, composed of typed elements (image/countdown/lyrics/verse).
The BINDING design is epic #718 comment "Stream graphics architecture" + ADR `0009-stream-graphics.md`.
Layers shipped so far: #703 schema+entities, #704 core types, #705 repository. Router/state (#706+),
Companion, and the WASM pages come later — the invariants below cross those PRs.

## serde tag is `"kind"`, not `"type"`
`StreamElementProps` is `#[serde(tag = "kind", rename_all = "snake_case")]` — the tag VALUE must
equal the `stream_elements.kind` column (`image|countdown|lyrics|verse`). Some early scoping text
said `"type"`; the schema column, the ADR, and the code all say `"kind"`. `ContentTransition`
defaults to `Fade{300}` (an omitted `content_transition` field deserialises to it).

## Entity ids are `i32`; DTOs + repository params are `i64`
Every `stream_*` entity `id` is `i32` (SQLite AUTOINCREMENT). The shared `presenter-core` DTOs and
the repository method signatures use `i64` (WASM/API friendly). When a client-supplied `i64` id
crosses into an entity lookup, use `i32::try_from(id).map_err(|_| RepositoryError::NotFound(...))?`
— NEVER `id as i32`, which wrap-truncates an out-of-range id into a DIFFERENT existing row (a
wrong-resource / IDOR-shaped bug). This bites the router PR the moment a route forwards a path id.

## `config_revision` bumps on CONFIG writes, NOT activation
`stream_outputs.config_revision` is bumped (via the single `bump_config_revision` helper) on every
CONFIG write: output/scene/element create/rename/patch/delete + scene reorder. It is DELIBERATELY
NOT bumped on activation (`set_active_scene` / `set_overlay_active` / `clear_stream_output`) —
activation is show-state, broadcast by the separate `StreamState` LiveEvent (arch §6/§8). Bumping
it on every scene switch would force a full def refetch and defeat the lightweight event. The client
refetches the def only when `config_revision` advances (`StreamConfigChanged`).

## Repository error-status taxonomy (on top of `repository-error-pattern.md`)
- duplicate output slug / scene name (case-insensitive, compared in Rust with `to_lowercase` — SQLite
  `lower()` is ASCII-only, names are Slovak) → `Conflict` (409).
- asset delete while referenced by an `image` element's props → `ConflictDetail(String)` (409),
  message naming the referencing scenes.
- wrong-kind activation (base-as-overlay / overlay-as-base), scene-not-in-output, props tag≠kind,
  core `validate_props` failure → `Invalid(String)` (422).
- a MISSING body-referenced scene (activation) → `TargetNotFound` (422, the documented "missing body
  target" variant), distinct from a missing URL resource → `NotFound` (404).
`RepositoryError::ConflictDetail(String)`→409 and `Invalid(String)`→422 were added in #705 and are
already wired in `router.rs`'s central `From<anyhow::Error> for AppError` — reuse them, don't add a
parallel taxonomy.

## `load_output_def` degrades per-element, never whole-def
An element whose stored `props` JSON fails to parse is SKIPPED with a `tracing::warn!` — the def
still returns. Order is deterministic: scenes by (kind: base<overlay, then position, then id),
elements by z_order.

## Repository tests: isolated single-connection in-memory DB, not `connect_in_memory`
`Repository::connect_in_memory()` uses `sqlite::memory:?cache=shared` — ALL such connections in the
process share ONE DB, so count/list assertions contaminate across tests. `stream_tests.rs` instead
builds an isolated repo: `ConnectOptions` with `max_connections(1).min_connections(1)`,
`execute_unprepared("PRAGMA foreign_keys = ON")` (required for the FK CASCADE-delete tests to fire),
`Migrator::up`, then `Repository { db }` (the `db` field is `pub(crate)`). Copy this idiom for any
stream repo test.

## JSON test fixtures with hex colors break `r#"..."#`
A raw-string fixture containing a color like `"#ffffff"` contains the sequence `"#`, which CLOSES an
`r#"..."#` raw string early → a confusing `unknown prefix` parse error. Use `r##"..."##` for any
JSON fixture that embeds a `#rrggbb`/`#rrggbbaa` value. Caught only by `cargo fmt` locally (Tier-0),
so run `cargo fmt --all --check` before committing stream fixtures. (Building the fixtures with
`serde_json::json!({...})` instead of raw strings sidesteps this entirely — the `#` lives in a
normal string literal — and is what the #707 router tests do.)

## Companion `stream_*` commands (companion/stream.rs, #711)
Wire contract is FIXED (arch §7 + the built JS plugin `ops/companion/presenter/lib/stream.js`):
`stream_scene_set` / `stream_overlay_on` / `_off` / `_toggle` → `{scene, output?}`;
`stream_scene_clear` / `stream_clear` → `{output?}`. `output` defaults to `"stream"` (both plugin- and
server-side); the payload structs use `#[serde(deny_unknown_fields)]` (the plugin sends exactly those
fields). Scenes are addressed by NAME, case-insensitively — there is NO scene-by-name repository query,
so `companion/stream.rs` resolves name→id via `load_output_def(slug)` + a `to_lowercase()` match
(consistent with the repo's own uniqueness rule; unique-per-output ⇒ first match is unambiguous, and it
can't leak across outputs since the def is per-slug). Execute via the `AppState::stream_activate_scene /
stream_set_overlay / stream_clear` methods (NEVER the repository directly) so `LiveEvent::StreamState`
fires. Kind validation is delegated to those methods (base-as-overlay / overlay-as-base →
`RepositoryError::Invalid`); surface EVERY refusal (unknown output/scene, wrong kind, bad payload) as the
non-fatal `OutgoingMessage::Error` reply the plugin logs — never a panic. Variables `stream_scene`
(active base name or `-`) / `stream_overlays` (comma-joined active overlay names or `-`) are resolved
async in the live-loop (see `live-events.md`), not in the sync `apply_live_event`. The dispatch arm goes
in `protocol.rs::handle_command` (where every command dispatches, post the protocol.rs extraction), NOT
literally in `mod.rs` — the arch's "mod.rs" wording predates that split.

## Server-side (state/router) tests share the in-memory DB → own a UNIQUE slug per test (#706/#707)
The repository-test isolation idiom above is NOT available from `presenter-server`: `Repository { db }`
is `pub(crate)` to `presenter-persistence`, so a server test cannot build the isolated single-connection
repo. `AppState::in_memory()` uses `Repository::connect_in_memory()` = `sqlite::memory:?cache=shared`,
so ALL server tests in the process share ONE DB. Existing server tests avoid this by never asserting a
global list/count on a shared table. Stream state/router tests must do the same: **each test creates its
own uniquely-slugged output** (`s706-<name>` / `t-<name>`) and operates on THAT — never the seeded
`stream` output — or two tests activating `stream` race on `active_scene_id`. The per-`AppState`
`LiveHub` is NOT shared, so hub-subscription assertions (`live_hub().subscribe()` + `try_recv`) are
safe as-is.

## Stream API is SLUG-addressed; show state lives in `/def` + the outputs list (no `/show` route)
Output-scoped routes take the output SLUG, not the numeric id: `/stream/api/outputs/{slug}/scenes`,
`.../active-scene` (PUT, body `{"sceneId":N}`), `.../overlays/{scene_id}` (PUT), `.../clear` (POST),
`.../def` (GET). Only scene/element mutation routes take numeric ids (`/stream/api/scenes/{id}`,
`/stream/api/elements/{id}`). There is NO `/show` endpoint — read current show state from
`GET /stream/api/outputs` (summaries) or `GET /stream/api/outputs/{slug}/def` (`activeSceneId`,
`configRevision`). A verification curl against `/outputs/1/...` 404s with `"stream output not found"`
(the numeric string is looked up as a slug).

## Element create/PATCH body is the RAW props object, flattened — NOT `{"props":…}`
`POST /stream/api/scenes/{id}/elements` and `PATCH /stream/api/elements/{id}` take the RAW
`StreamElementProps` JSON DIRECTLY as the whole body: the serde-tagged enum flattened at the top
level (the `kind` tag + that kind's fields, top-level snake_case; a nested `frame`/`TextStyle` is
camelCase). It is NOT wrapped in `{"props": {…}}` — wrapping it 422s with `missing field \`kind\``
(the handler deserialises the body straight into `StreamElementProps`, and `kind` is then absent at
the top level). The WASM editor does this correctly (`components/stream_editor/mod.rs::save_props`
PATCHes `&props`); a hand-rolled curl must send the flattened object, e.g.
`{"kind":"image","asset_id":2,"fit":"contain","frame":{"xPct":10,"yPct":10,"wPct":30,"hPct":20},"opacity":1}`.
Verified live 2026-08-24 (#751). PATCH replaces props only — `z_order` is preserved (reorder is the
`/elements/order` route); `frame` position may be off-canvas (x/y negative or past 100, bounded by
`presenter_core::STREAM_FRAME_POS_*`) — the editor warns when fully off-canvas but never clamps (#751).

## Frame validation is SPLIT (position off-canvas-OK vs size positive) — a bound change touches 3 test layers
`validate_frame` (core `stream.rs`) is NOT one range: `validate_frame_pos` (x/y) allows off-canvas
`STREAM_FRAME_POS_MIN_PCT..=MAX_PCT` (-200..=300) so an element can slide in / bleed off / move up
past the top; `validate_frame_size` (w/h) stays `0 < v <= STREAM_FRAME_SIZE_MAX_PCT` (300). `validate_pct`
(0..=100) is now ONLY for text `size_pct` — do not fold frame back into it. When you change a frame
bound, a hardcoded boundary-value assertion lives in THREE places and Tier-0 (no local build) surfaces
a miss only at CI: the core unit tests (`stream.rs` `#[cfg(test)]`), the server router test
(`crates/presenter-server/src/router/stream_tests.rs` — the `xPct` "invalid → 422" case), AND the
editor E2E (`tests/e2e/stream-editor.spec.ts` — the `stream-frame-x` "invalid → inline 422" case).
`#751` widened 100→300 and both the router test and the E2E asserted `xPct=150 → 422`, which silently
became VALID; grep every `150`/`xPct`/`out of range` frame assertion before pushing a bound change.
New `StreamValidationError` variants are additive-safe (no exhaustive `match` on it outside `stream.rs`),
but a new `pub const` needs adding to the `lib.rs` `pub use stream::{…}` re-export or `presenter_core::NAME`
won't resolve (per `quality-gates.md`; #751 review 🔵).

## Asset pipeline on disk (#708)
Uploaded images live at `<stream_assets_dir>/<sha256>.<ext>` (ext ∈ png|jpg|webp), one file per
sha256 (dedup), NOT DB blobs. The bytes layer is `state/stream_assets.rs` (`AssetStore` + pure
`detect_image`/`sha256_hex`/`image_dims`/`resolve_dir`); the metadata row is #705's repository.
- **Where the dir comes from:** `AppState.stream_assets_dir` (one field, resolved once at construction
  via `stream_assets::resolve_dir()`): `PRESENTER_STREAM_ASSETS_DIR` → sibling of `PRESENTER_DB_URL`'s
  sqlite file → cwd `./stream-assets`. Prod/dev units set both `WorkingDirectory` and `PRESENTER_DB_URL`,
  so all paths land on `<deploy-dir>/stream-assets`. Handlers get `state.asset_store()`; tests set an
  isolated `TempDir` via `#[cfg(test)] set_stream_assets_dir`. The startup `ensure_stream_assets_dir()`
  is wired in `main.rs` — and is therefore `pub`, not `pub(crate)`: `main.rs` is a SEPARATE crate root
  from the lib, so a `pub(crate)` method called only from `main.rs` is dead code to clippy (`-D warnings`
  failed CI on exactly this, Pipeline 32455457688). Any `AppState` method `main.rs` calls must be `pub`.
- **Route split: upload is `POST /stream/assets` (multipart field `file`), list is `GET /stream/api/assets`**
  — a `POST /stream/api/assets` 405s. Verification curl: `curl -F 'file=@x.png;type=image/png' $P/stream/assets`.
- **axum default body limit is 2 MiB** — ANY multipart/upload route MUST add
  `.layer(DefaultBodyLimit::max(N))` (on the `post(..)` MethodRouter) or real images 413 before the
  handler. The 20 MiB business cap is enforced separately in the handler (precise 413 + message);
  the layer is a higher DoS ceiling.
- **`tokio::fs` works in presenter-server** even though the workspace `tokio` features omit `fs` —
  it's on via feature unification (`ai/proxy.rs` uses it in prod). Don't waste time re-adding the feature.
- **Serve/delete are id-addressed, traversal-safe:** `GET/DELETE /stream/assets/{id}` (DB i64 id); the
  file name is built from the stored sha256 + mime→ext, never client input. `AssetStore::path_for`
  refuses anything not `^[0-9a-f]{64}$` + whitelisted ext. Serve sets
  `Cache-Control: public, max-age=31536000, immutable`. Delete = row first (guard → 409 via
  `ConflictDetail`) then best-effort file removal.
- **Deploy survival:** the deploy `rsync --delete` is scoped to `…/libraries/`, never the deploy-dir
  root; the binary is `scp`'d. So `stream-assets/` (sibling of `libraries/`) is never deleted — no
  workflow change needed; documented in `docs/configuration.md`.
- **`NewStreamAsset` is now re-exported at the persistence crate root** (`presenter_persistence::NewStreamAsset`),
  not only under the private `repository` module.
## WASM output page (`/stream/{slug}`, #709+) — page-lane gotchas

- **OBS transparency needs an `html` override, not just `body`.** `styles/tablet.css` ships a BARE
  GLOBAL `html { background:#1e293b }` that trunk bundles into EVERY page. A transparent stream page
  must force BOTH transparent: the page sets `class="stream"` on `<body>` AND `data-stream="true"` on
  `document.documentElement`, and `styles/stream_output.css` overrides
  `html[data-stream="true"]` + `body.stream` to `background: transparent !important`. Scope every new
  stream CSS rule to `html[data-stream]`/`body.stream`/`.stream-*` (bundled globally, must not touch
  other pages). E2E asserts `getComputedStyle(document.documentElement).backgroundColor` == `rgba(0, 0, 0, 0)`.
- **The timer model has NO id-addressable registry.** `presenter_core::TimersOverview` has exactly
  `countdown_to_start` (a `CountdownTimerSnapshot`) + `preach_timer` — there is no per-id timer. The
  Countdown element's `timer_id` prop is currently FORWARD-LOOKING: every countdown binds to
  `countdown_to_start`. Reuse `presenter_core::format_countdown(seconds_remaining)` (the shared stage
  formatter). Tick smoothly BETWEEN server `Timers` pushes with a received-at delta —
  `seconds_remaining - floor((now_ms - received_at_ms)/1000)` — never absolute `target - Date::now()`
  (client-clock-skew footgun) and never server-cadence-only (no smooth tick). Render nothing when
  there is no snapshot, when `state == TimerState::Idle` (the un-started default), or when
  `format_countdown` returns "".
- **Parallel-lane WS decoupling: parse NEW LiveEvents from RAW JSON, don't import them.** The
  `stream_state`/`stream_config_changed` variants live in `presenter-core::live::LiveEvent` on the
  #706 branch. A WASM page lane that must compile BEFORE #706 merges parses those two frames as
  `serde_json::Value` (switch on `"type"`) into LOCAL mirror structs matching the #706 wire shape
  (`tag="type"`, snake_case, extra `type` field ignored — no `deny_unknown_fields`), and parses the
  already-existing `Timers`/`Heartbeat` through the real `LiveEvent` type. The wire shape IS the
  contract, so nothing changes at integration. Model the reconnect/backoff/zombie hook on
  `ws/stage.rs`; a stream output page is a passive CONSUMER — send NO presence and NO heartbeat ACK
  (like the plain `ws/mod.rs` hook), just read + reset the zombie deadline on any frame.
- **Client def-sync rule (arch §6/§8):** apply `StreamState` directly; refetch the def on an unknown
  scene id OR a higher `config_revision`, on `StreamConfigChanged`, and on every WS reconnect (the
  live hub does not replay). Derive show-state from the def on cold load
  (`active_scene_id` + overlay `is_active` flags). The OBS output page is deliberately CHROME-FREE:
  NO version label (it would leak onto the broadcast) — the project's version display lives on the
  operator UI, not here.
- **Route precedence:** register `GET /stream/{slug}` (the WASM shell, mirror `stage_shell.rs`) AFTER
  the `/stream/api/*` (#707) + `/stream/assets/*` (#708) prefixes; axum 0.8 / matchit 0.8 resolve a
  static segment before a `{param}` sibling, and `api`/`assets` are reserved slugs, so no collision.

## Content elements — lyrics + verse (#710)

- **Content REUSES existing events; events that EXIST on-tree are IMPORTED, not mirror-parsed.** The
  raw-JSON mirror-struct trick above is ONLY for the not-yet-merged #706 events. Lyrics bind to
  `LiveEvent::Stage { snapshot }` (tag `stage`) → `snapshot.current` (a `StageDisplaySlide`:
  `.main`/`.translation`); verse binds to `LiveEvent::BibleSlide { output: BibleSlideOutput }`
  (tag `bible_slide`: `main_text`/`main_reference`/`secondary_text`/`secondary_reference`) and
  `LiveEvent::BibleCleared` (tag `bible_cleared`). All three exist in `presenter-core::live` today,
  so `ws/stream.rs` parses them through the REAL `LiveEvent` type in `handle_text`. `bible_slide`
  ⇒ `Some(output)`, `bible_cleared` ⇒ `None` — one `RwSignal<Option<BibleSlideOutput>>` (None = no
  verse). Stage never needs a separate clear event: a broom/clear arrives as a `Stage` snapshot with
  `current == None`, which the lyrics element already renders as nothing.
- **`Stage.current` (main/translation) is LAYOUT-INDEPENDENT — the surface=stream client gets worship
  content under ANY operator layout.** `state/stage.rs::build_stage_snapshot` sets
  `current: context.resolution.current.clone()` for EVERY layout, and
  `state/broadcasting.rs::publish_stage_context` ALWAYS publishes the camera-crew snapshot (same
  `current`) alongside (or, for the `api` layout, instead of) the operator-layout snapshot. So a
  worship trigger reaches `/live/ws?surface=stream` with a populated `current` regardless of the
  globally-selected stage layout — no `?layout=` workaround is needed for the LIVE path. Cold-load
  uses the selected `GET /stage/snapshot` (agrees with the live path's last event for worship layouts;
  it reflects the default first-presentation slide on a never-triggered server, same as the stage page).
- **Cold-load content on WS connect (parity with the countdown's `/timers/overview`).** A reconnecting
  OBS source must recover the CURRENT lyrics/verse, not wait for the next trigger — so the connected
  Effect also `spawn`s `GET /stage/snapshot` → `ctx.stage` and `GET /bible/active-slide`
  (`Option<BibleSlideOutput>`) → `ctx.bible`. Both API clients already exist (`api::stage::get_snapshot`,
  `api::bible::get_active_slide_output`). Wire the WS `stage` signal only-Some (a clear is a snapshot
  with `current: None`); wire the WS `bible` signal Some AND None (None IS the `BibleCleared` clear).
- **Leptos double-move trap: a non-`Copy` `String` style can be captured by only ONE `<Show>` child
  closure.** `<Show>` children are `move` closures that must OWN their captures. A single `line_style(...)`
  String reused across two `<Show>` children (e.g. the same `reference_style` for BOTH the main and
  secondary reference lines) is a double-move compile error — build ONE owned String per `<Show>`
  child (`main_reference_css` + `secondary_reference_css`). Inside each child, `style=css.clone()`
  (the child is `Fn`, may run on every mount, so clone). The reactive text closures (`main_text` etc.)
  ARE reusable in both the `<Show when=>` guard and the `{text}` child because they capture only the
  `Copy` `ctx.stage`/`ctx.bible` signal, so the closure is itself `Copy` (read the needed field via
  `.with()`, never `.get()` the whole large snapshot). This whole crate is Tier-0 (no local
  `cargo check`), so a move error only surfaces in the trunk build / `cargo test --lib` at CI — catch
  it by reading.
- **The 4000-char / overflow AC is satisfied by `.stream-element { overflow:hidden }`** (already in
  `stream_output.css`) plus per-line `white-space: pre-wrap; overflow-wrap: anywhere` — text wraps
  within the Frame and anything past the box is clipped; the container never grows past its Frame
  height. Assert `getComputedStyle(el).overflow === "hidden"` + the container's `boundingClientRect`
  height stays ≈ the Frame `h_pct`% of the viewport.
- **Shared text-style CSS helpers live in `components/stream/style.rs`** (`frame_css`, `text_style_css`,
  `css_font_family`/`css_align`/`css_justify`) — used by countdown/lyrics/verse. Add a new text element
  by reusing these, not by re-inlining the mapping (the #709 countdown was refactored onto them).

## Transitions — scene crossfade + content fade/cut (#716)

- **CSS crossfade primitive = `@starting-style` + a SINGLE `opacity` transition + inline dynamic
  `transition-duration`.** Resting `opacity:1`; `@starting-style { .X { opacity:0 } }` fades a
  freshly-inserted node IN with zero JS timing (no next-frame `requestAnimationFrame`/0-ms `Timeout`);
  a `--leaving` class (`opacity:0`) fades it OUT; `transition-duration` set inline per node (dynamic
  per scene/element). One animated property throughout ⇒ a mid-fade interruption (rapid A→B→A)
  interpolates smoothly. Chosen over `@keyframes` (animation-vs-transition conflict, pop on interrupt)
  and JS-rAF (timing fragility). Works in OBS CEF (Chromium 120+) + Playwright chromium.
- **Fade-OUT before unmount needs a leaving BUFFER — Leptos has no "animate before removal" for
  changing keyed content.** Keep the outgoing node in the list with a `leaving` flag, schedule its
  removal with `gloo_timers::callback::Timeout::new(dur + ~80, ...).forget()`, and remove via
  `layers.try_update(|ls| ls.retain(...))` — `try_update` is DISPOSE-SAFE (a Timeout firing after the
  page/element disposed is a no-op; the crate can't use `on_cleanup` for a `!Send` gloo timer).
- **KEY layers on a monotonic `seq`, never the scene/content id.** A scene/text re-entering while its
  previous copy is still leaving (A→B→A) would key-collide on the natural id.
- **The keyed-`<For>` reactive-field trap (#496/#693) applies to EVERY mutable per-layer field, not
  just the obvious one.** A keyed `<For>` does NOT re-run `children` when a field flips, so BOTH
  `leaving` AND `duration_ms` must be read REACTIVELY by seq (`Signal::derive(move || layers.with(...))`),
  and the child must apply them via reactive closures (`class:...=leaving`, `style=move || format!(...)`).
  Real #716 bug: `duration_ms` was passed as a plain `u32` baked into the inline style at creation, so
  `mark_leaving` re-pointing the outgoing base to the incoming scene's duration was a DEAD WRITE — the
  scene faded over its creation duration while its removal timeout used the new one → a pop when
  per-scene overrides differ. Tier-0 hides this (no local build); catch it by reading + review.
- **Content fade = a reusable `components/stream/transition.rs::CrossfadeText`.** Props: `text:
  Memo<String>` (the Memo dedups so a per-250 ms-re-derived countdown crossfades only on the per-second
  value change), a `ContentTransition`, wrapper role/class/style, a `fill` flag. It stacks layers in
  ONE CSS grid cell (`.stream-crossfade { display:grid }`, layers `grid-area:1/1`; `--fill` =
  `grid-template-columns: minmax(0,1fr)` so lyrics/verse text wraps, countdown stays content-sized).
  `Fade` marks old layers leaving + adds new; `Cut` replaces atomically (never 2 layers). It renders
  the wrapper ONLY when a layer exists, so empty/cleared content is DOM-ABSENT — preserving the #710
  `toHaveCount(0)` count-0-on-clear/toggle contract. Keep the outer element's text-style on the ELEMENT
  (countdown) so the layers INHERIT it and the #709 font-size/text-shadow asserts on the element still pass.
- **Scene reconcile = a `RwSignal<Vec<SceneLayer>>` + one Effect on `def`+`show_state`.** A
  `config_revision` bump ⇒ rebuild fresh (config edits are not the smooth path); same revision ⇒
  crossfade the base + reconcile overlays individually. Read `layers` only via `with_untracked`/`update`
  inside the Effect (never tracked `.get()`) or it self-triggers. Keep base layers before overlays with
  a STABLE sort (`isolation:isolate` ⇒ cross-scene layering is DOM order); gate the sort on an actual
  base add (a new base is pushed at the vec end and must move before overlays; overlays pushed at the
  end are already ordered). Incoming scene's `transition_ms ?? default_transition_ms` governs both fades;
  a clear uses the outgoing scene's own duration.
- **Countdown content-transition default is `Fade{300}` from core** (`ContentTransition::default()`),
  but the recommended default for a countdown is `Cut` (per-second fades look wrong). The OUTPUT page
  HONORS whatever is stored; setting the `Cut` default belongs in the EDITOR's new-element defaults
  (`stream_editor.rs`, not built yet) — a CONTRACT-ASSUMPTION for the editor lane, not a core-default change.

## Bundled OFL fonts (#717) — trunk `copy-dir` + relative `@font-face url()`

The three whitelisted OFL families (`STREAM_FONT_FAMILIES`: Inter / Bebas Neue / Oswald) are
bundled as latin-subset `woff2` so the OBS output page renders identically OFFLINE at the rig:
- Files live in `crates/presenter-ui/fonts/*.woff2` (+ `OFL.txt`). Grab the latin subset from Google
  Fonts with a Chrome User-Agent so the API serves `woff2` (`curl -A "Mozilla/5.0 … Chrome/120 …"
  "https://fonts.googleapis.com/css2?family=Inter:wght@100..900"` → take the `/* latin */` block's
  gstatic `url()` → `curl -A "<chrome UA>" -o <file>.woff2 <url>`; gstatic 404s without the UA).
- `index.html` carries `<link data-trunk rel="copy-dir" href="fonts">`; trunk copies the dir to
  `dist/fonts/`, served under `/ui-pkg/fonts/` by `wasm_ui_asset` (`include_dir!` of `../presenter-ui/dist`).
- `@font-face` lives in `styles/stream_output.css` with a RELATIVE `src: url("fonts/<f>.woff2")` — trunk
  copies `rel="css"` verbatim (no `url()` rewrite), so it resolves against the CSS's own `/ui-pkg/` path.
  The `font-family` name MUST match `components/stream/style.rs::css_font_family`'s output (`"Inter"` /
  `"Bebas Neue"` / `"Oswald"`), which keeps a `system-ui, sans-serif` fallback; add `font-display: swap`.
- `wasm_ui.rs::mime_from_path` serves `.woff2` as `font/woff2` (the browser uses the `format("woff2")`
  hint regardless, but correct MIME is cheap). A wrong `@font-face` path 404s and the zero-console E2E
  gate catches it — so a spec that renders text in a bundled family (e.g. countdown=Oswald,
  lyrics/verse=Inter, verse-reference=Bebas Neue) verifies the bundling end-to-end. Bebas Neue ships
  ONE weight (400); request weight 400 in a `TextStyle` to avoid synth-bold.
