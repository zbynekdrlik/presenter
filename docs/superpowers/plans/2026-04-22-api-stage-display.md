# API Stage Display Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `PUT /api/stage` endpoint that lets an external app push slide data to a new "API" stage layout, rendered using the same worship-snv component with group color pills.

**Architecture:** A single REST endpoint writes `ApiStageState` to in-memory storage, resolves group colors, converts to `StageDisplaySnapshot` with layout code "api", and broadcasts via the existing `LiveHub`. Client-side filtering in the WASM stage WS handler ensures API snapshots only update "api" layout clients and normal snapshots only update normal layout clients.

**Tech Stack:** Rust (axum, serde, tokio), Leptos WASM, Playwright E2E

**Spec:** `docs/superpowers/specs/2026-04-22-api-stage-display-design.md`

---

## Context

The external custom app pushes current/next slide text, song names, and group names to Presenter's stage display. The "api" layout reuses the worship-snv component visually — no new WASM component. The API stage state is independent from Presenter's internal slide system.

**Key existing code:**
- `crates/presenter-core/src/stage_display.rs` — `StageDisplayLayout::built_in()`, `StageDisplaySlide`, `StageDisplaySnapshot`
- `crates/presenter-server/src/state/mod.rs` — `AppState`, `resolve_group_color()`
- `crates/presenter-server/src/state/broadcasting.rs` — `publish_stage_update()`, `enrich_stage_context()`
- `crates/presenter-server/src/state/stage_display.rs` — `stage_display_snapshot()`, `selected_stage_display_snapshot()`
- `crates/presenter-server/src/state/stage.rs` — `StageResolution`, `build_stage_snapshot()`
- `crates/presenter-server/src/router.rs` — route registration
- `crates/presenter-server/src/router/stage.rs` — existing stage REST handlers
- `crates/presenter-ui/src/pages/stage.rs` — stage page with layout matching and WS event handling (lines 52-53)
- `crates/presenter-ui/src/ws/stage.rs` — WebSocket handler
- `crates/presenter-core/src/timer.rs` — `TimersOverview`, `CountdownTimerSnapshot`, `PreachTimerSnapshot`

---

## File Structure

### Modified Files
| File | Change |
|------|--------|
| `crates/presenter-core/src/stage_display.rs` | Add "api" to `BUILT_IN_LAYOUTS`, update test |
| `crates/presenter-server/src/state/mod.rs` | Add `api_stage` field, `ApiStageState` struct, methods |
| `crates/presenter-server/src/state/stage_display.rs` | Return API snapshot when layout is "api" |
| `crates/presenter-server/src/router.rs` | Register `PUT /api/stage` route, add `mod api_stage` |
| `crates/presenter-ui/src/pages/stage.rs` | Filter `LiveEvent::Stage` by layout code |

### New Files
| File | Purpose |
|------|---------|
| `crates/presenter-server/src/router/api_stage.rs` | `PUT /api/stage` handler |
| `tests/e2e/api-stage.spec.ts` | E2E test for API stage push and display |

---

## Task 1: Register "api" Layout

**Files:**
- Modify: `crates/presenter-core/src/stage_display.rs:22-51` (built_in layouts)
- Modify: `crates/presenter-core/src/stage_display.rs:230-241` (test)

- [ ] **Step 1: Add "api" to built_in layouts**

In `crates/presenter-core/src/stage_display.rs`, add the "api" layout after the "bible" entry in `built_in()` (line 49):

```rust
            Self::new("bible", "BIBLE", "Full-screen Bible passage display"),
            Self::new("api", "API", "External API-driven stage display"),
```

- [ ] **Step 2: Update the built_in layouts test**

In `crates/presenter-core/src/stage_display.rs`, replace the test (lines 230-241):

