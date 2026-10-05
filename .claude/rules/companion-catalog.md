---
paths:
  - "ops/companion/presenter/**"
  - "crates/presenter-server/src/companion/**"
---

# Companion dropdown choices come from the server `catalog` push — never a hardcoded list (#814)

Owner (2026-10-05): "ma to byt inteligentne loadovat si veci". A dropdown whose choices exist
in presenter (layouts, stream outputs, scenes, overlays, plates) is FED BY THE SERVER, never a
static list in `index.js`. The old hardcoded `STAGE_LAYOUT_CHOICES` never got `api` /
`api-ambient` (#799).

## Server (`crates/presenter-server/src/companion/catalog.rs`)

- The outgoing message is `{type:"catalog", layouts:[{code,name}], stream:[{slug,name,
  scenes:[{name,kind}]}]}`.
- Layouts come from `StageDisplayLayout::operator_selectable()`, NOT `built_in()`. The
  `stage.layout` command refuses `camera-crew` (`validate_operator_selectable`).
- Stream data comes from `list_stream_outputs` + `load_output_def`, scenes in def order.
- The snapshot lives in `CompanionVariableState`; `apply_catalog` returns whether it changed.
- `send_snapshot` (`protocol.rs`) sends variables + nameplates + catalog on connect and on lag
  recovery.
- `handle_live_event` re-resolves on `StreamConfigChanged` and sends ONLY when the content
  changed. An element edit re-resolves to the same catalog, so nothing is sent.
- The session subscribes to the live hub BEFORE its initial snapshot, so a change that lands
  during connect is never lost. The WS test in `catalog_tests.rs` relies on that ordering.
- A NEGATIVE "nothing was sent" assertion over the real socket needs an ORDERING BARRIER, never a
  timeout or a ping: the session handles live events one at a time in hub order, so publish a
  marker event right after the one under test (`state.set_broadcast_live(true)`) and read frames
  until its `variables` frame arrives — any `catalog` frame before it is the bug. Without the
  barrier, a later DB write can be seen by the earlier event's refresh and mask a broken gate
  (#814 review). `tokio::select!` picks branches randomly, so a ping/pong orders nothing.
- Tests use an ISOLATED temp-file DB (`catalog_tests.rs::isolated_state`). The shared
  `AppState::in_memory()` DB would let a parallel test's new output change the catalog and race
  the "unchanged → no re-send" assertions.
- Residual: deleting an output publishes no live event. It stays in the dropdown until the next
  config change or reconnect. That is harmless, because `allowCustom` keeps stored values working.

## Module (`ops/companion/presenter/`)

- `lib/catalog.js` holds the pure logic: `normaliseCatalog`, `catalogEquals`,
  `stageLayoutChoices`, `streamOutputChoices`, `streamSceneChoices`.
- The `case "catalog"` arm in `index.js` stores the catalog, then re-runs `_setupActions()` +
  `_setupFeedbacks()` + `checkFeedbacks()`. It skips an identical catalog (a reconnect).
- Until the first catalog arrives, `FALLBACK_STAGE_LAYOUT_CHOICES` and the default output apply.
  An older server never sends a catalog, so the module degrades gracefully.
- Saved buttons keep working because every catalog dropdown keeps the SAME option id as the old
  list / text input, and is `allowCustom: true`. A stored string, even one not in the catalog,
  is passed through unchanged. So textinput → dropdown needs no upgrade script; a KEY rename
  still does (`companion-upgrade-scripts.md`).
- Scenes are a union across outputs, because a Companion dropdown cannot depend on another
  option's value. They are de-duplicated case-insensitively (the server matches names that way)
  and labelled with their output(s) when there is more than one output.
- The scene/overlay FEEDBACKS offer only the default output's scenes, because
  `stream_scene` / `stream_overlays` track only `stream`.

## Adding a new catalog-driven field

1. Extend `CompanionCatalog` on the server, plus a `catalog_tests.rs` assertion.
2. Extend `normaliseCatalog` and add a choice builder in `lib/catalog.js`.
3. Keep the option id and set `allowCustom: true`.
4. Add a legacy-value press test to `lib/catalog.test.js`.
