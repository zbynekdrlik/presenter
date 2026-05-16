# Camera-Crew Layout Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship `/ui/camera` — an always-on, group-focused stage layout for the video director / camera crew, pinned client-side and unaffected by operator-side stage-layout changes. Closes #311.

**Architecture:** Server adds a new built-in stage layout `camera-crew` plus a new snapshot field `upcoming_groups: Vec<UpcomingGroup>`. `state/broadcasting.rs::publish_stage_context` is changed to ALWAYS publish a `camera-crew`-tagged snapshot (even when the operator-selected layout is `api`). A new WASM page at `/ui/camera` initializes its `StageContext` pinned to `"camera-crew"`, fetches its own initial snapshot via `/stage/snapshot?layout=camera-crew`, and ignores `LiveEvent::StageLayout` broadcasts. Group colors are resolved client-side via the existing `fetch_group_colors()` map.

**Tech Stack:** Rust (axum, sea-orm), Leptos WASM, CSS, Playwright TypeScript.

**Spec:** `docs/superpowers/specs/2026-05-15-camera-crew-layout-design.md` (commit `ef65502`).

---

## Files map

| Action | Path | Purpose |
|---|---|---|
| Modify | `Cargo.toml` | Bump workspace version 0.4.78 → 0.4.79 |
| Modify | `crates/presenter-core/src/stage_display.rs` | New `UpcomingGroup` struct; new `upcoming_groups` field on `StageDisplaySnapshot`; new layout `camera-crew` in `built_in()` |
| Modify | `crates/presenter-server/src/state/stage.rs` | Pure fn `upcoming_distinct_groups`; extend `resolve_slide_positions` to surface upcoming groups; wire through `build_stage_snapshot` |
| Modify | `crates/presenter-server/src/state/broadcasting.rs` | Always publish camera-crew snapshot alongside operator-selected (or alone when api active) |
| Modify | `crates/presenter-server/src/router/stage.rs` | `/stage/snapshot` accepts optional `?layout=` query |
| Modify | `crates/presenter-server/src/router.rs` | New route `/ui/camera` |
| Modify | `crates/presenter-server/src/companion/tests.rs` | Update `StageDisplaySnapshot::new` calls (positional arg added) |
| Modify | `crates/presenter-server/src/state/mod.rs` | Update `StageDisplaySnapshot::new` call (positional arg added) |
| Modify | `crates/presenter-ui/src/api/stage.rs` | New helper `get_snapshot_for(layout)` |
| Create | `crates/presenter-ui/src/pages/camera.rs` | Pinned camera page (`StageContext::new("camera-crew")`) |
| Modify | `crates/presenter-ui/src/pages/mod.rs` | `pub mod camera;` |
| Modify | `crates/presenter-ui/src/lib.rs` | URL routing: `/ui/camera` → `<CameraPage/>` |
| Create | `crates/presenter-ui/src/components/stage/camera_crew.rs` | Layout component |
| Modify | `crates/presenter-ui/src/components/stage/mod.rs` | `pub mod camera_crew;` |
| Modify | `crates/presenter-ui/styles/stage.css` | `.stage__camera-crew*` rules |
| Create | `tests/e2e/wasm-stage-camera-crew.spec.ts` | E2E test |

---

### Task 1: Bump workspace version

**Files:**
- Modify: `Cargo.toml:15`

- [ ] **Step 1: Edit `Cargo.toml`**

Change line 15 from:

```toml
version = "0.4.78"
```

to:

```toml
version = "0.4.79"
```

- [ ] **Step 2: Refresh main Cargo.lock**

Run: `cargo update --workspace -p presenter-core -p presenter-server -p presenter-persistence -p presenter-migration -p presenter-importer -p presenter-bible -p presenter-ndi`

Expected: lockfile entries for those crates updated to `version = "0.4.79"`.

- [ ] **Step 3: Refresh presenter-ui Cargo.lock**

Run: `cd crates/presenter-ui && cargo update --workspace && cd ../..`

Expected: `crates/presenter-ui/Cargo.lock` entries for those workspace crates updated to `0.4.79`.

- [ ] **Step 4: Verify fmt + check**

Run: `cargo fmt --all --check && cargo check --workspace`

Expected: both succeed.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock crates/presenter-ui/Cargo.lock
git commit -m "chore: bump workspace version to 0.4.79 for camera-crew layout (#311)"
```

---

### Task 2: Add `UpcomingGroup` type and `camera-crew` built-in layout

**Files:**
- Modify: `crates/presenter-core/src/stage_display.rs`

- [ ] **Step 1: Write the failing test for `UpcomingGroup` serde**

Append to the existing tests section in `crates/presenter-core/src/stage_display.rs` (or create `#[cfg(test)] mod tests` if absent — there are already tests for `StageDisplaySnapshot` elsewhere in the workspace; this in-file test is local):

```rust
#[cfg(test)]
mod camera_crew_tests {
    use super::*;

    #[test]
    fn upcoming_group_round_trips_through_json() {
        let g = UpcomingGroup { name: "Verse 1".to_string() };
        let json = serde_json::to_string(&g).unwrap();
        assert_eq!(json, r#"{"name":"Verse 1"}"#);
        let back: UpcomingGroup = serde_json::from_str(&json).unwrap();
        assert_eq!(back, g);
    }

    #[test]
    fn built_in_layouts_include_camera_crew() {
        let codes: Vec<String> =
            StageDisplayLayout::built_in().into_iter().map(|l| l.code).collect();
        assert!(codes.iter().any(|c| c == "camera-crew"), "codes={codes:?}");
    }

    #[test]
    fn stage_display_snapshot_omits_empty_upcoming_groups_in_json() {
        let layout = StageDisplayLayout::built_in().into_iter().next().unwrap();
        let snap = StageDisplaySnapshot::new(
            layout,
            chrono::Utc::now(),
            None, None, None, None, None, None,
            None, None, None, None,
            crate::timer::TimersOverview::default(),
            None, None, None, None, None, None,
            Vec::new(), // upcoming_groups
        );
        let json = serde_json::to_string(&snap).unwrap();
        assert!(!json.contains("upcomingGroups"), "empty upcoming_groups must not serialize: {json}");
    }
}
```

- [ ] **Step 2: Run test — must fail**

Run: `cargo test -p presenter-core stage_display::camera_crew_tests --no-run 2>&1 | head -40`

Expected: compile error — `UpcomingGroup` not in scope, `StageDisplaySnapshot::new` arity mismatch.

- [ ] **Step 3: Add `UpcomingGroup` struct**

Insert ABOVE `pub struct StageDisplaySnapshot { ... }` in `crates/presenter-core/src/stage_display.rs` (around line 88):

```rust
/// One upcoming distinct group name for camera-crew layout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpcomingGroup {
    pub name: String,
}
```

- [ ] **Step 4: Add `upcoming_groups` field to `StageDisplaySnapshot`**

In `crates/presenter-core/src/stage_display.rs`, in the `StageDisplaySnapshot` struct (around line 88-130), add this field BEFORE the closing `}`:

```rust
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub upcoming_groups: Vec<UpcomingGroup>,
```

- [ ] **Step 5: Add `upcoming_groups` to `StageDisplaySnapshot::new`**

