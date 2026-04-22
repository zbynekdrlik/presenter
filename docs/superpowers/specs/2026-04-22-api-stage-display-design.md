# API Stage Display Design

> **Date:** 2026-04-22 | **Status:** Approved

## Problem

An external custom application needs to drive a stage display in Presenter with minimal latency. The app has its own data for current/next slide text, song names, and group names. It needs a simple REST endpoint to push this data, and Presenter should render it using the same visual layout as worship-snv (group pills with WCAG contrast, auto-fit text, status bar).

## Design

### API Contract

Single endpoint, full state replacement:

```
PUT /api/stage
Content-Type: application/json

{
  "currentText": "Haleluja, haleluja",
  "nextText": "Spievajte Hospodinovi",
  "currentGroup": "Vsetci",
  "nextGroup": "Zeny",
  "currentSong": "Haleluja",
  "nextSong": "Spievajte"
}
```

- All fields are strings. Missing or `null` fields default to `""`.
- Response: `204 No Content` (no body — fastest possible response).
- No read-back endpoint. Push only.

### Server Architecture

**State storage:** New `api_stage: Arc<RwLock<ApiStageState>>` field on `AppState`. Pure in-memory — no database persistence. If the server restarts, the API stage is blank until the custom app pushes again.

**`ApiStageState` struct:**

```rust
struct ApiStageState {
    current_text: String,
    next_text: String,
    current_group: String,
    next_group: String,
    current_song: String,
    next_song: String,
}
```

Default: all fields are empty strings.

**Request flow:**

1. `PUT /api/stage` → deserialize JSON into `ApiStageState`
2. Resolve group colors for `current_group` and `next_group` via the existing `resolve_group_color()` cache (database-backed, auto-generates for unknown groups)
3. Convert to `StageDisplaySnapshot` with layout metadata set to the "api" layout
4. Store the `ApiStageState` in `api_stage`
5. Broadcast `LiveEvent::Stage { snapshot }` only to WebSocket clients subscribed to the "api" layout
6. Return `204`

**Selective broadcast:** The existing `StagePresence { layout_code }` message already tells the server which layout each connected stage client uses. The API stage broadcast filters by `layout_code == "api"` so it does not overwrite normal stage clients, and normal stage updates do not overwrite API stage clients.

### Layout Registration

Add `"api"` to the `BUILT_IN_LAYOUTS` array in `presenter-core/src/stage_display.rs`:

- Code: `api`
- Name: `API`
- Description: `External API-driven stage display`

This makes it appear in the stage layout dropdown in the operator header alongside worship-snv, timer, etc.

### Frontend / Display

No new WASM component. The stage page router maps the `"api"` layout code to the existing `WorshipSnv` component. The data path is identical to normal worship-snv:

1. `LiveEvent::Stage` arrives via WebSocket
2. Updates `ctx.snapshot` signal
3. `WorshipSnv` component reactively re-renders

Group pills use the same `group_pill_style()` with WCAG luminance-based black/white text contrast. Colors come from the `group_colors` database table, auto-generated for unknown groups via FNV-1a hash.

Visually identical to worship-snv — same auto-fit text, same status bar, same CSS.

### Independence from Normal Stage

The API stage state is completely separate from Presenter's internal slide system:

- Normal stage updates (slide changes, timer updates) do not affect the API stage
- API pushes do not affect the normal stage
- Both can run simultaneously on different stage clients

### Data Defaults

If a field is missing from the JSON payload, it defaults to `""`. This means:

- Missing `currentText` → the current slide area is blank
- Missing `currentGroup` → no group pill is shown (same behavior as worship-snv with no group)
- Missing `currentSong` → no song name displayed

This is the "fail safe" behavior — partial pushes result in empty display areas, not stale data.

## Testing

### Unit Tests

- `ApiStageState` deserialization: full payload, partial payload (missing fields → empty), empty body
- Conversion to `StageDisplaySnapshot`: verify group colors are resolved, layout metadata is correct, empty fields produce empty slide fields

### E2E Playwright Test

1. Set stage layout to "api"
2. `PUT /api/stage` with test data (currentText, currentGroup "Vsetci", etc.)
3. Open `/stage` in Playwright
4. Assert current text displays correctly
5. Assert group pill has correct background color and WCAG-compliant text color
6. Assert next text and next group are displayed
7. Push empty state → assert all fields clear
8. Zero browser console errors

## Files Changed

| File | Change |
|------|--------|
| `crates/presenter-core/src/stage_display.rs` | Add "api" to `BUILT_IN_LAYOUTS` |
| `crates/presenter-server/src/state/mod.rs` | Add `api_stage: Arc<RwLock<ApiStageState>>` to `AppState` |
| `crates/presenter-server/src/state/broadcasting.rs` | Add selective broadcast filtering by layout code |
| `crates/presenter-server/src/router/api_stage.rs` | New file: `PUT /api/stage` handler |
| `crates/presenter-server/src/router.rs` | Register `/api/stage` route |
| `crates/presenter-ui/src/pages/stage.rs` | Map "api" layout to `WorshipSnv` component |
| `tests/e2e/api-stage.spec.ts` | New E2E test |

## Out of Scope

- Read-back endpoint (GET)
- WebSocket input from the custom app
- Any visual differences from worship-snv
- Persistence of API stage state across server restarts
