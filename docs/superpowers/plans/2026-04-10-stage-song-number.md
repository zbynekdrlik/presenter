# Stage Song Number Display Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Display the ProPresenter catalog number (e.g., `#042`) in the stage status bar so musicians can identify the current song.

**Architecture:** Extract the 3-digit number prefix from `presentation_name` into a new `song_number` field on `StageDisplaySnapshot`, render it in the WASM `StatusBar` component between the clock and LIVE pill, and style it with a distinct blue color.

**Tech Stack:** Rust (presenter-core, presenter-server), Leptos WASM (presenter-ui), CSS, Playwright E2E

**Spec:** `docs/superpowers/specs/2026-04-10-stage-song-number-design.md`

---

## Context

Issue #225: Musicians on the stage display want to see the song's catalog number from the ProPresenter library. Songs are named like `"042 Amazing Grace"` — the existing `sanitize_song_title()` function already identifies and strips this prefix. We need to extract and display it separately.

**Key existing code:**
- `crates/presenter-core/src/stage_display.rs` — `StageDisplaySnapshot` struct (lines 80-112)
- `crates/presenter-server/src/state/stage.rs` — `sanitize_song_title()` (lines 232-246), `build_stage_snapshot()` (lines 203-230)
- `crates/presenter-ui/src/components/stage/status_bar.rs` — `StatusBar` component (lines 1-105)
- `crates/presenter-ui/styles/stage.css` — status bar CSS (lines 75-139)
- `tests/e2e/stage-status-bar.spec.ts` — existing E2E tests (lines 1-458)

---

## File Structure

### Modified Files
| File | Change |
|------|--------|
| `crates/presenter-core/src/stage_display.rs` | Add `song_number: Option<String>` field to `StageDisplaySnapshot` |
| `crates/presenter-server/src/state/stage.rs` | Add `extract_song_number()` function, populate field in `build_stage_snapshot()` |
| `crates/presenter-ui/src/components/stage/status_bar.rs` | Render `#NNN` element between clock and LIVE pill |
| `crates/presenter-ui/styles/stage.css` | Add `.stage__song-number` styles, adjust clock/live-pill widths |
| `tests/e2e/stage-status-bar.spec.ts` | Add E2E test for song number display |

---

## Task 1: Add `song_number` Field to StageDisplaySnapshot

**Files:**
- Modify: `crates/presenter-core/src/stage_display.rs:80-211`
- Modify: `crates/presenter-server/src/state/stage.rs:203-230, 232-246`

- [ ] **Step 1: Add `extract_song_number` function with unit tests**

In `crates/presenter-server/src/state/stage.rs`, add after the `sanitize_song_title` function (after line 246):

```rust
pub(crate) fn extract_song_number(name: &str) -> Option<String> {
    let trimmed = name.trim_start();
    let bytes = trimmed.as_bytes();
    if bytes.len() >= 4
        && bytes[0].is_ascii_digit()
        && bytes[1].is_ascii_digit()
        && bytes[2].is_ascii_digit()
        && bytes[3].is_ascii_whitespace()
    {
        Some(trimmed[..3].to_string())
    } else {
        None
    }
}
```

- [ ] **Step 2: Add unit tests for `extract_song_number`**

In `crates/presenter-server/src/state/stage.rs`, add a test module at the end of the file (or in the existing test location):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_song_number_returns_prefix_for_numbered_songs() {
        assert_eq!(extract_song_number("042 Amazing Grace"), Some("042".to_string()));
        assert_eq!(extract_song_number("001 First Song"), Some("001".to_string()));
        assert_eq!(extract_song_number("115 Last Song"), Some("115".to_string()));
    }

    #[test]
    fn extract_song_number_returns_none_for_unnumbered_songs() {
        assert_eq!(extract_song_number("Amazing Grace"), None);
        assert_eq!(extract_song_number(""), None);
        assert_eq!(extract_song_number("12 Two Digit"), None);
        assert_eq!(extract_song_number("1 One Digit"), None);
    }

    #[test]
    fn extract_song_number_handles_leading_whitespace() {
        assert_eq!(extract_song_number("  042 Song"), Some("042".to_string()));
    }

    #[test]
    fn sanitize_song_title_strips_number_prefix() {
        assert_eq!(sanitize_song_title("042 Amazing Grace"), "Amazing Grace");
        assert_eq!(sanitize_song_title("Song Without Number"), "Song Without Number");
    }
}
```

- [ ] **Step 3: Run tests to verify they pass**

```bash
cargo test -p presenter-server -- state::stage::tests --nocapture
```

Expected: All 4 tests pass.

- [ ] **Step 4: Add `song_number` field to `StageDisplaySnapshot`**

In `crates/presenter-core/src/stage_display.rs`, add after the `song_name` field (after line 92):

```rust
    #[serde(skip_serializing_if = "Option::is_none")]
    pub song_number: Option<String>,