Update the `pub fn new(` signature in `impl StageDisplaySnapshot` (around line 186) — append one more parameter at the end:

```rust
        playlist_entries: Option<Vec<StagePlaylistEntry>>,
        upcoming_groups: Vec<UpcomingGroup>,
    ) -> Self {
```

In the constructor body (around line 207), add `upcoming_groups,` to the struct literal AFTER `playlist_entries,`:

```rust
            playlist_entries,
            upcoming_groups,
        }
    }
```

- [ ] **Step 6: Add `camera-crew` to `StageDisplayLayout::built_in()`**

In `crates/presenter-core/src/stage_display.rs::built_in()` (around line 53-54), insert BEFORE the closing `]`:

```rust
            Self::new(
                "camera-crew",
                "CAMERA CREW",
                "Group-focused director / camera-crew monitor",
            ),
```

- [ ] **Step 7: Update existing `StageDisplaySnapshot::new` callers**

In `crates/presenter-server/src/state/stage.rs` around line 218, in `build_stage_snapshot`, change the trailing argument from:

```rust
        context.resolution.playlist_entries.clone(),
    )
```

to:

```rust
        context.resolution.playlist_entries.clone(),
        Vec::new(), // upcoming_groups — populated in Task 3
    )
```

In `crates/presenter-server/src/state/mod.rs` around line 787, find the `StageDisplaySnapshot::new(` block and append at the end (before the closing `)`):

```rust
            None,           // playlist_entries
            Vec::new(),     // upcoming_groups (api layout has no upcoming context)
        )
```

In `crates/presenter-server/src/companion/tests.rs` at line 80 and line 117, append `Vec::new(),` to each `StageDisplaySnapshot::new(...)` call after the last positional arg:

```rust
        None,        // playlist_entries
        Vec::new(),  // upcoming_groups
    );
```

- [ ] **Step 8: Run tests — must pass**

Run: `cargo test -p presenter-core stage_display::camera_crew_tests`

Expected: 3 tests pass. Run `cargo build -p presenter-server --tests` to confirm callers compile.

- [ ] **Step 9: Commit**

```bash
git add crates/presenter-core/src/stage_display.rs crates/presenter-server/src/state/stage.rs crates/presenter-server/src/state/mod.rs crates/presenter-server/src/companion/tests.rs
git commit -m "feat(stage): add UpcomingGroup + camera-crew built-in layout + snapshot field (#311)"
```

---

### Task 3: Compute `upcoming_groups` during stage resolution

**Files:**
- Modify: `crates/presenter-server/src/state/stage.rs`

- [ ] **Step 1: Write failing test for `upcoming_distinct_groups`**

Append to the existing `#[cfg(test)] mod tests` in `crates/presenter-server/src/state/stage.rs` (the tests live after `build_stage_snapshot` — around line 400):

```rust
    #[test]
    fn upcoming_distinct_groups_collapses_consecutive_duplicates() {
        let names: Vec<Option<&str>> =
            vec![Some("Verse 1"), Some("Verse 1"), Some("Chorus"), Some("Verse 2")];
        let groups = upcoming_distinct_groups(names, 4);
        assert_eq!(
            groups.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(),
            vec!["Verse 1", "Chorus", "Verse 2"]
        );
    }

    #[test]
    fn upcoming_distinct_groups_skips_ungrouped() {
        let names: Vec<Option<&str>> = vec![None, Some("Verse 1"), None, Some("Verse 1"), Some("Chorus")];
        let groups = upcoming_distinct_groups(names, 4);
        assert_eq!(
            groups.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(),
            vec!["Verse 1", "Chorus"]
        );
    }

    #[test]
    fn upcoming_distinct_groups_caps_at_max() {
        let names: Vec<Option<&str>> =
            vec![Some("A"), Some("B"), Some("C"), Some("D"), Some("E"), Some("F")];
        let groups = upcoming_distinct_groups(names, 4);
        assert_eq!(groups.len(), 4);
        assert_eq!(groups.last().unwrap().name, "D");
    }

    #[test]
    fn upcoming_distinct_groups_empty_when_no_names() {
        let groups = upcoming_distinct_groups(Vec::<Option<&str>>::new(), 4);
        assert!(groups.is_empty());
    }
```

- [ ] **Step 2: Run tests — must fail**

Run: `cargo test -p presenter-server state::stage::tests::upcoming_distinct_groups --no-run 2>&1 | head -20`

Expected: compile error — `upcoming_distinct_groups` not found.

- [ ] **Step 3: Add the pure function**

In `crates/presenter-server/src/state/stage.rs`, insert this BEFORE `pub(crate) fn build_stage_snapshot` (around line 214):

```rust
/// Returns up to `max` distinct upcoming group names from an ordered iterator
/// of per-slide group names (`None` = ungrouped slide). Consecutive duplicates
/// are collapsed; ungrouped slides are skipped (they do not break a run).
pub(crate) fn upcoming_distinct_groups<'a, I>(
    groups: I,
    max: usize,
) -> Vec<presenter_core::UpcomingGroup>
where
    I: IntoIterator<Item = Option<&'a str>>,
{
    let mut out: Vec<presenter_core::UpcomingGroup> = Vec::new();
    let mut last_pushed: Option<String> = None;
    for entry in groups {
        let Some(name) = entry else { continue };
        if last_pushed.as_deref() == Some(name) {
            continue;
        }
        out.push(presenter_core::UpcomingGroup { name: name.to_string() });
        last_pushed = Some(name.to_string());
        if out.len() >= max {
            break;
        }
    }
    out
}
```

- [ ] **Step 4: Ensure `UpcomingGroup` is exported from `presenter_core`**

In `crates/presenter-core/src/lib.rs`, ensure `UpcomingGroup` is re-exported. Find the existing `pub use stage_display::{...}` line and add `UpcomingGroup` to its list. If `pub use stage_display::*;` is used, no change needed.

Run: `grep -n "UpcomingGroup\|pub use stage_display" crates/presenter-core/src/lib.rs`

If `UpcomingGroup` is missing from the re-export list, add it.

- [ ] **Step 5: Run unit tests — must pass**

Run: `cargo test -p presenter-server state::stage::tests::upcoming_distinct_groups`

Expected: 4 tests pass.

- [ ] **Step 6: Add `upcoming_groups` to `StageResolution`**

In `crates/presenter-server/src/state/stage.rs` around line 19-40, in the `StageResolution` struct, add this field BEFORE the closing `}`:

```rust
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) upcoming_groups: Vec<presenter_core::UpcomingGroup>,
```

In `StageResolution::cleared()` around line 45-60, add this field assignment:

```rust
            playlist_entries: None,
            upcoming_groups: Vec::new(),
        }
```

- [ ] **Step 7: Extend `resolve_slide_positions` to emit upcoming groups**

In `crates/presenter-server/src/state/stage.rs` around line 150 (the `resolve_slide_positions` function), change its return type and body.

Current return type: `ResolvedSlides<'a>`.

Add a new field to `ResolvedSlides`:

```rust
struct ResolvedSlides<'a> {
    current: Option<SlideCtx<'a>>,
    next: Option<SlideCtx<'a>>,
    upcoming_groups: Vec<presenter_core::UpcomingGroup>,
}
```