```rust
    #[test]
    fn built_in_layouts_cover_expected_variants() {
        let layouts = StageDisplayLayout::built_in();
        assert_eq!(layouts.len(), 7);
        let codes: Vec<_> = layouts.iter().map(|layout| layout.code.as_str()).collect();
        assert!(codes.contains(&DEFAULT_STAGE_LAYOUT_CODE));
        assert!(codes.contains(&"worship-pp"));
        assert!(codes.contains(&"timer"));
        assert!(codes.contains(&"preach"));
        assert!(codes.contains(&"ndi-fullscreen"));
        assert!(codes.contains(&"bible"));
        assert!(codes.contains(&"api"));
    }
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p presenter-core -- stage_display --nocapture
```

Expected: 1 test passes with the new layout count (7).

- [ ] **Step 4: Commit**

```bash
cargo fmt --all
git add crates/presenter-core/src/stage_display.rs
git commit -m "feat(stage): register API layout in built-in stage displays (#XXX)"
```

---

## Task 2: Add ApiStageState and AppState Field

**Files:**
- Modify: `crates/presenter-server/src/state/mod.rs:79-106` (AppState struct)
- Modify: `crates/presenter-server/src/state/mod.rs:148-188` (constructors)

- [ ] **Step 1: Define ApiStageState struct and add field to AppState**

In `crates/presenter-server/src/state/mod.rs`, add the struct before the `AppState` struct definition (before line 78):

```rust
/// External API-driven stage state. All fields default to empty strings.
/// Missing or null JSON fields deserialize to "".
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApiStageState {
    #[serde(default)]
    pub(crate) current_text: String,
    #[serde(default)]
    pub(crate) next_text: String,
    #[serde(default)]
    pub(crate) current_group: String,
    #[serde(default)]
    pub(crate) next_group: String,
    #[serde(default)]
    pub(crate) current_song: String,
    #[serde(default)]
    pub(crate) next_song: String,
}
```

Add the field to the `AppState` struct (after `group_color_cache` at line 102):

```rust
    api_stage: Arc<RwLock<ApiStageState>>,
```

- [ ] **Step 2: Initialize the field in constructors**

In `new_with_heartbeat()` (around line 162), add `api_stage` to the struct literal after `group_color_cache`:

```rust
            group_color_cache: Arc::new(RwLock::new(HashMap::new())),
            api_stage: Arc::new(RwLock::new(ApiStageState::default())),
```

- [ ] **Step 3: Add methods to build API snapshot**

In `crates/presenter-server/src/state/mod.rs`, add these methods to the `impl AppState` block (after the `resolve_group_color` method, around line 720):

```rust
    pub(crate) async fn update_api_stage(&self, state: ApiStageState) -> anyhow::Result<()> {
        let snapshot = self.build_api_stage_snapshot(&state).await;
        *self.api_stage.write().await = state;
        self.live_hub.publish(LiveEvent::Stage { snapshot });
        Ok(())
    }

    pub(crate) async fn api_stage_snapshot(&self) -> StageDisplaySnapshot {
        let state = self.api_stage.read().await;
        self.build_api_stage_snapshot(&state).await
    }

    async fn build_api_stage_snapshot(&self, state: &ApiStageState) -> StageDisplaySnapshot {
        let layout = StageDisplayLayout::built_in()
            .into_iter()
            .find(|l| l.code == "api")
            .expect("api layout must exist in built_in");

        let current = if state.current_text.is_empty() && state.current_group.is_empty() {
            None
        } else {
            let group = if state.current_group.is_empty() {
                None
            } else {
                Some(state.current_group.clone())
            };
            let group_color = if let Some(ref name) = group {
                self.resolve_group_color(name).await
            } else {
                None
            };
            Some(StageDisplaySlide {
                main: state.current_text.clone(),
                translation: String::new(),
                stage: String::new(),
                group,
                group_color,
            })
        };

        let next = if state.next_text.is_empty() && state.next_group.is_empty() {
            None
        } else {
            let group = if state.next_group.is_empty() {
                None
            } else {
                Some(state.next_group.clone())
            };
            let group_color = if let Some(ref name) = group {
                self.resolve_group_color(name).await
            } else {
                None
            };
            Some(StageDisplaySlide {
                main: state.next_text.clone(),
                translation: String::new(),
                stage: String::new(),
                group,
                group_color,
            })
        };

        let song_name = if state.current_song.is_empty() {
            None
        } else {
            Some(state.current_song.clone())
        };
        let next_song_name = if state.next_song.is_empty() {
            None
        } else {
            Some(state.next_song.clone())
        };

        let now = Utc::now();
        let timers = self
            .load_or_init_timers(now)
            .await
            .map(|t| t.overview(now))
            .unwrap_or_else(|_| TimersOverview::demo(now));

        StageDisplaySnapshot::new(
            layout,
            now,
            None,                // presentation_id
            None,                // presentation_name
            None,                // library_name
            song_name,           // song_name
            None,                // song_number
            next_song_name,      // next_song_name
            None,                // current_slide_id
            current,             // current
            None,                // next_slide_id
            next,                // next
            timers,              // timers
            None,                // latency_ms
            None,                // current_position
            None,                // total_slides
            None,                // playlist_id
            None,                // playlist_name
            None,                // playlist_entries
        )
    }
```

