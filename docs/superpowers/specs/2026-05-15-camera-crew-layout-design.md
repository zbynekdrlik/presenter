# Camera-Crew Layout — Design Spec

Closes #311.

## Goal

Add a new always-on browser view dedicated to the video/camera crew. Shows current and upcoming worship groups large and visible, plus secondary info (timers, song, library, on-air status). View is pinned and is NOT affected by operator-side stage-layout changes.

## Why

The video director and camera crew run a separate monitor. They need to anticipate group/section transitions ("which singer is singing, which group will sing next") so they can plan camera moves. Today they share the operator's `/stage` view, which means an operator switching stage layouts disrupts their reference. They also have no future-group horizon — only `current.group` and `next.group` are visible.

## User-visible behavior

A new URL `/ui/camera`. Opens to a dark, big-text monitor layout. Updates in real time over WebSocket. Layout priorities, per service-team requirements:

1. **Current group** — huge color pill, dominant visual.
2. **Next distinct group** — big pill below.
3. **Three more upcoming distinct groups** — smaller strip below.
4. **Bottom bar (compressed, low priority)** — song name · library, preach timer, countdown timer, ON-AIR indicator, version + WS latency.

When the operator changes the `/stage` layout (worship-pp → preach → bible → …), the camera-crew view is unaffected.

## Architecture

### Server-side

1. **New stage layout variant** in `presenter-core::StageDisplayLayout::built_in()`:
   ```rust
   Self::new("camera-crew", "CAMERA CREW", "Group-focused director / camera-crew monitor")
   ```
   This layout is selectable via the snapshot/broadcast pipeline but is hidden from the operator's stage-layout picker UI (operator only picks among the regular layouts).

2. **New snapshot field**: `StageDisplaySnapshot.upcoming_groups: Vec<UpcomingGroup>` where:
   ```rust
   #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
   #[serde(rename_all = "camelCase")]
   pub struct UpcomingGroup {
       pub name: String,
   }
   ```
   - `#[serde(skip_serializing_if = "Vec::is_empty")]` on the field so existing payloads stay unchanged.
   - Vec entries are distinct groups ahead of `current_position` in the resolved slide list, deduped consecutively, max 4 entries (1 next-big + 3 next-distinct). The first entry of `upcoming_groups` IS the "next group" pill (so the camera-crew client uses `upcoming_groups[0]` for the BIG next pill and `[1..=3]` for the small strip).
   - Computed in `crates/presenter-server/src/state/stage.rs` during stage-context resolution. Colors are NOT included on the wire — client resolves group → color locally via the existing `api::presentations::fetch_group_colors()` map (same path used by the operator slide list).

3. **Dual-publish hook** in `state/broadcasting.rs::publish_stage_context`: after publishing the operator-selected snapshot, also build and publish a `"camera-crew"`-tagged snapshot from the same `StageContext`:
   - Skip if the operator-selected layout IS `"camera-crew"` (no double publish).
   - Skip if the layout is `"api"` (api stage path already short-circuits — preserve that).
   - `LiveEvent::Stage { snapshot }` fires with `snapshot.layout.code == "camera-crew"`. Camera-crew clients subscribe.

4. **Snapshot fetch endpoint extension**: `/stage/snapshot` accepts an optional `?layout=<code>` query parameter. When provided, returns `stage_display_snapshot(<code>)` regardless of the server's selected stage layout. Default behavior unchanged (uses `selected_stage_display_snapshot`).

5. **New WASM shell route**: `/ui/camera` → `wasm_ui::wasm_ui_shell` (same shell as `/ui/operator`, `/ui/tablet`). Router determines the page from URL.

### Client-side (WASM)

6. **New page** `crates/presenter-ui/src/pages/camera.rs`:
   - Mirrors `pages/stage.rs` but:
     - `StageContext` initialized with layout_code `"camera-crew"`, **never updated** from `LiveEvent::StageLayout` events.
     - Initial fetch calls `/stage/snapshot?layout=camera-crew` (not `/stage/snapshot` and not `/stage/layout`).
     - WS subscription identical (reuses `ws::stage::use_stage_websocket`).
     - Renders `<CameraCrew>` component unconditionally — no layout-variant match.
   - Hosted by the same router as other WASM pages.

7. **New component** `crates/presenter-ui/src/components/stage/camera_crew.rs`:
   - Reads from `StageContext`: `snapshot`, `broadcast_live`.
   - Renders the layout (see "Layout details" below).