```

Update `StageDisplaySnapshot::new()` (lines 171-210) to accept and set the new field. Add `song_number: Option<String>` parameter after `song_name`:

```rust
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        layout: StageDisplayLayout,
        generated_at: DateTime<Utc>,
        presentation_id: Option<PresentationId>,
        presentation_name: Option<String>,
        library_name: Option<String>,
        song_name: Option<String>,
        song_number: Option<String>,
        current_slide_id: Option<SlideId>,
        current: Option<StageDisplaySlide>,
        next_slide_id: Option<SlideId>,
        next: Option<StageDisplaySlide>,
        timers: crate::timer::TimersOverview,
        latency_ms: Option<f64>,
        current_position: Option<u32>,
        total_slides: Option<u32>,
        playlist_id: Option<PlaylistId>,
        playlist_name: Option<String>,
        playlist_entries: Option<Vec<StagePlaylistEntry>>,
    ) -> Self {
        Self {
            layout,
            generated_at,
            presentation_id,
            presentation_name,
            library_name,
            song_name,
            song_number,
            current_slide_id,
            current,
            next_slide_id,
            next,
            timers,
            latency_ms,
            current_position,
            total_slides,
            playlist_id,
            playlist_name,
            playlist_entries,
        }
    }
```

- [ ] **Step 5: Update `build_stage_snapshot` to populate `song_number`**

In `crates/presenter-server/src/state/stage.rs`, update `build_stage_snapshot()` (lines 203-230) to pass `song_number`:

```rust
pub(crate) fn build_stage_snapshot(
    layout: StageDisplayLayout,
    context: &StageContext,
) -> StageDisplaySnapshot {
    StageDisplaySnapshot::new(
        layout,
        context.generated_at,
        context.resolution.presentation_id,
        context.resolution.presentation_name.clone(),
        context.resolution.library_name.clone(),
        context
            .resolution
            .presentation_name
            .clone()
            .map(|name| sanitize_song_title(&name)),
        context
            .resolution
            .presentation_name
            .as_deref()
            .and_then(extract_song_number),
        context.resolution.current_slide_id,
        context.resolution.current.clone(),
        context.resolution.next_slide_id,
        context.resolution.next.clone(),
        context.overview.clone(),
        context.latency_ms,
        context.resolution.current_index,
        context.resolution.total_slides,
        context.resolution.playlist_id,
        context.resolution.playlist_name.clone(),
        context.resolution.playlist_entries.clone(),
    )
}
```

- [ ] **Step 6: Fix any compilation errors from callers of `StageDisplaySnapshot::new`**

Search for other callers of `StageDisplaySnapshot::new` across the workspace and add the `song_number` parameter (as `None` for test/mock callers):

```bash
cargo build -p presenter-core -p presenter-server 2>&1 | head -40
```

Fix each caller by adding `None` for `song_number` after `song_name` in the argument list. Common locations: snapshot tests, companion tests, router tests.

- [ ] **Step 7: Run full test suite**

```bash
cargo test -p presenter-core -p presenter-server --nocapture 2>&1 | tail -20
```

Expected: All tests pass.

- [ ] **Step 8: Commit**

```bash
cargo fmt --all
git add crates/presenter-core/src/stage_display.rs crates/presenter-server/src/state/stage.rs
git commit -m "feat(stage): add song_number field to StageDisplaySnapshot (#225)

Extract the 3-digit catalog number prefix from the ProPresenter
presentation name (e.g., '042' from '042 Amazing Grace') into a
new song_number field. The field is None when the name has no
number prefix."
```

---

## Task 2: Render Song Number in Stage Status Bar

**Files:**
- Modify: `crates/presenter-ui/src/components/stage/status_bar.rs:1-105`
- Modify: `crates/presenter-ui/styles/stage.css:75-139`

- [ ] **Step 1: Add song number element to StatusBar component**

In `crates/presenter-ui/src/components/stage/status_bar.rs`, add a new signal and node ref for the song number. After the `connection_ref` declaration (line 23), add:

```rust
    let song_number_ref = NodeRef::<leptos::html::Div>::new();
