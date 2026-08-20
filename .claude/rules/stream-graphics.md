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
so run `cargo fmt --all --check` before committing stream fixtures.

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