- [ ] **Step 4: Add imports**

Ensure these imports are present at the top of `crates/presenter-server/src/state/mod.rs`:

```rust
use presenter_core::{
    // ... existing imports ...
    StageDisplaySlide,
};
```

The `StageDisplaySlide` import may need to be added to the existing `use presenter_core::{...}` block (around line 50-53).

- [ ] **Step 5: Run tests**

```bash
cargo test -p presenter-server --lib -- --nocapture
```

Expected: All existing tests pass (the new field has a default, so existing test constructors like `in_memory()` are already covered).

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add crates/presenter-server/src/state/mod.rs
git commit -m "feat(stage): add ApiStageState and in-memory storage (#XXX)"
```

---

## Task 3: Add Client-Side Layout Filtering

**Files:**
- Modify: `crates/presenter-ui/src/pages/stage.rs:51-54`

- [ ] **Step 1: Filter LiveEvent::Stage by layout code**

In `crates/presenter-ui/src/pages/stage.rs`, replace the `LiveEvent::Stage` handler (lines 52-54):

```rust
                LiveEvent::Stage { snapshot } => {
                    // Only accept snapshots matching our layout to keep
                    // API stage and normal stage independent.
                    if snapshot.layout.code == ctx.layout_code.get_untracked() {
                        ctx.snapshot.set(Some(snapshot));
                    }
                }
```

- [ ] **Step 2: Run WASM build to verify compilation**

```bash
cargo build -p presenter-ui --target wasm32-unknown-unknown
```

Expected: Compiles without errors.

- [ ] **Step 3: Commit**

```bash
cargo fmt --all
git add crates/presenter-ui/src/pages/stage.rs
git commit -m "feat(stage): filter stage snapshots by layout code (#XXX)

Prevents API stage pushes from overwriting normal stage clients
and vice versa. Each stage client only accepts snapshots whose
layout code matches its own."
```

---

## Task 4: Add PUT /api/stage Endpoint

**Files:**
- Create: `crates/presenter-server/src/router/api_stage.rs`
- Modify: `crates/presenter-server/src/router.rs:1-14` (add module)
- Modify: `crates/presenter-server/src/router.rs:155-156` (add route)

- [ ] **Step 1: Create the handler file**

Create `crates/presenter-server/src/router/api_stage.rs`:

```rust
use axum::{extract::State, http::StatusCode, Json};
use tracing::instrument;

use super::AppError;
use crate::state::{ApiStageState, AppState};

#[instrument(skip_all)]
pub(super) async fn update_api_stage(
    State(state): State<AppState>,
    Json(payload): Json<ApiStageState>,
) -> Result<StatusCode, AppError> {
    state
        .update_api_stage(payload)
        .await
        .map_err(AppError::internal)?;
    Ok(StatusCode::NO_CONTENT)
}
```

- [ ] **Step 2: Register the module and route**

In `crates/presenter-server/src/router.rs`, add the module declaration after the existing ones (around line 1):

```rust
mod api_stage;
```

Add the route after the `/stage/broadcast-live` route (after line 156):

```rust
        .route("/api/stage", put(api_stage::update_api_stage))