```

After the `broadcast_live` line (line 30), add the song number signal:

```rust
    let song_number = move || {
        ctx.snapshot
            .get()
            .and_then(|s| s.song_number)
            .map(|n| format!("#{n}"))
            .unwrap_or_default()
    };

    let has_song_number = move || {
        ctx.snapshot
            .get()
            .and_then(|s| s.song_number)
            .is_some()
    };
```

After the connection autofit (line 76), add:

```rust
    autofit_effect(song_number_ref, STATUS_MAX_FONT, song_number.clone());
```

In the `view!` macro, add the song number element between the clock and the LIVE pill (after the clock `div`, before the `(!hide_live)` block):

```rust
        {move || has_song_number().then(|| view! {
            <div node_ref=song_number_ref class="stage__song-number" data-role="song-number">
                <span class="stage__debug-label">"song-number"</span>
                {song_number.clone()}
            </div>
        })}
```

- [ ] **Step 2: Add CSS for song number in status bar**

In `crates/presenter-ui/styles/stage.css`, add after the `.stage__clock` block (after line 88):

```css
.stage__song-number {
    position: absolute;
    left: 18%;
    bottom: 0;
    width: 10%;
    height: 7%;
    color: #60a5fa;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    overflow: hidden;
    line-height: 0.95;
    text-align: center;
}
```

Adjust the clock width to make room — change `.stage__clock` `width` from `22%` to `15%`:

```css
.stage__clock {
    ...
    width: 15%;
    ...
}
```

Adjust `.stage__live-pill` `left` from `30%` to `29%` to tighten the gap:

```css
.stage__live-pill {
    ...
    left: 29%;
    ...
}
```

- [ ] **Step 3: Add song number to the debug label overflow list**

In `crates/presenter-ui/styles/stage.css`, find the combined selector that sets `overflow: hidden` on status bar elements (around line 344) and add `.stage__song-number`:

```css
.stage__current-slide,
.stage__next-group,
.stage__next-slide,
.stage__clock,
.stage__song-number,
.stage__live-pill,
.stage__connection,
```

- [ ] **Step 4: Build WASM to verify compilation**

```bash
cargo build -p presenter-ui --target wasm32-unknown-unknown 2>&1 | tail -10
```

Expected: Compiles without errors.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/presenter-ui/src/components/stage/status_bar.rs crates/presenter-ui/styles/stage.css
git commit -m "feat(stage): render song catalog number in status bar (#225)

Display '#042' between clock and LIVE pill when the active
presentation has a 3-digit number prefix. Hidden when no number
prefix exists. Uses blue color to distinguish from other status
bar elements."
```

---

## Task 3: E2E Test for Song Number Display

**Files:**
- Modify: `tests/e2e/stage-status-bar.spec.ts`

- [ ] **Step 1: Add E2E test for song number visibility**

In `tests/e2e/stage-status-bar.spec.ts`, add test data and helpers. After the `wsURL` variable declaration (line 15), add:

```typescript
let numberedPresentationId: string;
let numberedSlideIds: string[];
let unnumberedPresentationId: string;
let unnumberedSlideIds: string[];
```

At the end of the `beforeAll` block (before the closing `}`), add:

```typescript
  // Create test library for song number tests
  const libResp = await fetch(new URL("/libraries", baseURL).toString(), {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ name: "_E2E Song Number Test" }),
  });
  const lib = await libResp.json();

  // Create presentation WITH number prefix
  const numberedResp = await fetch(
    new URL(`/libraries/${lib.id}/presentations`, baseURL).toString(),
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        name: "042 Amazing Grace",
        slides: [{ main: "How sweet the sound", group: "Verse 1" }],
      }),
    },
  );
  const numberedData = await numberedResp.json();
  numberedPresentationId = numberedData.presentation.id;
  numberedSlideIds = numberedData.presentation.slides.map(
    (s: { id: string }) => s.id,
  );

  // Create presentation WITHOUT number prefix
  const unnumberedResp = await fetch(
    new URL(`/libraries/${lib.id}/presentations`, baseURL).toString(),
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        name: "Song Without Number",
        slides: [{ main: "Just a song", group: "Verse 1" }],
      }),
    },
  );
  const unnumberedData = await unnumberedResp.json();
  unnumberedPresentationId = unnumberedData.presentation.id;
  unnumberedSlideIds = unnumberedData.presentation.slides.map(
    (s: { id: string }) => s.id,
  );
```

- [ ] **Step 2: Add test for numbered song showing `#042`**