Modify `resolve_slide_positions` body. Inside the existing `for slide in &presentation.slides { ... }` loop, AFTER the existing `effective_group` mutation and AFTER setting `current_ctx`, collect a parallel `Vec<Option<String>>` of group names for slides AFTER the current slide. Simplest: do a SECOND pass after the first loop, once `current_order` is known.

Replace the existing tail of `resolve_slide_positions` (the `let resolved_current = ...` block and `ResolvedSlides { ... }` literal) with:

```rust
    let resolved_current = current_ctx.or_else(|| first.clone());
    let resolved_next = if let Some(next_ctx) = next_by_id {
        Some(next_ctx)
    } else if current_order.is_some() {
        next_after_current
    } else {
        second
    };

    // Build per-slide group names AFTER the current slide for upcoming_groups.
    let upcoming_names: Vec<Option<&str>> = {
        let mut active_group: Option<&str> = None;
        let mut collected: Vec<Option<&str>> = Vec::new();
        let mut past_current = current_order.is_none();
        for slide in &presentation.slides {
            if let Some(g) = slide.content.group.as_ref() {
                active_group = Some(g.name());
            }
            if past_current {
                collected.push(active_group);
            } else if Some(slide.order) == current_order {
                past_current = true;
            }
        }
        collected
    };
    let upcoming_groups = upcoming_distinct_groups(upcoming_names, 4);

    ResolvedSlides {
        current: resolved_current,
        next: resolved_next,
        upcoming_groups,
    }
}
```

Note: when no `current` is selected, `past_current` starts `true` so `collected` accumulates ALL slides from the top — meaning `upcoming_groups` previews the whole presentation. This is the right behavior: if nothing is current, the camera crew sees the structural plan.

- [ ] **Step 8: Wire `upcoming_groups` through to `StageResolution`**

In `crates/presenter-server/src/state/stage.rs` around line 132, in the `StageResolution { ... }` literal at the end of the non-empty branch, add the field assignment:

```rust
        playlist_entries: None,
        upcoming_groups: resolved.upcoming_groups,
    }
}
```

Also update the empty-slides branch around line 96 (`return StageResolution { ... }`) to add `upcoming_groups: Vec::new(),` before the closing brace:

```rust
            playlist_entries: None,
            upcoming_groups: Vec::new(),
        };
```

There is also a third construction at line 380 in `state/stage.rs` (inside a test). Update it similarly: append `upcoming_groups: Vec::new(),` inside the `StageResolution { ... }` literal.

- [ ] **Step 9: Update `build_stage_snapshot` to pass through `upcoming_groups`**

In `crates/presenter-server/src/state/stage.rs::build_stage_snapshot` (around line 218), change the trailing argument we set in Task 2 Step 7 from:

```rust
        context.resolution.playlist_entries.clone(),
        Vec::new(), // upcoming_groups — populated in Task 3
    )
```

to:

```rust
        context.resolution.playlist_entries.clone(),
        context.resolution.upcoming_groups.clone(),
    )
```

- [ ] **Step 10: Add integration test — upcoming_groups populated from a real presentation**

Append to `#[cfg(test)] mod tests` in `crates/presenter-server/src/state/stage.rs` (after the unit tests added in Step 1):

```rust
    #[test]
    fn resolve_stage_collects_upcoming_distinct_groups_after_current() {
        use presenter_core::{
            DurationMs, Presentation, PresentationId, Slide, SlideContent, SlideGroup,
            SlideId, SlideMetadata, SlideText,
        };

        fn slide(order: u32, group: Option<&str>) -> Slide {
            Slide {
                id: SlideId::new(uuid::Uuid::new_v4()),
                order,
                duration_ms: DurationMs(5_000),
                content: SlideContent {
                    main: SlideText::new(""),
                    translation: SlideText::new(""),
                    stage: SlideText::new(""),
                    group: group.map(SlideGroup::new),
                },
                metadata: SlideMetadata::default(),
            }
        }

        let slides = vec![
            slide(0, Some("Verse 1")),
            slide(1, None),
            slide(2, Some("Chorus")),
            slide(3, None),
            slide(4, Some("Verse 2")),
            slide(5, Some("Bridge")),
        ];
        let current_id = slides[0].id;
        let presentation = Presentation {
            id: PresentationId::new(uuid::Uuid::new_v4()),
            name: "Test".to_string(),
            slides,
        };

        let resolved =
            resolve_slide_positions(&presentation, Some(current_id), None);
        let names: Vec<&str> =
            resolved.upcoming_groups.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, vec!["Verse 1", "Chorus", "Verse 2", "Bridge"]);
    }
```

Note: the exact field shape of `Slide` / `SlideContent` may differ — refer to `crates/presenter-core/src/slide.rs` for the actual fields and adjust the helper accordingly (e.g. `duration_ms`, `metadata` may not exist; check the file). The shape MUST compile against current types.

- [ ] **Step 11: Run integration test — must pass**

Run: `cargo test -p presenter-server state::stage::tests::resolve_stage_collects_upcoming_distinct_groups_after_current`

Expected: pass.

- [ ] **Step 12: Full server test run**

Run: `cargo test -p presenter-server`

Expected: all tests pass (no regressions in existing snapshot tests).

- [ ] **Step 13: Commit**

```bash
git add crates/presenter-server/src/state/stage.rs crates/presenter-core/src/lib.rs
git commit -m "feat(stage): compute upcoming_groups during slide resolution (#311)"
```

---

### Task 4: Dual-publish camera-crew snapshot in broadcasting

**Files:**
- Modify: `crates/presenter-server/src/state/broadcasting.rs`

- [ ] **Step 1: Write failing integration test for dual-publish**

Append to `crates/presenter-server/src/state/tests.rs` (near other broadcasting tests — search for `publish_stage_context` references at lines 376-512 for examples of test setup):

```rust
    #[tokio::test]
    async fn publish_stage_context_emits_camera_crew_snapshot_alongside_operator_layout() {
        let state = build_test_state().await;
        state
            .set_stage_layout_code("worship-snv")
            .await
            .expect("set layout");
        seed_one_song_with_groups(&state).await;
        let mut receiver = state.live_hub.subscribe();
        state
            .broadcast_stage_snapshots()
            .await
            .expect("broadcast");

        let mut saw_worship = false;
        let mut saw_camera = false;
        for _ in 0..10 {
            match tokio::time::timeout(std::time::Duration::from_millis(200), receiver.recv()).await {
                Ok(Ok(LiveEvent::Stage { snapshot })) => match snapshot.layout.code.as_str() {
                    "worship-snv" => saw_worship = true,
                    "camera-crew" => saw_camera = true,
                    _ => {}
                },
                Ok(Ok(_)) => continue,
                _ => break,
            }
            if saw_worship && saw_camera {
                break;
            }
        }
        assert!(saw_worship, "expected worship-snv snapshot");
        assert!(saw_camera, "expected camera-crew snapshot alongside worship-snv");
    }

    #[tokio::test]
    async fn publish_stage_context_emits_camera_crew_snapshot_even_when_api_active() {
        let state = build_test_state().await;
        state
            .set_stage_layout_code("api")
            .await
            .expect("set layout");
        seed_one_song_with_groups(&state).await;
        let mut receiver = state.live_hub.subscribe();
        state
            .broadcast_stage_snapshots()
            .await
            .expect("broadcast");

        let mut saw_camera = false;
        for _ in 0..10 {
            match tokio::time::timeout(std::time::Duration::from_millis(200), receiver.recv()).await {
                Ok(Ok(LiveEvent::Stage { snapshot })) if snapshot.layout.code == "camera-crew" => {
                    saw_camera = true;
                    break;
                }
                Ok(Ok(_)) => continue,
                _ => break,
            }
        }
        assert!(saw_camera, "camera-crew snapshot must publish even when api layout is selected");
    }
```