```

- [ ] **Step 3: Make ApiStageState public to the router**

In `crates/presenter-server/src/state/mod.rs`, change the visibility of `ApiStageState` from `pub(crate)` to `pub(crate)` — it should already be `pub(crate)` from Task 2. Verify that the router module can access it via `crate::state::ApiStageState`.

- [ ] **Step 4: Run tests**

```bash
cargo test -p presenter-server --lib -- --nocapture
```

Expected: All tests pass.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/presenter-server/src/router/api_stage.rs crates/presenter-server/src/router.rs
git commit -m "feat(stage): add PUT /api/stage endpoint (#XXX)

Accepts full state JSON with currentText, nextText, currentGroup,
nextGroup, currentSong, nextSong. Missing fields default to empty.
Returns 204 No Content."
```

---

## Task 5: Handle Initial Snapshot for API Layout

**Files:**
- Modify: `crates/presenter-server/src/state/stage_display.rs:27-35`

- [ ] **Step 1: Return API snapshot when layout is "api"**

In `crates/presenter-server/src/state/stage_display.rs`, replace `selected_stage_display_snapshot` (lines 27-35):

```rust
    pub async fn selected_stage_display_snapshot(
        &self,
    ) -> anyhow::Result<Option<StageDisplaySnapshot>> {
        let code = {
            let guard = self.stage_layout.read().await;
            guard.clone()
        };
        if code == "api" {
            return Ok(Some(self.api_stage_snapshot().await));
        }
        self.stage_display_snapshot(&code).await
    }
```

Also update `stage_display_snapshot` (lines 10-25) to handle "api":

```rust
    pub async fn stage_display_snapshot(
        &self,
        layout_code: &str,
    ) -> anyhow::Result<Option<StageDisplaySnapshot>> {
        if layout_code == "api" {
            return Ok(Some(self.api_stage_snapshot().await));
        }
        let layout = StageDisplayLayout::built_in()
            .into_iter()
            .find(|layout| layout.code == layout_code);
        let Some(layout) = layout else {
            return Ok(None);
        };
        let Some(context) = self.build_stage_context().await? else {
            return Ok(None);
        };
        let context = self.enrich_stage_context(&context).await;
        Ok(Some(build_stage_snapshot(layout, &context)))
    }
```

- [ ] **Step 2: Run tests**

```bash
cargo test -p presenter-server --lib -- --nocapture
```

Expected: All tests pass.

- [ ] **Step 3: Commit**

```bash
cargo fmt --all
git add crates/presenter-server/src/state/stage_display.rs
git commit -m "feat(stage): return API snapshot for initial stage page load (#XXX)

When the selected layout is 'api', stage_display_snapshot returns
the in-memory API state instead of Presenter's internal slide data."
```

---

## Task 6: E2E Playwright Test

**Files:**
- Create: `tests/e2e/api-stage.spec.ts`

- [ ] **Step 1: Write the E2E test**

Create `tests/e2e/api-stage.spec.ts`:

```typescript
import { test, expect, BrowserContext } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

test.describe.configure({ timeout: 180_000 });

let server: ServerHandle | undefined;
let baseURL = "";

test.beforeAll(async ({}, testInfo) => {
  const cfg = deriveTestConfig(testInfo);
  baseURL = cfg.baseURL;
  await refreshDevData(cfg.dbUrl);
  server = await startTestServer(cfg.port, cfg.dbUrl, cfg.oscPort);
});

test.afterAll(async () => {
  await stopServer(server);
  server = undefined;
});

async function openApiStage(context: BrowserContext) {
  // Set global layout to "api"
  await context.request.post(new URL("/stage/layout", baseURL).toString(), {
    data: { code: "api" },
  });
  const stagePage = await context.newPage();
  await stagePage.goto(new URL("/stage", baseURL).toString(), {
    waitUntil: "domcontentloaded",
  });
  await stagePage.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await stagePage.waitForFunction(
    () => window.__presenterStageConnectionState === "connected",
    { timeout: 30_000 },
  );
  return stagePage;
}

test("API stage push displays text and group colors", async ({
  context,
  request,
}) => {
  const consoleMessages: string[] = [];

  const stagePage = await openApiStage(context);
  stagePage.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  // Push data via API
  const putResp = await request.put(
    new URL("/api/stage", baseURL).toString(),
    {
      data: {
        currentText: "Haleluja, haleluja",
        nextText: "Spievajte Hospodinovi",
        currentGroup: "Vsetci",
        nextGroup: "Zeny",
        currentSong: "Haleluja",
        nextSong: "Spievajte",
      },
    },
  );
  expect(putResp.status()).toBe(204);

  // Verify current text
  const currentText = stagePage.locator(".stage__current-text");
  await expect(currentText).toContainText("Haleluja, haleluja", {
    timeout: 10_000,
  });

  // Verify next text
  const nextText = stagePage.locator(".stage__next-text");
  await expect(nextText).toContainText("Spievajte Hospodinovi", {
    timeout: 10_000,
  });

  // Verify current group pill with legacy color for "Vsetci" (#E08A3C = rgb(224, 138, 60))
  const currentGroupPill = stagePage.locator(
    ".stage__current-group .stage__group-pill",
  );
  await expect(currentGroupPill).toBeVisible({ timeout: 10_000 });
  await expect(currentGroupPill).toContainText("Vsetci");
  const bgColor = await currentGroupPill.evaluate(
    (el) => window.getComputedStyle(el).backgroundColor,
  );
  expect(bgColor).toBe("rgb(224, 138, 60)");

  // Verify text color is black (WCAG contrast for light background)
  const textColor = await currentGroupPill.evaluate(
    (el) => window.getComputedStyle(el).color,
  );
  expect(textColor).toBe("rgb(0, 0, 0)");

  // Verify next group pill
  const nextGroupPill = stagePage.locator(
    ".stage__next-group .stage__group-pill",
  );
  await expect(nextGroupPill).toBeVisible({ timeout: 10_000 });
  await expect(nextGroupPill).toContainText("Zeny");

  // Verify current song name
  const songName = stagePage.locator(".stage__current-song");
  await expect(songName).toContainText("Haleluja", { timeout: 10_000 });

  // Verify next song name
  const nextSongName = stagePage.locator(".stage__next-song");
  await expect(nextSongName).toContainText("Spievajte", { timeout: 10_000 });

  await stagePage.close();
  expect(consoleMessages).toEqual([]);
});

test("API stage push with empty state clears display", async ({
  context,
  request,
}) => {
  const consoleMessages: string[] = [];

  const stagePage = await openApiStage(context);
  stagePage.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  // Push data first
  await request.put(new URL("/api/stage", baseURL).toString(), {
    data: {
      currentText: "Some text",
      currentGroup: "Vsetci",
      currentSong: "Song",
    },
  });

  const currentText = stagePage.locator(".stage__current-text");
  await expect(currentText).toContainText("Some text", { timeout: 10_000 });

  // Push empty state
  const putResp = await request.put(
    new URL("/api/stage", baseURL).toString(),
    { data: {} },
  );
  expect(putResp.status()).toBe(204);

  // Wait for the display to clear — current text should become empty
  await expect(currentText).toHaveText("", { timeout: 10_000 });

  // Group pill should not be visible
  const currentGroupPill = stagePage.locator(
    ".stage__current-group .stage__group-pill",
  );
  await expect(currentGroupPill).not.toBeVisible({ timeout: 5_000 });

  await stagePage.close();
  expect(consoleMessages).toEqual([]);
});

test("API stage does not interfere with normal stage", async ({
  context,
  request,
}) => {
  const consoleMessages: string[] = [];

  // Create a library and presentation for normal stage
  const libResp = await request.post(
    new URL("/libraries", baseURL).toString(),
    { data: { name: `ApiIsolation Lib ${Date.now()}` } },
  );
  expect(libResp.ok()).toBeTruthy();
  const library: { id: string } = await libResp.json();

  const presResp = await request.post(
    new URL(`/libraries/${library.id}/presentations`, baseURL).toString(),
    { data: { name: "Normal Song" } },
  );
  expect(presResp.ok()).toBeTruthy();
  const presPayload: {
    presentation: { id: string; slides: Array<{ id: string }> };
  } = await presResp.json();
  const presentationId = presPayload.presentation.id;
  const slideId = presPayload.presentation.slides[0].id;

  // Set slide text
  await request.patch(
    new URL(
      `/presentations/${presentationId}/slides/${slideId}`,
      baseURL,
    ).toString(),
    {
      data: { main: "Normal slide text", translation: "", stage: "" },
    },
  );

  // Set normal stage layout and trigger slide
  await request.post(new URL("/stage/layout", baseURL).toString(), {
    data: { code: "worship-snv" },
  });
  await request.post(new URL("/stage/state", baseURL).toString(), {
    data: { presentationId, currentSlideId: slideId },
  });

  // Open normal stage page
  const normalPage = await context.newPage();
  normalPage.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });
  await normalPage.goto(new URL("/stage", baseURL).toString(), {
    waitUntil: "domcontentloaded",
  });
  await normalPage.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await normalPage.waitForFunction(
    () => window.__presenterStageConnectionState === "connected",
    { timeout: 30_000 },
  );

  // Verify normal stage shows normal text
  const normalText = normalPage.locator(".stage__current-text");
  await expect(normalText).toContainText("Normal slide text", {
    timeout: 10_000,
  });

  // Push API stage data — should NOT affect the normal stage
  await request.put(new URL("/api/stage", baseURL).toString(), {
    data: { currentText: "API override attempt" },
  });

  // Wait briefly and verify normal stage still shows normal text
  await normalPage.waitForTimeout(2_000);
  await expect(normalText).toContainText("Normal slide text");

  await normalPage.close();
  expect(consoleMessages).toEqual([]);
});
```