At the end of the test file (before the global type declarations), add:

```typescript
test("song number displays #042 for numbered presentation", async ({
  context,
}) => {
  const consoleMessages: string[] = [];

  const stagePage = await openStageDisplay(context);
  stagePage.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  // Trigger numbered presentation
  await context.request.post(new URL("/stage/state", baseURL).toString(), {
    data: {
      presentationId: numberedPresentationId,
      currentSlideId: numberedSlideIds[0],
      nextSlideId: null,
    },
  });

  // Wait for song number to appear
  const songNumberEl = stagePage.locator('[data-role="song-number"]');
  await expect(songNumberEl).toBeVisible({ timeout: 10_000 });
  await expect(songNumberEl).toContainText("#042");

  // Verify it's positioned between clock and LIVE pill
  const clockBox = await stagePage.locator(".stage__clock").boundingBox();
  const songBox = await songNumberEl.boundingBox();
  const liveBox = await stagePage.locator(".stage__live-pill").boundingBox();

  expect(clockBox).toBeTruthy();
  expect(songBox).toBeTruthy();
  expect(liveBox).toBeTruthy();

  if (clockBox && songBox && liveBox) {
    expect(clockBox.x + clockBox.width).toBeLessThanOrEqual(songBox.x + 1);
    expect(songBox.x + songBox.width).toBeLessThanOrEqual(liveBox.x + 1);
  }

  expect(consoleMessages).toEqual([]);

  await stagePage.close();
});
```

- [ ] **Step 3: Add test for unnumbered song hiding song number**

```typescript
test("song number is hidden for presentation without number prefix", async ({
  context,
}) => {
  const consoleMessages: string[] = [];

  const stagePage = await openStageDisplay(context);
  stagePage.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  // Trigger unnumbered presentation
  await context.request.post(new URL("/stage/state", baseURL).toString(), {
    data: {
      presentationId: unnumberedPresentationId,
      currentSlideId: unnumberedSlideIds[0],
      nextSlideId: null,
    },
  });

  // Wait for slide text to appear (confirms snapshot arrived)
  await expect(
    stagePage.locator(".stage__slide-text"),
  ).toContainText("Just a song", { timeout: 10_000 });

  // Song number element should NOT be visible
  const songNumberEl = stagePage.locator('[data-role="song-number"]');
  await expect(songNumberEl).not.toBeVisible();

  expect(consoleMessages).toEqual([]);

  await stagePage.close();
});
```

- [ ] **Step 4: Run E2E tests locally**

```bash
npm run test:playwright -- stage-status-bar
```

Expected: All existing tests pass plus 2 new tests.

- [ ] **Step 5: Commit**

```bash
git add tests/e2e/stage-status-bar.spec.ts
git commit -m "test(e2e): add stage song number display tests (#225)

Verify #042 appears in status bar for numbered presentations,
and is hidden for presentations without a number prefix."
```

---

## Task 4: Version Bump, Format Check, Push, Monitor CI

- [ ] **Step 1: Check and bump version**

```bash
git fetch origin
grep '^version' Cargo.toml | head -1
```

Compare dev vs main version. If dev version ≤ main, bump patch in `Cargo.toml` workspace `[workspace.package].version`.

- [ ] **Step 2: Commit version bump**

```bash
cargo fmt --all
git add Cargo.toml Cargo.lock
git commit -m "chore: bump version to X.Y.Z"
```

- [ ] **Step 3: Run local checks**

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all
cargo test -p presenter-core -p presenter-server --nocapture 2>&1 | tail -20
```

Fix any issues in ONE commit if needed.

- [ ] **Step 4: Push and monitor CI**

```bash
git push origin dev
gh run list --branch dev --limit 3
```

Monitor until ALL jobs complete. If any fail, `gh run view <run-id> --log-failed`, fix ALL issues in ONE commit, push ONCE, monitor again.

---

## Verification Summary

| Check | How to verify |
|-------|---------------|
| Song number extracted | Unit tests for `extract_song_number()` with numbered and unnumbered names |
| Field serialized | `StageDisplaySnapshot` JSON includes `songNumber` when present |
| Rendered on stage | E2E: trigger numbered presentation → `#042` visible in status bar |
| Hidden when absent | E2E: trigger unnumbered presentation → no song number element |
| Positioned correctly | E2E: song number is between clock and LIVE pill (bounding box check) |
| Clean console | E2E: zero browser console errors/warnings |
| No regressions | All existing stage-status-bar tests still pass |
