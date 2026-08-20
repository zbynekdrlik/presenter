# ADR 0009: Stream Graphics — a Resolume-replacement stream output

## Status

Accepted — 2026-08-20

Architecture ratified in the stream-graphics epic (#718, design synthesis 2026-08-16).
This ADR is committed with PR-1 (#703), which lands the data schema + entities; the
remaining subsystem ships across epic issues #704–#717.

## Context

The STREAM output (the composited graphics fed into OBS as a browser source) is currently
authored in Resolume Arena. We want to replace it with a first-party, nameable, transparent
web page that renders one exclusive BASE scene plus any set of OVERLAY scenes, switched by
Bitfocus Companion, and authored in an in-app operator editor. The design goal is good
foundations ("dobre zaklady") that may be reused: N outputs from day one, typed element
props, and reuse of the existing live-content pipelines (Timers / Stage / BibleSlide) rather
than new content feeds.

The example scenes the church uses map cleanly onto the model: `ytfast` / `5 min` / `1 min`
= an image + a countdown at increasing sizes; `chvaly` = lyrics (main + translation, fade
transition); overlay scenes = an image and/or a verse (a translation toggle).

## Decision

Build a `/stream/{slug}` WASM output page (transparent, an OBS browser source) plus a
`/ui/stream` WASM operator editor, backed by four SQLite tables, activation state persisted
in the DB, and two new small `LiveEvent` variants. Content binds to the EXISTING live
pipelines. Companion drives scene switching via a new `stream_*` command family.

### Component overview

```
Companion (Bitfocus)
   │ JSON/WS (companion sub-server, own port)
   ▼
companion/stream.rs ──► AppState (state/stream.rs: StreamManager, own lock)
                              │  persists activation + config to SQLite (repository/stream*.rs)
                              │  publishes LiveEvent::StreamState / StreamConfigChanged
                              ▼
                        LiveHub (/live/ws) ────────────────┐
                                                           ▼
Operator editor (/ui/stream, WASM) ──REST──► router/stream.rs   /stream/{slug} output page (WASM,
   │ preview iframe /stream/{slug}?preview=1&scene=..            transparent, in OBS browser source)
   └─ asset upload ──► router/stream_assets.rs ──► <workdir>/stream-assets/<sha256>.<ext>
                                                           ▲
Existing pipelines feed content UNCHANGED:                 │
  LiveEvent::Timers (countdown), LiveEvent::Stage{snapshot}│(lyrics),
  LiveEvent::BibleSlide/BibleCleared (verse) ──────────────┘
```

### Key decisions (recommendation + rejected alternative)

| # | Decision | Rejected + why |
|---|---|---|
| 1 | **Output page = WASM stage-style page** at `/stream/{slug}` (OBS is CEF/Chromium; WASM is proven long-running on the Android stage displays). | SSR + vanilla JS: would hand-roll DOM diffing, scene switching, per-element styling/transitions that Leptos gives for free; console-zero harder in hand-written JS. |
| 1b | **`stream_outputs` table for N nameable outputs now**; migration seeds one default `stream`. | A single hardcoded output: the table costs ~nothing now; retrofitting N later costs a schema PR + an OBS-URL migration. |
| 2 | **Structural columns + a typed JSON `props` column**: `kind`/`z_order`/`position`/`scene_id`/FKs are real columns; all style/config lives in `props`, typed by a serde-tagged `StreamElementProps`, validated on every write. | Fully normalized style tables: a migration per style knob + joins, for zero benefit on single-site config data. |
| 3 | **Active state persisted in the DB** (`stream_outputs.active_scene_id`; `stream_scenes.is_active` for overlays) + an in-memory cache. A cold OBS load / server restart restores the last look with no Companion action. | In-memory only: a restart blanks the stream until someone re-triggers. Settings-KV: not a user setting; would abuse the audited-setter path. |
| 4 | **Assets = hash-named files on disk** (`<workdir>/stream-assets/<sha256>.<ext>`) + a `stream_assets` metadata row; dedup by sha256; delete refused (409) while referenced. | DB blobs: bloat sqlite/backups. Unhashed filenames: no dedup, cache-busting pain. |
| 5 | **Companion command family `stream_*`** parsed in a NEW `companion/stream.rs`. | Growing `parse_command`/`handle_incoming_message` inline: function-length gate risk. |
| 6 | **Editor v1 = numeric property panel + live iframe preview**. Drag-on-canvas = v2. | Canvas drag in v1: no DnD lib in tree; a pointer-event editor is its own project. |
| 7 | **Transitions v1 = CSS crossfade only** (per-output default + per-scene override; per-element content fade/cut). | Wipes/slides/zooms in v1: scope; crossfade covers today's use. |
| 8 | **Data flow = fetch def + subscribe**: the page loads `GET /stream/api/outputs/{slug}/def`, then consumes existing LiveEvents for content + 2 new small state/config events. Config change ⇒ refetch def. | Pushing full scene defs over WS: big payloads, dual source of truth, lag-drop risk on the 256-cap hub. |
| 9 | Route family **`/stream/*`** (reserved slugs `api`, `assets`), not `/overlays/*`. | Reusing `/overlays` collides with the overlay-SCENE concept. |
| 10 | **No sync/LWW for stream config** (single-site). Stream tables are authored content: NOT wiped on dev deploy; the asset dir lives next to `presenter.db` and is untouched by deploys. | — recorded so a future multi-site pass knows it was deliberate. |

### Data model (this PR — incremental migration `m20260820_000001_create_stream_tables.rs`)

```
stream_outputs
  id INTEGER PK AUTOINCREMENT
  slug TEXT NOT NULL UNIQUE            -- ^[a-z0-9-]{1,64}$; reserved: "api","assets"
  name TEXT NOT NULL
  default_transition_ms INTEGER NOT NULL DEFAULT 400
  active_scene_id INTEGER NULL         -- NO FK (circular); repository clears on scene delete
  config_revision INTEGER NOT NULL DEFAULT 0
  created_at TEXT NOT NULL, updated_at TEXT NOT NULL   -- timestamptz, DEFAULT CURRENT_TIMESTAMP

stream_scenes
  id INTEGER PK AUTOINCREMENT
  output_id INTEGER NOT NULL REFERENCES stream_outputs(id) ON DELETE CASCADE
  name TEXT NOT NULL                   -- unique per (output_id, lower(name)), app-enforced 409
  kind TEXT NOT NULL                   -- 'base' | 'overlay'
  position INTEGER NOT NULL            -- L→R order within its kind
  is_active INTEGER NOT NULL DEFAULT 0 -- overlays only; base uses outputs.active_scene_id
  transition_ms INTEGER NULL           -- scene-switch override of output default
  created_at, updated_at

stream_elements
  id INTEGER PK AUTOINCREMENT
  scene_id INTEGER NOT NULL REFERENCES stream_scenes(id) ON DELETE CASCADE
  kind TEXT NOT NULL                   -- 'image'|'countdown'|'lyrics'|'verse'
  z_order INTEGER NOT NULL
  props TEXT NOT NULL                  -- JSON; must deserialize to StreamElementProps (tag == kind)
  created_at, updated_at

stream_assets
  id INTEGER PK AUTOINCREMENT
  sha256 TEXT NOT NULL UNIQUE
  original_filename TEXT NOT NULL
  mime TEXT NOT NULL                   -- image/png, image/jpeg, image/webp
  size_bytes INTEGER NOT NULL
  width INTEGER NULL, height INTEGER NULL
  created_at TEXT NOT NULL
```

Seed: one row `stream_outputs(slug='stream', name='Stream')` (`INSERT OR IGNORE`, so the
migration is idempotent). Indexes: unique on `slug` and `sha256`; FK-lookup indexes on
`stream_scenes.output_id` and `stream_elements.scene_id`.

Integer ids/FKs are stored as SQLite `INTEGER` (required for `AUTOINCREMENT`; every stream
value is comfortably within `i32` for this single-site domain). The `i64` in the Rust core
types (LiveEvents, `props.asset_id`/`timer_id`) is a downstream layer (#704+), not part of
this schema.

### Core types (later — presenter-core, #704)

`StreamElementProps` is a serde `#[serde(tag = "kind")]` enum over `Image` / `Countdown` /
`Lyrics` / `Verse`, each carrying a `Frame` (percent-of-16:9 canvas), a `TextStyle`
(size as % of canvas height ⇒ CSS `vh`), and a `ContentTransition` (`Cut` | `Fade{ms}`).
Validation (pct ranges, `#rrggbb[aa]` colors, `1≤weight≤1000`, slug regex + reserved list,
props-tag == the `kind` column) is unit-tested in core. Font families: a fixed v1 list
(system-ui, Arial + bundled OFL Inter / Bebas Neue / Oswald).

## Consequences

**Easier:** N nameable outputs and typed elements from day one; a cold OBS reload restores
the last look with no operator action; content reuses proven pipelines (no new feeds to keep
in sync); assets dedup by hash and are immutable-cacheable; the whole output is a plain
transparent web page (no Resolume licence, no separate machine).

**Harder / accepted cost:** a `props` JSON column trades queryability for flexibility
(mitigated by strict validation on every write); the circular `outputs↔scenes` reference
means `active_scene_id` has no FK and the repository must clear it on scene delete; the
single-file `entities.rs` grows past its 800-line soft target (847 prod lines) though it
stays well under the 1000 hard cap — a future split of that pre-existing single-file design
is tracked separately, not forced by this PR.

## Alternatives Considered

See the decisions table above (each row records the rejected alternative). At the subsystem
level the main forks were: WASM vs SSR+JS output page (dec. #1); an outputs table vs a single
hardcoded output (dec. #1b); structural columns + JSON props vs fully normalized style tables
(dec. #2); DB-persisted vs in-memory-only active state (dec. #3); on-disk hash-named assets vs
DB blobs (dec. #4); and reusing the existing content pipelines vs pushing full scene defs over
WS (dec. #8).

## Deploy / ops finding (acceptance criterion, #703)

Grepped every deploy workflow and script:

- The **dev-deploy wipe SQL** (`.github/workflows/pipeline.yml`, "Redirect integrations to
  mock endpoints") deletes ONLY `android_stage_displays` and `video_sources` (integration
  config that must point at mock endpoints on dev). **The four stream tables are in NO wipe
  list** — they are authored content (like libraries), not machine-specific integration
  config.
- Every `rsync -avz --delete` (in `pipeline.yml`, `deploy.yml`, `release.yml`,
  `import-data.yml`) targets the `${DEPLOY_DIR}/libraries/` **subdirectory only** — never the
  deploy-dir root. So `presenter.db` and a future sibling `stream-assets/` directory (created
  by the server at startup next to `presenter.db`) are untouched by any deploy.

OBS setup (later): browser source URL `http://<host>/stream/stream`, 1920×1080, transparent.

## Follow-up

The rest of the epic (#718): core types + validation (#704); repository CRUD + def assembly
(#705); state manager + LiveEvents (#706); REST routes (#707); WASM output page (#708);
operator editor (#709); asset upload/serve (#710); Companion `stream_*` commands (#711);
Companion node plugin (#712); and the E2E specs, docs, and OBS setup notes across the
remaining issues.

**Explicitly deferred to v2:** drag-on-canvas editing; custom font upload; wipe/slide/zoom
transitions; animated assets (webm/gif); countdown end-behaviors; text stroke/outline +
rotation; scene duplication; non-16:9 / multi-resolution outputs; NDI-out of stream graphics;
sync/LWW of stream config; Companion feedbacks beyond variables; an orphan-asset GC command;
per-overlay transition overrides.