8. **CSS** in `crates/presenter-ui/styles/stage.css`:
   - New class block `.stage__camera-crew` for the camera-crew layout root.
   - Subclasses for `__group-current` (huge), `__group-next` (big), `__group-future` (small strip), `__footer-bar`, `__on-air` (dim/bright).

## Layout details

```
┏━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃                                                        ┃
┃   ████████ VERSE 1 ████████                            ┃
┃   (~30vh, group_color background, huge font)           ┃
┃                                                        ┃
┃   Next: ████ CHORUS ████   (~15vh, group_color)        ┃
┃                                                        ┃
┃   Then: VERSE 2 · BRIDGE · CHORUS   (~5vh, small)      ┃
┃                                                        ┃
┃   ──────────────────────────────────────────────       ┃
┃                                                        ┃
┃   Song · LIBRARY    PREACH 14:32  COUNTDOWN 00:42      ┃
┃   ● ON AIR (red dot when BroadcastLive=true)           ┃
┃   v0.4.78 · 12ms                                       ┃
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛
```

All group pill colors are resolved CLIENT-SIDE via the existing `api::presentations::fetch_group_colors()` map (fetched once on page mount, keyed by group name). Server snapshots only carry group NAMES — no color field is added to `UpcomingGroup` and `StageDisplaySlide.group_color` is not relied upon (it is currently always `None` server-side; that stays unchanged).

- **Current group pill**: occupies the top ~50% of the viewport. Label from `snapshot.current.group`. Background color resolved via the local color map; if absent from the map, use a neutral default (`var(--stage-group-pill-default-bg)` or equivalent existing CSS variable). If `current.group` is `None`, render a dimmed placeholder pill with no label.
- **Next group pill**: ~15vh, label from `snapshot.upcoming_groups[0]`. Color resolved via the same client-side map. If `upcoming_groups` is empty, hide this pill.
- **Future-groups strip**: small horizontal strip below, labels from `snapshot.upcoming_groups[1..=3]`. Renders as dot-separated label list, each label tinted by the same color-map lookup. Hidden if no entries.
- **Footer bar** (low importance, compressed at ~12vh total):
  - Top of bar: `<song_name> · <library_name>` left, `PREACH <preach>` middle, `COUNTDOWN <countdown>` right.
  - Bottom of bar: `● ON AIR` (red filled circle + label) when `broadcast_live=true`, dim/grey when false. Version label + latency badge to the right.

## Distinct-group computation (pure function in `state/stage.rs`)

```rust
/// Given an iterator over upcoming slide group-names (already in slide order,
/// `None` allowed for ungrouped slides), return up to `max` distinct group
/// labels with consecutive duplicates collapsed. Ungrouped slides are skipped
/// (they don't break a run — a same-group repeat after a None still counts as
/// the same group). Order preserved.
pub fn upcoming_distinct_groups<'a, I>(groups: I, max: usize) -> Vec<UpcomingGroup>
where
    I: IntoIterator<Item = Option<&'a str>>,
{
    let mut out: Vec<UpcomingGroup> = Vec::new();
    let mut last_pushed: Option<String> = None;
    for entry in groups {
        let Some(name) = entry else { continue };
        if last_pushed.as_deref() == Some(name) {
            continue;
        }
        out.push(UpcomingGroup { name: name.to_string() });
        last_pushed = Some(name.to_string());
        if out.len() >= max {
            break;
        }
    }
    out
}
```

Called during stage-context resolution with `max = 4`. Caller feeds the iterator from the resolved slide list (the same source that populates `current_index` / `total_slides` today — see `state/stage.rs::build_stage_context`), skipping slides at and before `current_index`. Returns 0–4 entries.

Reference: existing code at `crates/presenter-core/src/slide.rs:261` uses `s.effective_group.as_ref().map(|g| g.name().to_string())` to extract group names per slide — same pattern.

## Routing summary

| URL | Purpose | Layout source |
|---|---|---|
| `/stage` | Operator/wall stage view (existing) | Server-selected layout, broadcast via `StageLayout` event |
| `/ui/operator` | Operator UI (existing) | n/a |
| `/ui/camera` | **NEW** — camera-crew view | Pinned client-side to `"camera-crew"`, ignores `StageLayout` events |

## Error handling

- WS disconnected → show full-screen "OFFLINE" overlay (reuse `worship_snv` pattern).
- No active presentation → render placeholder dashes for all group slots, hide footer song+library text. Timers and ON AIR still render if their data exists independently.
- Snapshot has no `current.group` → render placeholder pill with dimmed border + no label.