If `build_test_state` or `seed_one_song_with_groups` helpers don't exist in `tests.rs`, mirror the patterns used by neighbouring tests (the file at lines 376-512 sets up state and exercises broadcasts — copy the setup pattern verbatim and adjust seed-data to include groups).

- [ ] **Step 2: Run tests — must fail**

Run: `cargo test -p presenter-server state::tests::publish_stage_context_emits_camera_crew --no-run 2>&1 | head -20`

Expected: compiles. If it runs, `saw_camera` is `false` → test fails. If unfamiliar helper functions are missing, fix the test code to use real helpers visible in tests.rs.

- [ ] **Step 3: Modify `publish_stage_context` for dual-publish**

In `crates/presenter-server/src/state/broadcasting.rs`, replace the body of `async fn publish_stage_context` (around lines 200-225). Current body:

```rust
    async fn publish_stage_context(&self, context: &StageContext) -> anyhow::Result<()> {
        let code = self.stage_layout_code().await;
        // The "api" layout is driven by PUT /api/stage, not by internal state.
        // Skip normal broadcasting to avoid overwriting API-pushed data.
        if code == "api" {
            return Ok(());
        }
        let mut layouts = StageDisplayLayout::built_in()
            .into_iter()
            .map(|layout| (layout.code.clone(), layout))
            .collect::<HashMap<_, _>>();
        let Some(layout) = layouts
            .remove(&code)
            .or_else(|| layouts.remove(DEFAULT_STAGE_LAYOUT_CODE))
        else {
            return Ok(());
        };

        let context = self.enrich_stage_context(context).await;
        let snapshot = build_stage_snapshot(layout, &context);
        self.publish_stage_update(snapshot);
        Ok(())
    }
```

New body:

```rust
    async fn publish_stage_context(&self, context: &StageContext) -> anyhow::Result<()> {
        let code = self.stage_layout_code().await;
        let context = self.enrich_stage_context(context).await;

        // Always publish camera-crew snapshot — its clients are pinned to /ui/camera
        // and must not be flipped by operator-side layout changes (including "api").
        if code != "camera-crew" {
            if let Some(camera_layout) = StageDisplayLayout::built_in()
                .into_iter()
                .find(|l| l.code == "camera-crew")
            {
                let camera_snapshot = build_stage_snapshot(camera_layout, &context);
                self.publish_stage_update(camera_snapshot);
            }
        }

        // The "api" layout is driven by PUT /api/stage, not by internal state.
        // Skip normal broadcasting for the operator-selected snapshot to avoid
        // overwriting API-pushed data.
        if code == "api" {
            return Ok(());
        }

        let mut layouts = StageDisplayLayout::built_in()
            .into_iter()
            .map(|layout| (layout.code.clone(), layout))
            .collect::<HashMap<_, _>>();
        let Some(layout) = layouts
            .remove(&code)
            .or_else(|| layouts.remove(DEFAULT_STAGE_LAYOUT_CODE))
        else {
            return Ok(());
        };

        let snapshot = build_stage_snapshot(layout, &context);
        self.publish_stage_update(snapshot);
        Ok(())
    }
```

- [ ] **Step 4: Run tests — must pass**

Run: `cargo test -p presenter-server state::tests::publish_stage_context_emits_camera_crew`

Expected: both tests pass.

- [ ] **Step 5: Run full server test suite to catch regressions**

Run: `cargo test -p presenter-server`

Expected: all green.

- [ ] **Step 6: Commit**

```bash
git add crates/presenter-server/src/state/broadcasting.rs crates/presenter-server/src/state/tests.rs
git commit -m "feat(stage): always publish camera-crew snapshot alongside operator layout (#311)"
```

---

### Task 5: `/stage/snapshot?layout=` query + `/ui/camera` route

**Files:**
- Modify: `crates/presenter-server/src/router/stage.rs`
- Modify: `crates/presenter-server/src/router.rs`

- [ ] **Step 1: Write failing integration test for `?layout=` query**

Append to existing integration tests (look in `crates/presenter-server/src/router/stage.rs` for `#[cfg(test)]` or in `crates/presenter-server/tests/` for HTTP-level tests; if none, create a unit test that calls the handler directly with a `Query` argument).

Direct handler test (place at the bottom of `crates/presenter-server/src/router/stage.rs`):

```rust
#[cfg(test)]
mod camera_snapshot_query_tests {
    use super::*;
    use crate::test_support::build_test_state_with_song;

    #[tokio::test]
    async fn snapshot_query_returns_requested_layout_regardless_of_global_selection() {
        let state = build_test_state_with_song().await;
        state.set_stage_layout_code("worship-snv").await.unwrap();
        let query = StageSnapshotQuery { layout: Some("camera-crew".to_string()) };
        let Json(snapshot) =
            stage_display_selected_snapshot_json(State(state), Query(query))
                .await
                .expect("ok");
        assert_eq!(snapshot.layout.code, "camera-crew");
    }
}
```

If `test_support::build_test_state_with_song` doesn't exist, factor out the existing test-state setup from `state/tests.rs` into a `pub(crate) mod test_support` module, OR write the test using the existing setup pattern inlined.

- [ ] **Step 2: Add `StageSnapshotQuery` and modify the handler**

In `crates/presenter-server/src/router/stage.rs`, add (near the other query/response structs):

```rust
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct StageSnapshotQuery {
    pub(super) layout: Option<String>,
}
```

Modify `stage_display_selected_snapshot_json` signature to take the query:

```rust
#[instrument(skip_all)]
pub(super) async fn stage_display_selected_snapshot_json(
    State(state): State<AppState>,
    Query(query): Query<StageSnapshotQuery>,
) -> Result<Json<StageDisplaySnapshot>, AppError> {
    let result = if let Some(code) = query.layout.as_deref() {
        state.stage_display_snapshot(code).await?
    } else {
        state.selected_stage_display_snapshot().await?
    };
    match result {
        Some(snapshot) => Ok(Json(snapshot)),
        None => Err(AppError::not_found("Stage display unavailable")),
    }
}
```

Ensure `axum::extract::Query` is in scope (`use axum::extract::Query;` at the top of the file if not already).

- [ ] **Step 3: Add `/ui/camera` route**

In `crates/presenter-server/src/router.rs` at line 131 (after `/ui/tablet`), insert:

```rust
        .route("/ui/camera", get(wasm_ui::wasm_ui_shell))
```

- [ ] **Step 4: Run tests — must pass**

Run: `cargo test -p presenter-server router::stage`

Expected: pass.

- [ ] **Step 5: Manual route check (smoke)**

Run: `cargo build -p presenter-server` then quick spawn:

```bash
cargo run -p presenter-server -- --port 18181 &
SERVER_PID=$!
sleep 4
curl -s -o /dev/null -w "%{http_code}\n" http://127.0.0.1:18181/ui/camera
curl -s "http://127.0.0.1:18181/stage/snapshot?layout=camera-crew" | head -c 200
kill $SERVER_PID
```

Expected: `200` for `/ui/camera`. The `/stage/snapshot?layout=camera-crew` returns a JSON body whose `"layout":{"code":"camera-crew"}`. If no presentation is loaded the request may 404 (`Stage display unavailable`); that is acceptable for this smoke — the route wiring is what matters.

- [ ] **Step 6: Commit**

```bash
git add crates/presenter-server/src/router/stage.rs crates/presenter-server/src/router.rs
git commit -m "feat(stage): /stage/snapshot accepts ?layout= query + new /ui/camera route (#311)"
```

---

### Task 6: WASM api helper `get_snapshot_for`

**Files:**
- Modify: `crates/presenter-ui/src/api/stage.rs`

- [ ] **Step 1: Add helper**

In `crates/presenter-ui/src/api/stage.rs`, near the existing `pub async fn get_snapshot()`, add:

```rust
pub async fn get_snapshot_for(layout: &str) -> Result<StageDisplaySnapshot, ApiError> {
    // urlencode the layout code defensively (camera-crew has no special chars
    // today, but `?layout=` is a contract surface, so keep this safe).
    let encoded = urlencoding::encode(layout);
    let path = format!("/stage/snapshot?layout={encoded}");
    get_json(&path).await
}
```

If `urlencoding` crate is not in `crates/presenter-ui/Cargo.toml`, replace the body with a direct concat (the only legal layout codes are ASCII kebab-case, so no encoding strictly needed):

```rust
pub async fn get_snapshot_for(layout: &str) -> Result<StageDisplaySnapshot, ApiError> {
    let path = format!("/stage/snapshot?layout={layout}");
    get_json(&path).await
}
```

Pick whichever is consistent with the existing dependency set. Default to the second form (no new dependency).

- [ ] **Step 2: Build check**

Run: `cd crates/presenter-ui && cargo check --target wasm32-unknown-unknown && cd ../..`

Expected: clean compile.

- [ ] **Step 3: Commit**

```bash
git add crates/presenter-ui/src/api/stage.rs
git commit -m "feat(ui): add stage::get_snapshot_for(layout) helper (#311)"
```

---

### Task 7: New WASM page `pages/camera.rs`

**Files:**
- Create: `crates/presenter-ui/src/pages/camera.rs`
- Modify: `crates/presenter-ui/src/pages/mod.rs`

- [ ] **Step 1: Create `pages/camera.rs`**

Create `crates/presenter-ui/src/pages/camera.rs` with this content:

```rust
use leptos::prelude::*;
use presenter_core::LiveEvent;
use wasm_bindgen::prelude::*;

use crate::api;
use crate::components::stage::camera_crew::CameraCrew;
use crate::state::stage::StageContext;
use crate::ws::stage::{self, StageWsState};

const CAMERA_LAYOUT: &str = "camera-crew";

#[component]
pub fn CameraPage() -> impl IntoView {
    if let Some(body) = crate::utils::window::document_body() {
        let _ = body.set_attribute("class", "stage");
    }

    let ctx = StageContext::new(CAMERA_LAYOUT.to_string());
    provide_context(ctx.clone());

    set_global_string("__presenterStageClientId", &ctx.client_id);
    set_global_string("__presenterStageLayout", CAMERA_LAYOUT);

    // Connect stage WebSocket — same subscription as /stage clients use.
    let ws_handle = stage::use_stage_websocket(ctx.client_id.clone(), ctx.layout_code);

    {
        let ws_state = ws_handle.state;
        Effect::new(move |_| {
            let state_str = match ws_state.get() {
                StageWsState::Connecting => "connecting",
                StageWsState::Connected => "connected",
                StageWsState::Reconnecting => "reconnecting",
                StageWsState::Disconnected => "disconnected",
            };
            set_global_string("__presenterStageConnectionState", state_str);
        });
    }

    // Handle WS events. CRITICAL: do NOT update layout_code from
    // LiveEvent::StageLayout — camera-crew is pinned. Other event arms mirror
    // pages/stage.rs for parity (broadcast, timers, ndi, bible overlay).
    {
        let ctx = ctx.clone();
        let last_event = ws_handle.last_event;
        Effect::new(move |_| {
            let Some(event) = last_event.get() else {
                return;
            };
            match event {
                LiveEvent::Stage { snapshot } if snapshot.layout.code == CAMERA_LAYOUT => {
                    ctx.snapshot.set(Some(snapshot));
                }
                LiveEvent::BibleSlide { output } => {
                    ctx.bible_overlay.set(Some(output));
                }
                LiveEvent::BibleCleared => {
                    ctx.bible_overlay.set(None);
                }
                LiveEvent::BroadcastLive { enabled } => {
                    ctx.broadcast_live.set(enabled);
                }
                LiveEvent::Timers { overview } => {
                    ctx.snapshot.update(|snap| {
                        if let Some(s) = snap {
                            s.timers = overview;
                        }
                    });
                }
                _ => {}
            }
        });
    }

    // Initial data fetch — pinned to camera-crew, ignores server-side global layout.
    {
        let ctx = ctx.clone();
        leptos::task::spawn_local(async move {
            if let Ok(snapshot) = api::stage::get_snapshot_for(CAMERA_LAYOUT).await {
                ctx.snapshot.set(Some(snapshot));
            }
            if let Ok(broadcast) = api::stage::get_broadcast_live().await {
                ctx.broadcast_live.set(broadcast.enabled);
            }
        });
    }

    // Sync body attributes for E2E test compatibility.
    {
        Effect::new(move |_| {
            if let Some(body) = crate::utils::window::document_body() {
                let _ = body.set_attribute("data-layout-code", CAMERA_LAYOUT);
            }
        });
    }

    let ws_state = ws_handle.state;
    let latency_ms = ws_handle.latency_ms;

    view! { <CameraCrew ws_state=ws_state latency_ms=latency_ms /> }
}

fn set_global_string(name: &str, value: &str) {
    let _ = js_sys::Reflect::set(
        &js_sys::global(),
        &JsValue::from_str(name),
        &JsValue::from_str(value),
    );
}
```

- [ ] **Step 2: Export the page from `pages/mod.rs`**

Open `crates/presenter-ui/src/pages/mod.rs` and add:

```rust
pub mod camera;
```

- [ ] **Step 3: Compile-check (without rendering — Task 9 creates `CameraCrew`)**

Step 3 is intentionally skipped — the WASM crate will not compile until the `CameraCrew` component is created in Task 9. Track this as an expected intermediate-state failure; the final compile gate is in Task 9.

- [ ] **Step 4: Commit (stage only the new page + mod.rs)**

```bash
git add crates/presenter-ui/src/pages/camera.rs crates/presenter-ui/src/pages/mod.rs
git commit -m "feat(ui): pinned /ui/camera page (#311)"
```

---

### Task 8: WASM URL routing → CameraPage

**Files:**
- Modify: `crates/presenter-ui/src/lib.rs`

- [ ] **Step 1: Add route match arm**

In `crates/presenter-ui/src/lib.rs` around line 43-46, the existing chain looks like:

```rust
        } else if p == "/ui/tablet" {
            view! { <pages::tablet::TabletPage /> }.into_any()
        } else {
            view! { <pages::stage::StagePage /> }.into_any()
```

Insert a new arm BEFORE the `else` final branch:

```rust
        } else if p == "/ui/tablet" {
            view! { <pages::tablet::TabletPage /> }.into_any()
        } else if p == "/ui/camera" {
            view! { <pages::camera::CameraPage /> }.into_any()
        } else {
            view! { <pages::stage::StagePage /> }.into_any()
```

- [ ] **Step 2: Skip standalone compile (still depends on Task 9)**

Same expected intermediate-state failure as Task 7 Step 3. Compile gate is in Task 9.

- [ ] **Step 3: Commit**

```bash
git add crates/presenter-ui/src/lib.rs
git commit -m "feat(ui): wire /ui/camera URL to CameraPage (#311)"
```

---

### Task 9: WASM component `CameraCrew`

**Files:**
- Create: `crates/presenter-ui/src/components/stage/camera_crew.rs`
- Modify: `crates/presenter-ui/src/components/stage/mod.rs`

- [ ] **Step 1: Create component**

Create `crates/presenter-ui/src/components/stage/camera_crew.rs` with:

```rust
use std::collections::HashMap;

use leptos::prelude::*;
use presenter_core::UpcomingGroup;

use crate::api;
use crate::components::version_label::VersionLabel;
use crate::state::stage::StageContext;
use crate::ws::stage::StageWsState;

#[component]
pub fn CameraCrew(
    ws_state: ReadSignal<StageWsState>,
    latency_ms: ReadSignal<Option<f64>>,
) -> impl IntoView {
    let ctx = use_context::<StageContext>().expect("StageContext provided by CameraPage");

    let group_colors = RwSignal::new(HashMap::<String, String>::new());
    {
        leptos::task::spawn_local(async move {
            if let Ok(colors) = api::presentations::fetch_group_colors().await {
                group_colors.set(colors);
            }
        });
    }

    let color_for = move |name: &str| -> Option<String> {
        group_colors.with(|map| map.get(name).cloned())
    };

    let current_group_label = move || {
        ctx.snapshot
            .get()
            .and_then(|s| s.current.and_then(|sl| sl.group))
            .unwrap_or_default()
    };

    let current_group_style = move || {
        let name = current_group_label();
        if name.is_empty() {
            return String::new();
        }
        color_for(&name)
            .map(|c| format!("background-color: {c};"))
            .unwrap_or_default()
    };

    let upcoming = move || {
        ctx.snapshot
            .get()
            .map(|s| s.upcoming_groups)
            .unwrap_or_default()
    };

    let next_group = move || upcoming().into_iter().next();
    let future_groups = move || -> Vec<UpcomingGroup> {
        upcoming().into_iter().skip(1).take(3).collect()
    };

    let song_label = move || {
        let snap = ctx.snapshot.get();
        let song = snap.as_ref().and_then(|s| s.song_name.clone()).unwrap_or_default();
        let library = snap.as_ref().and_then(|s| s.library_name.clone()).unwrap_or_default();
        match (song.is_empty(), library.is_empty()) {
            (false, false) => format!("{song} · {library}"),
            (false, true) => song,
            (true, false) => library,
            _ => String::new(),
        }
    };

    let preach_label = move || {
        ctx.snapshot
            .get()
            .map(|s| s.timers.preach.display.clone())
            .unwrap_or_else(|| "--:--".to_string())
    };

    let countdown_label = move || {
        ctx.snapshot
            .get()
            .map(|s| s.timers.countdown.display.clone())
            .unwrap_or_else(|| "--:--".to_string())
    };

    let on_air = move || ctx.broadcast_live.get();

    let latency_label = move || {
        latency_ms
            .get()
            .map(|ms| format!("{:.0}ms", ms))
            .unwrap_or_else(|| "—".to_string())
    };

    let connection_class = move || match ws_state.get() {
        StageWsState::Connected => "stage__camera-crew__conn stage__camera-crew__conn--ok",
        _ => "stage__camera-crew__conn stage__camera-crew__conn--bad",
    };

    view! {
        <div class="stage__camera-crew">
            <div class="stage__camera-crew__current stage__group-pill" style=current_group_style>
                {current_group_label}
            </div>

            <div class="stage__camera-crew__next">
                <span class="stage__camera-crew__next-label">"Next:"</span>
                {move || next_group().map(|g| {
                    let name = g.name.clone();
                    let style = color_for(&name).map(|c| format!("background-color: {c};")).unwrap_or_default();
                    view! {
                        <span class="stage__group-pill stage__camera-crew__next-pill" style=style>
                            {name}
                        </span>
                    }
                })}
            </div>

            <div class="stage__camera-crew__future">
                {move || future_groups()
                    .into_iter()
                    .map(|g| {
                        let name = g.name.clone();
                        let style = color_for(&name)
                            .map(|c| format!("background-color: {c};"))
                            .unwrap_or_default();
                        view! {
                            <span class="stage__group-pill stage__camera-crew__future-pill" style=style>
                                {name}
                            </span>
                        }.into_any()
                    })
                    .collect::<Vec<_>>()
                }
            </div>

            <div class="stage__camera-crew__footer">
                <span class="stage__camera-crew__song" data-testid="camera-crew-song">
                    {song_label}
                </span>
                <span class="stage__camera-crew__preach" data-testid="camera-crew-preach">
                    "PREACH "{preach_label}
                </span>
                <span class="stage__camera-crew__countdown" data-testid="camera-crew-countdown">
                    "COUNTDOWN "{countdown_label}
                </span>
                <span
                    class="stage__camera-crew__on-air"
                    class:is-on=on_air
                    data-testid="camera-crew-on-air"
                >
                    "● ON AIR"
                </span>
                <span class=connection_class>
                    <VersionLabel />" · "{latency_label}
                </span>
            </div>
        </div>
    }
}
```

If `s.timers.preach.display` / `s.timers.countdown.display` do not exist, inspect `crates/presenter-core/src/timer.rs::TimersOverview` for the actual display-string field name (likely `.formatted`, `.text`, or computed via a helper). Use the same field name `worship_snv.rs` / `preach_layout.rs` already use for their displayed timers — `grep -rn 'timers\.\(preach\|countdown\)' crates/presenter-ui/src/components/stage/` to find the canonical access pattern, and replace this code's `.display` references with that pattern verbatim.

- [ ] **Step 2: Export from `components/stage/mod.rs`**

Open `crates/presenter-ui/src/components/stage/mod.rs` and add:

```rust
pub mod camera_crew;
```

- [ ] **Step 3: Compile WASM (full gate)**

Run from repo root:

```bash
cd crates/presenter-ui && cargo check --target wasm32-unknown-unknown --all-targets && cd ../..
```

Expected: clean compile. Fix any borrow / signal / type errors before continuing. Most likely issues:
- `s.timers.preach.display` field path wrong → replace with the canonical pattern from `worship_snv.rs` or `preach_layout.rs`.
- `VersionLabel` not imported via the right path → grep existing pages for `use crate::components::version_label`.
- `presenter_core::UpcomingGroup` not reachable → ensure Task 3 Step 4 re-export landed.

- [ ] **Step 4: Run workspace clippy**

Run from repo root:

```bash
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all
```

And WASM clippy:

```bash
cd crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings -W clippy::all && cd ../..
```

Expected: zero warnings on both.

- [ ] **Step 5: Commit**

```bash
git add crates/presenter-ui/src/components/stage/camera_crew.rs crates/presenter-ui/src/components/stage/mod.rs
git commit -m "feat(ui): CameraCrew component renders group-focused layout (#311)"
```

---

### Task 10: CSS

**Files:**
- Modify: `crates/presenter-ui/styles/stage.css`

- [ ] **Step 1: Append camera-crew layout rules**

Append to `crates/presenter-ui/styles/stage.css`:

```css
/* === Camera-Crew layout ============================================== */
.stage__camera-crew {
    display: grid;
    grid-template-rows: 1fr 0.5fr 0.2fr 0.18fr;
    gap: 1.5vh;
    padding: 2vh 2vw;
    height: 100vh;
    box-sizing: border-box;
    background: var(--stage-bg, #050505);
    color: var(--stage-fg, #f0f0f0);
    font-family: 'Helvetica Neue', Arial, sans-serif;
}

.stage__camera-crew__current {
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: clamp(6rem, 18vw, 22rem);
    font-weight: 800;
    line-height: 1;
    text-transform: uppercase;
    border-radius: 1.5vh;
    background-color: var(--stage-group-pill-default-bg, #1a1a1a);
    color: #fff;
    padding: 0 4vw;
    overflow: hidden;
    white-space: nowrap;
}

.stage__camera-crew__next {
    display: flex;
    align-items: center;
    gap: 2vw;
    padding: 0 1vw;
}

.stage__camera-crew__next-label {
    font-size: 4vh;
    font-weight: 500;
    color: #aaa;
    letter-spacing: 0.05em;
}

.stage__camera-crew__next-pill {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: clamp(3rem, 9vw, 10rem);
    font-weight: 700;
    line-height: 1;
    text-transform: uppercase;
    border-radius: 1vh;
    padding: 1vh 3vw;
    background-color: var(--stage-group-pill-default-bg, #1a1a1a);
    color: #fff;
    min-height: 100%;
}

.stage__camera-crew__future {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 1.5vw;
    flex-wrap: nowrap;
    overflow: hidden;
}

.stage__camera-crew__future-pill {
    font-size: clamp(1.2rem, 3.5vw, 3.5rem);
    font-weight: 600;
    text-transform: uppercase;
    border-radius: 0.6vh;
    padding: 0.6vh 1.4vw;
    background-color: var(--stage-group-pill-default-bg, #1a1a1a);
    color: #ddd;
    opacity: 0.85;
}

.stage__camera-crew__footer {
    display: grid;
    grid-template-columns: minmax(0, 2fr) auto auto auto auto;
    align-items: center;
    gap: 2vw;
    font-size: 2.2vh;
    color: #ccc;
    border-top: 1px solid #222;
    padding-top: 1vh;
}

.stage__camera-crew__song {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
}

.stage__camera-crew__on-air {
    color: #444;
    font-weight: 700;
    letter-spacing: 0.08em;
    transition: color 120ms ease-out;
}

.stage__camera-crew__on-air.is-on {
    color: #ff2a2a;
    text-shadow: 0 0 1.2vh rgba(255, 42, 42, 0.6);
}

.stage__camera-crew__conn {
    font-size: 1.8vh;
    color: #888;
}

.stage__camera-crew__conn--bad {
    color: #ff8c00;
}
```

- [ ] **Step 2: Visual smoke (local trunk-build optional)**

This step is verified properly on dev after deploy in Task 12. Local CSS lint is not required.

- [ ] **Step 3: Commit**

```bash
git add crates/presenter-ui/styles/stage.css
git commit -m "feat(ui): styles for camera-crew layout (#311)"
```

---

### Task 11: Playwright E2E

**Files:**
- Create: `tests/e2e/wasm-stage-camera-crew.spec.ts`

- [ ] **Step 1: Write the spec**

Create `tests/e2e/wasm-stage-camera-crew.spec.ts`:

```typescript
import { test, expect } from '@playwright/test';
import { startTestServer, stopTestServer } from './support';

test.describe('/ui/camera — camera-crew layout', () => {
  let server: Awaited<ReturnType<typeof startTestServer>>;

  test.beforeAll(async () => {
    server = await startTestServer();
  });
  test.afterAll(async () => {
    if (server) await stopTestServer(server);
  });

  test('renders pinned camera-crew layout independent of operator layout switch', async ({ page }) => {
    const consoleErrors: string[] = [];
    page.on('console', (msg) => {
      if (msg.type() === 'error' || msg.type() === 'warning') {
        consoleErrors.push(`[${msg.type()}] ${msg.text()}`);
      }
    });

    await page.goto(`${server.baseUrl}/ui/camera`);
    await expect(page.locator('body')).toHaveAttribute('data-layout-code', 'camera-crew');
    await expect(page.locator('[data-testid="version"]')).toBeVisible();

    // Operator flips the global stage layout via the API. Camera page MUST stay
    // pinned to 'camera-crew'.
    const flip = await page.request.post(`${server.baseUrl}/stage/layout`, {
      data: { code: 'preach' },
    });
    expect(flip.ok()).toBeTruthy();

    // Allow live event to round-trip.
    await page.waitForTimeout(500);
    await expect(page.locator('body')).toHaveAttribute('data-layout-code', 'camera-crew');

    // Group pill exists (even if its label is empty when no presentation is loaded).
    await expect(page.locator('.stage__camera-crew__current')).toBeVisible();
    await expect(page.locator('.stage__camera-crew__footer')).toBeVisible();

    expect(consoleErrors).toEqual([]);
  });

  test('ON-AIR indicator reacts to BroadcastLive toggle', async ({ page }) => {
    await page.goto(`${server.baseUrl}/ui/camera`);
    await expect(page.locator('[data-testid="camera-crew-on-air"]')).toBeVisible();

    // Turn broadcast on via API.
    const on = await page.request.post(`${server.baseUrl}/stage/broadcast-live`, {
      data: { enabled: true },
    });
    expect(on.ok()).toBeTruthy();

    await expect(page.locator('[data-testid="camera-crew-on-air"]')).toHaveClass(/is-on/);

    const off = await page.request.post(`${server.baseUrl}/stage/broadcast-live`, {
      data: { enabled: false },
    });
    expect(off.ok()).toBeTruthy();
    await expect(page.locator('[data-testid="camera-crew-on-air"]')).not.toHaveClass(/is-on/);
  });
});
```

If `support.ts` does not export `startTestServer` / `stopTestServer`, copy the bootstrapping pattern used in `tests/e2e/operator-slide-save-indicator.spec.ts` or any other `wasm-stage-*.spec.ts` neighbour file. The startup pattern must produce an isolated server per spec, per project E2E convention (see memory `tests/e2e/` notes).

If `POST /stage/broadcast-live` is GET-only or shaped differently, find the actual contract — `grep -rn "broadcast-live" crates/presenter-server/src/router/stage.rs` shows the route + handler — and adjust the request (method, body shape) to match. The test must use the REAL endpoint shape, not a guessed one.

- [ ] **Step 2: Build + run E2E locally**

Run:

```bash
cargo build --release -p presenter-server
npm run test:playwright -- wasm-stage-camera-crew
```

Expected: both scenarios pass. Browser console clean.

- [ ] **Step 3: Commit**

```bash
git add tests/e2e/wasm-stage-camera-crew.spec.ts
git commit -m "test(e2e): camera-crew layout pinned + ON AIR reactivity (#311)"
```

---

### Task 12: Push, monitor CI, verify on dev, open PR (CONTROLLER-HANDLED)

**This task is handled by the controlling agent, not by an implementation subagent.**

- [ ] **Step 1: Local pre-push gate**

Run from repo root:

```bash
cargo fmt --all --check && \
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all && \
(cd crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings -W clippy::all) && \
cargo test --workspace
```

Expected: all green. If anything fails, FIX before pushing.

- [ ] **Step 2: Single push**

```bash
git push origin dev
```

Capture the run ID:

```bash
gh run list --branch dev --limit 1 --json databaseId,status,conclusion
```

- [ ] **Step 3: Monitor CI to terminal state**

Run in the background (per `core/ci-monitoring.md`):

```bash
sleep 300 && gh run view <run-id> --json status,conclusion,jobs
```

When the run completes, ALL jobs (pipeline build, e2e, deploy-dev) must be `conclusion: success`. Any failure → `gh run view <run-id> --log-failed`, fix root cause, push ONE fix commit, monitor again.

- [ ] **Step 4: Post-deploy verification on dev (Playwright)**

```bash
curl -s http://10.77.8.134:8080/healthz
```

Expected JSON includes `"version":"0.4.79","channel":"dev"`.

Then open Playwright on dev (NOT localhost):

1. Navigate to `http://10.77.8.134:8080/ui/camera`.
2. Read `[data-testid="version"]` from the DOM — must show `v0.4.79`.
3. Read `body[data-layout-code]` — must be `"camera-crew"`.
4. POST to `http://10.77.8.134:8080/stage/layout` with `{"code":"preach"}` — verify camera page DOM still has `data-layout-code="camera-crew"`.
5. POST to `http://10.77.8.134:8080/stage/broadcast-live` with `{"enabled":true}` — verify ON-AIR indicator gains the `is-on` class. Toggle off, verify class removed.
6. Read browser console — must have ZERO errors/warnings other than the well-known pre-existing favicon 404.

Record the version + the data-layout-code values for the completion report.

- [ ] **Step 5: Open PR**

```bash
gh pr create --title "feat(stage): /ui/camera layout for video director / camera crew (#311)" \
  --body "$(cat <<'EOF'
## Summary
- New always-on `/ui/camera` view focused on group transitions. Per service-team requirements: HUGE current group pill, BIG next group pill, small strip of 3 future distinct groups, compressed footer (song · library · timers · ON AIR · version + latency).
- Server dual-publishes a `camera-crew`-tagged stage snapshot alongside the operator-selected one (including when `api` layout is active). Camera page is pinned and ignores `LiveEvent::StageLayout` events.
- New snapshot field `upcoming_groups: Vec<UpcomingGroup>` computed during slide resolution; up to 4 distinct group names, consecutive duplicates collapsed.
- New `/stage/snapshot?layout=<code>` query so the camera page can prime independently.

Closes #311.

## Test plan
- [ ] Workspace `cargo test` green
- [ ] Playwright `wasm-stage-camera-crew` spec passes
- [ ] Dev deploy at v0.4.79 shows `/ui/camera` with `data-layout-code="camera-crew"`
- [ ] Operator switches `/stage/layout` → camera page UNCHANGED
- [ ] BroadcastLive toggle → ON-AIR indicator reacts on camera page
- [ ] Browser console clean

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 6: Verify mergeable**

```bash
gh pr view --json number,mergeable,mergeStateStatus
```

Both must be `mergeable: true` AND `mergeStateStatus: CLEAN`. If `UNSTABLE` / `BLOCKED`, investigate the failing check and fix.

- [ ] **Step 7: Send completion report**

Per `core/completion-report.md`:

- `✅ CI: green`
- `✅ /plan-check: 12/12 fulfilled` (after running plan-check)
- `✅ /review: clean — 0 🔴 0 🟡 0 🔵` (after running review)
- `✅ Deploy: dev frontend shows v0.4.79 ([data-testid="version"] read from DOM, matches /healthz)`
- Plan steps recap
- Goal + What changed (plain language)
- 🌐 Dev: http://10.77.8.134:8080/ui/camera
- 🌐 Prod: http://10.77.9.205/ui/camera (will be live after the user merges)
- PR link with full title

WAIT for explicit "merge it" before merging. Do NOT bypass `pr-merge-policy.md`.

---

## Self-review (controller-side, after the plan is written)

### Spec coverage

| Spec section | Plan task |
|---|---|
| New layout variant `camera-crew` in `built_in()` | Task 2 Step 6 |
| `UpcomingGroup` struct + snapshot field | Task 2 Steps 3-5 |
| Dual-publish in `publish_stage_context` | Task 4 Step 3 |
| Dual-publish covers `api`-layout case | Task 4 (publish moved BEFORE the api early-return) |
| `/stage/snapshot?layout=` query | Task 5 Step 2 |
| `/ui/camera` server route | Task 5 Step 3 |
| `pages/camera.rs` pinned + ignores StageLayout | Task 7 |
| `api/stage::get_snapshot_for` | Task 6 |
| `camera_crew.rs` component | Task 9 |
| CSS rules | Task 10 |
| Distinct-groups computation in `state/stage.rs` | Task 3 Steps 1-12 |
| Client-side group-color resolution via `fetch_group_colors` | Task 9 Step 1 (component body) |
| E2E Playwright test | Task 11 |
| Operator-layout-switch doesn't flip camera page | Task 11 Step 1 (first scenario) + Task 12 Step 4 |
| Browser console zero errors | Task 11 Step 1 (consoleErrors check) + Task 12 Step 4 |
| Version bump 0.4.78 → 0.4.79 | Task 1 |

### Placeholder scan

No "TBD" / "TODO" / "fill in details" entries. Every step has either explicit code or an explicit command with expected output. Some adapt-on-execution notes exist (e.g., `s.timers.preach.display` field path may need adjustment) and they cite EXACTLY where to look and what to copy.

### Type consistency

- `UpcomingGroup` defined in Task 2 (Step 3) → used in Tasks 3, 9 with the same `{ name: String }` shape.
- `StageDisplaySnapshot::new` signature change in Task 2 (Step 5) → all callers updated in Task 2 (Step 7).
- `upcoming_groups` field name used consistently across Task 2 (snapshot), Task 3 (resolution), Task 9 (component reads `s.upcoming_groups`).
- `camera-crew` literal layout code used consistently across server (`StageDisplayLayout::built_in`, `publish_stage_context`), client (`pages/camera.rs` const `CAMERA_LAYOUT`, route match), CSS, and Playwright.
- `get_snapshot_for(layout)` defined in Task 6 → used in `pages/camera.rs` Task 7.

### Scope check

Single feature, single PR. ~500 LoC estimate per spec. Bundling gate: 🔴 solo PR (over 300 LoC + cross-cuts core / server / ui). No follow-up issue needed.

---

**Plan complete and saved.** Next: execute via subagent-driven-development.
