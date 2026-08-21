---
paths:
  - "crates/presenter-core/src/stream.rs"
  - "crates/presenter-persistence/src/repository/stream*.rs"
  - "crates/presenter-server/src/**/stream*.rs"
  - "crates/presenter-ui/src/**/stream*.rs"
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
so run `cargo fmt --all --check` before committing stream fixtures.

## Asset pipeline on disk (#708)
Uploaded images live at `<stream_assets_dir>/<sha256>.<ext>` (ext ∈ png|jpg|webp), one file per
sha256 (dedup), NOT DB blobs. The bytes layer is `state/stream_assets.rs` (`AssetStore` + pure
`detect_image`/`sha256_hex`/`image_dims`/`resolve_dir`); the metadata row is #705's repository.
- **Where the dir comes from:** `AppState.stream_assets_dir` (one field, resolved once at construction
  via `stream_assets::resolve_dir()`): `PRESENTER_STREAM_ASSETS_DIR` → sibling of `PRESENTER_DB_URL`'s
  sqlite file → cwd `./stream-assets`. Prod/dev units set both `WorkingDirectory` and `PRESENTER_DB_URL`,
  so all paths land on `<deploy-dir>/stream-assets`. Handlers get `state.asset_store()`; tests set an
  isolated `TempDir` via `#[cfg(test)] set_stream_assets_dir`. The startup `ensure_stream_assets_dir()`
  is wired in `main.rs`.
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