## Testing

### Unit (Rust)

- `upcoming_distinct_groups` — empty slides, single group, alternating groups, repeated same-group slides, partial fill (only 2 distinct groups left).
- `StageDisplaySnapshot` serde round-trip including `upcoming_groups`.
- `state/broadcasting::publish_stage_context` publishes BOTH operator-selected snapshot AND camera-crew snapshot on state change.
- Operator-selected snapshot is unchanged when camera-crew dual-publish is added (regression guard).

### Server integration

- `GET /stage/snapshot?layout=camera-crew` returns a snapshot with `layout.code == "camera-crew"` regardless of the server's currently-selected stage layout.

### E2E Playwright (`tests/e2e/wasm-stage-camera-crew.spec.ts`)

- Open `/ui/camera`, assert `body[data-layout-code="camera-crew"]`.
- Set a presentation active via API, navigate slide to a known position, assert:
  - Current-group pill shows correct label.
  - Next-group big pill shows correct label.
  - Future-groups strip shows 0–3 distinct labels.
- Operator changes layout to `"preach"` via `POST /stage/layout` — camera-crew page still shows `data-layout-code="camera-crew"`, still updates on slide changes.
- Toggle `BroadcastLive` ON via API — ON-AIR indicator becomes red.
- Browser console clean (zero errors / warnings).

### Regression test gate

Issue #311 is a feature request, not a bug fix — `regression-test-first.md` does NOT require a RED-before-GREEN commit pair. Tests still ship in the same PR.

## Files

| Action | Path | Purpose |
|---|---|---|
| Modify | `crates/presenter-core/src/stage_display.rs` | Add `camera-crew` to `built_in()`, add `UpcomingGroup` struct, add `upcoming_groups` field on `StageDisplaySnapshot` (and to `::new` signature) |
| Modify | `crates/presenter-server/src/state/stage.rs` | Add `upcoming_distinct_groups` pure function; populate `upcoming_groups` in `build_stage_snapshot` from the resolution slide list |
| Modify | `crates/presenter-server/src/state/broadcasting.rs` | In `publish_stage_context`, also publish a `"camera-crew"`-tagged snapshot |
| Modify | `crates/presenter-server/src/router/stage.rs` | Accept optional `?layout=<code>` on `/stage/snapshot` |
| Modify | `crates/presenter-server/src/router.rs` | Add `/ui/camera` route → wasm shell |
| Modify | `crates/presenter-ui/src/api/stage.rs` | New `get_snapshot_for(code: &str)` helper |
| Create | `crates/presenter-ui/src/pages/camera.rs` | Camera-crew page (StageContext pinned, ignores StageLayout) |
| Modify | `crates/presenter-ui/src/pages/mod.rs` | Export new page |
| Modify | `crates/presenter-ui/src/lib.rs` or router-equivalent | Wire `/ui/camera` → `CameraPage` |
| Create | `crates/presenter-ui/src/components/stage/camera_crew.rs` | Layout component |
| Modify | `crates/presenter-ui/src/components/stage/mod.rs` | Export `CameraCrew` |
| Modify | `crates/presenter-ui/styles/stage.css` | New `.stage__camera-crew*` rules |
| Create | `tests/e2e/wasm-stage-camera-crew.spec.ts` | E2E spec |
| Modify | `crates/presenter-core/src/stage_display.rs` tests, `crates/presenter-server/src/state/tests.rs`, `crates/presenter-server/src/state/broadcasting.rs` tests | Unit + integration coverage |

## Scope estimate

| Area | Lines |
|---|---|
| Server (core types + computation + dual-publish + route) | ~150 |
| WASM page + component | ~150 |
| CSS | ~80 |
| E2E + unit tests | ~120 |
| **Total** | **~500** |

🔴 Solo PR (over 300 LoC, cross-cuts core/server/ui).

## Versioning

Workspace currently at 0.4.78 (post-#318 merge). First commit of this work bumps to 0.4.79 per `core/version-bumping.md`.

## Non-goals

- Operator-controlled band-name / team-name field (user confirmed library name is sufficient).
- Lyrics preview on camera-crew page.
- Click-track or BPM info.
- Multiple simultaneous camera-crew views with different presets.
- Persisting camera-crew URL in session state.
- Authentication on `/ui/camera` (read-only; LAN trust model matches `/stage`).