- [ ] **Step 2: Run E2E test locally**

```bash
npm run test:playwright -- api-stage
```

Expected: All 3 tests pass.

- [ ] **Step 3: Commit**

```bash
git add tests/e2e/api-stage.spec.ts
git commit -m "test(e2e): add API stage display E2E tests (#XXX)

Three tests: push with text and group colors, empty state clears
display, API push does not interfere with normal stage."
```

---

## Task 7: Version Bump, Local Checks, Push, Monitor CI

- [ ] **Step 1: Check and bump version**

```bash
git fetch origin
grep '^version' Cargo.toml | head -1
```

Compare with main. Bump the patch version in `Cargo.toml` workspace `[workspace.package].version` if needed.

- [ ] **Step 2: Commit version bump**

```bash
git add Cargo.toml Cargo.lock
git commit -m "chore: bump version to X.Y.Z"
```

- [ ] **Step 3: Run local checks**

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all
cargo test -p presenter-core -- stage_display --nocapture
cargo test -p presenter-server --lib -- --nocapture
```

Fix any issues in ONE commit if needed.

- [ ] **Step 4: Push and monitor CI**

```bash
git push origin dev
```

Monitor with `gh run list --branch dev --limit 3` and `gh run view <run-id>` until all jobs complete. If any fail, `gh run view <run-id> --log-failed`, fix ALL issues in ONE commit, push again.

---

## Verification Summary

| Check | How to verify |
|-------|---------------|
| Layout registered | `GET /stage-displays` returns 7 layouts including "api" |
| PUT endpoint works | `PUT /api/stage` with JSON body returns 204 |
| Empty fields default | `PUT /api/stage` with `{}` returns 204, stage shows blank |
| Group colors resolve | Push with `currentGroup: "Vsetci"` → stage shows orange pill with black text |
| Normal stage unaffected | API push while layout is worship-snv → normal text unchanged |
| Initial load works | Select "api" layout → open /stage → shows last API-pushed data |
| No regressions | All existing stage E2E tests and unit tests still pass |
| Clean console | No browser console errors or warnings |
