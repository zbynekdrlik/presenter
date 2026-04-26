# Stage Display Song Name Boxes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add "Current Song" and "Next Song" amber-colored boxes to the worship-snv stage display, with next-song data sourced from AbleSet setlist or Presenter playlist.

**Architecture:** Extend `StageDisplaySnapshot` with a `next_song_name` field. Cache the full AbleSet setlist in the bridge to derive the next song. In the frontend, add two new Leptos elements to `worship_snv.rs` with auto-fit text and amber CSS styling, repositioning group pills from centered to left-aligned.

**Tech Stack:** Rust (presenter-core, presenter-server), Leptos WASM (presenter-ui), CSS, Playwright E2E

**Spec:** `docs/superpowers/specs/2026-04-19-stage-song-name-boxes-design.md`

---

## Context

The worship-snv stage display currently shows: current-group pill (centered), current-slide lyrics, next-group pill (centered), next-slide lyrics, and a status bar with clock/song-number/live-pill/connection. The worship band wants to see the current and next song names on screen.

**Key existing code:**
- `crates/presenter-core/src/stage_display.rs` — `StageDisplaySnapshot` struct (18-field constructor), `StageDisplayLayout`
- `crates/presenter-server/src/state/stage.rs` — `build_stage_snapshot()`, `StageContext`, `StageResolution`, `sanitize_song_title()`
- `crates/presenter-server/src/state/broadcasting.rs` — `broadcast_stage_resolution()`, `publish_stage_context()`
- `crates/presenter-server/src/ableset.rs` — `AbleSetBridge`, `run_tracker()`, `fetch_active_song()`, `AbleSetStatusInner`
- `crates/presenter-ui/src/components/stage/worship_snv.rs` — Leptos component for worship-snv layout
- `crates/presenter-ui/styles/stage.css` — stage CSS with absolute positioning

---

## File Structure

### Modified Files
| File | Change |
|------|--------|
| `crates/presenter-core/src/stage_display.rs` | Add `next_song_name: Option<String>` to `StageDisplaySnapshot` struct and constructor |
| `crates/presenter-server/src/ableset.rs` | Cache full setlist in `AbleSetStatusInner`, add `next_song_name()` method |
| `crates/presenter-server/src/state/stage.rs` | Pass `next_song_name` into `build_stage_snapshot()` |
| `crates/presenter-server/src/state/broadcasting.rs` | Resolve `next_song_name` from AbleSet or playlist |
| `crates/presenter-ui/src/components/stage/worship_snv.rs` | Add current-song and next-song boxes to component |
| `crates/presenter-ui/styles/stage.css` | Add `.stage__current-song`, `.stage__next-song` CSS; reposition group pills |

### New Files
| File | Purpose |
|------|---------|
| `tests/e2e/stage-song-names.spec.ts` | E2E test for song name boxes on stage display |

---

## Task 1: Add `next_song_name` to `StageDisplaySnapshot`

**Files:**
- Modify: `crates/presenter-core/src/stage_display.rs:81-215`
- Modify: `crates/presenter-server/src/state/stage.rs:203-234`

- [ ] **Step 1: Add the field to the struct**

In `crates/presenter-core/src/stage_display.rs`, add `next_song_name` after `song_number` (line 95):

```rust
    #[serde(skip_serializing_if = "Option::is_none")]
    pub song_number: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_song_name: Option<String>,
```

- [ ] **Step 2: Update the constructor**

In `crates/presenter-core/src/stage_display.rs`, update `StageDisplaySnapshot::new()` (line 175). Add `next_song_name: Option<String>` parameter after `song_number` and include it in the struct initialization:

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
        next_song_name: Option<String>,
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
            next_song_name,
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

- [ ] **Step 3: Update `build_stage_snapshot` caller**

In `crates/presenter-server/src/state/stage.rs`, update `build_stage_snapshot()` (line 203-234) to pass `None` for `next_song_name` initially (will be wired up in Task 3):

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
        context.resolution.next_song_name.clone(),
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

- [ ] **Step 4: Add `next_song_name` to `StageResolution`**

In `crates/presenter-server/src/state/stage.rs`, add the field to `StageResolution` struct (line 17-37):

```rust
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) next_song_name: Option<String>,
```

Update `StageResolution::cleared()` to include `next_song_name: None`.

Update `stage_resolution_from_presentation()` (line 80-137) to include `next_song_name: None` in both return paths.

- [ ] **Step 5: Verify it compiles**

```bash
cargo check -p presenter-core -p presenter-server
```

Expected: compiles with no errors.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add crates/presenter-core/src/stage_display.rs crates/presenter-server/src/state/stage.rs
git commit -m "feat(stage): add next_song_name field to StageDisplaySnapshot

Add next_song_name: Option<String> to the snapshot struct and
StageResolution. Wired as None for now — will be populated from
AbleSet or playlist in the next task."
```

---

## Task 2: Cache AbleSet Setlist and Add `next_song_name()` Method

**Files:**
- Modify: `crates/presenter-server/src/ableset.rs:36-59, 174-184, 362-472`

- [ ] **Step 1: Add setlist cache to `AbleSetStatusInner`**

In `crates/presenter-server/src/ableset.rs`, add a `setlist_songs` field to `AbleSetStatusInner` (line 41-52):

```rust
struct AbleSetStatusInner {
    enabled: bool,
    host: String,
    http_port: u16,
    osc_port: u16,
    library_name: String,
    song_prefix_length: u8,
    tracking: bool,
    last_song: Option<SongState>,
    setlist_songs: Vec<SetlistCachedSong>,
    last_error: Option<String>,
    follow_enabled: bool,
}
```

Add the cached song struct after `SongState` (line 59):

```rust
#[derive(Clone)]
struct SetlistCachedSong {
    id: String,
    name: String,
}
```

Update the `AbleSetBridge::new()` constructor (line 118-136) to initialize `setlist_songs: Vec::new()`.

- [ ] **Step 2: Update `run_tracker` to cache the full song list**

In `crates/presenter-server/src/ableset.rs`, modify `run_tracker()` (line 362-419) to call a new `fetch_setlist` function instead of `fetch_active_song`, and cache the full song list:

Replace the `fetch_active_song` call and its match block (lines 382-412) with:

```rust
                match fetch_setlist(&client, &host, http_port).await {
                    Ok(Some(setlist)) => {
                        let mut status = inner.status.write().await;
                        status.setlist_songs = setlist.songs.iter().map(|s| {
                            let name = s.meta.as_ref()
                                .and_then(|m| m.name.as_ref().cloned().or_else(|| m.raw.clone()))
                                .or_else(|| s.cue.as_ref().and_then(|c| c.name.clone()))
                                .unwrap_or_default();
                            SetlistCachedSong {
                                id: s.id.clone().unwrap_or_default(),
                                name,
                            }
                        }).collect();

                        if let Some(active_id) = &setlist.active_song_id {
                            for (idx, song) in setlist.songs.iter().enumerate() {
                                if song.id.as_deref() == Some(active_id.as_str()) {
                                    let name = status.setlist_songs[idx].name.clone();
                                    if let Some(prefix) = extract_song_prefix(&name, song_prefix_length) {
                                        let index = song.internal_meta
                                            .as_ref()
                                            .and_then(|m| m.order)
                                            .or(Some(idx as u32));
                                        status.last_song = Some(SongState {
                                            name,
                                            prefix,
                                            index,
                                            last_seen_at: Utc::now(),
                                        });
                                        status.last_error = None;
                                    } else {
                                        status.last_error = Some(format!(
                                            "unable to extract prefix of length {} from song '{name}'",
                                            song_prefix_length
                                        ));
                                    }
                                    break;
                                }
                            }
                        } else {
                            status.last_song = None;
                            status.last_error = None;
                        }
                    }
                    Ok(None) => {
                        let mut status = inner.status.write().await;
                        status.last_song = None;
                        status.setlist_songs.clear();
                        status.last_error = None;
                    }
                    Err(err) => {
                        let mut status = inner.status.write().await;
                        status.last_error = Some(err.to_string());
                        debug!(?err, "ableset fetch failed");
                    }
                }
```

- [ ] **Step 3: Rename `fetch_active_song` to `fetch_setlist`**

Replace the `fetch_active_song` function (line 421-472) with `fetch_setlist` that returns the full `SetlistResponse`:

```rust
async fn fetch_setlist(
    client: &Client,
    host: &str,
    http_port: u16,
) -> anyhow::Result<Option<SetlistResponse>> {
    let url = format!("http://{host}:{http_port}{SETLIST_ENDPOINT}");
    let response = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("failed to query AbleSet at {url}"))?;

    if response.status().is_success() {
        let payload: SetlistResponse = response
            .json()
            .await
            .context("failed to parse AbleSet setlist payload")?;
        return Ok(Some(payload));
    }

    if response.status().as_u16() == 404 {
        return Ok(None);
    }

    Err(anyhow!(
        "AbleSet responded with status {}",
        response.status()
    ))
}
```

- [ ] **Step 4: Add `next_song_name()` method to `AbleSetBridge`**

Add after the `song_snapshot()` method (line 184):

```rust
    pub async fn next_song_name(&self) -> Option<String> {
        let status = self.inner.status.read().await;
        let last_song = status.last_song.as_ref()?;
        let active_idx = status.setlist_songs.iter().position(|s| s.id == last_song.name || {
            // Match by prefix since last_song.name is the display name
            // Find by matching the cached song name
            s.name == last_song.name
        })?;
        let next = status.setlist_songs.get(active_idx + 1)?;
        Some(sanitize_song_title(&next.name))
    }
```

Wait — the matching needs to be on song ID from the active_song_id. Let me reconsider. The `SongState` stores `name` and `prefix` but not the song `id`. We need the id to find the position. Let me fix this:

Add `id: String` to `SongState`:

```rust
struct SongState {
    id: String,
    name: String,
    prefix: String,
    index: Option<u32>,
    last_seen_at: DateTime<Utc>,
}
```

Update the `run_tracker` active song creation to include the id:

```rust
                                        status.last_song = Some(SongState {
                                            id: active_id.clone(),
                                            name,
                                            prefix,
                                            index,
                                            last_seen_at: Utc::now(),
                                        });
```

Now the `next_song_name()` method uses the id:

```rust
    pub async fn next_song_name(&self) -> Option<String> {
        let status = self.inner.status.read().await;
        let last_song = status.last_song.as_ref()?;
        let active_idx = status.setlist_songs.iter().position(|s| s.id == last_song.id)?;
        let next = status.setlist_songs.get(active_idx + 1)?;
        Some(sanitize_song_title(&next.name))
    }
```

Import `sanitize_song_title` at the top of the file. It's in `crate::state::stage` which is `pub(crate)`. Since `ableset.rs` is in `crate::ableset`, use:

```rust
use crate::state::stage::sanitize_song_title;
```

- [ ] **Step 5: Add `next_song_name` to `AbleSetClient` trait and mock**

Add to the `AbleSetClient` trait (line 26-31):

```rust
    fn next_song_name(&self) -> AbleSetFuture<'_, Option<String>>;
```

Implement in `AbleSetClient for AbleSetBridge` (line 257-277):

```rust
    fn next_song_name(&self) -> AbleSetFuture<'_, Option<String>> {
        let bridge = self.clone();
        Box::pin(async move { AbleSetBridge::next_song_name(&bridge).await })
    }
```

Implement in `AbleSetClient for MockAbleSetClient` (line 328-360):

```rust
    fn next_song_name(&self) -> AbleSetFuture<'_, Option<String>> {
        Box::pin(async move { None })
    }
```

- [ ] **Step 6: Write unit tests for `next_song_name`**

Add tests at the end of `ableset.rs` (in a new `#[cfg(test)] mod tests` block or extend the existing test infrastructure):

Since `AbleSetBridge` uses internal async state, we test via the public interface. Create a helper test:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn next_song_name_returns_next_when_active_song_exists() {
        let bridge = AbleSetBridge::new();
        {
            let mut status = bridge.inner.status.write().await;
            status.setlist_songs = vec![
                SetlistCachedSong { id: "s1".into(), name: "001 First Song".into() },
                SetlistCachedSong { id: "s2".into(), name: "002 Second Song".into() },
                SetlistCachedSong { id: "s3".into(), name: "003 Third Song".into() },
            ];
            status.last_song = Some(SongState {
                id: "s1".into(),
                name: "001 First Song".into(),
                prefix: "001".into(),
                index: Some(0),
                last_seen_at: Utc::now(),
            });
        }
        let next = bridge.next_song_name().await;
        assert_eq!(next, Some("Second Song".to_string()));
    }

    #[tokio::test]
    async fn next_song_name_returns_none_when_last_in_setlist() {
        let bridge = AbleSetBridge::new();
        {
            let mut status = bridge.inner.status.write().await;
            status.setlist_songs = vec![
                SetlistCachedSong { id: "s1".into(), name: "001 First Song".into() },
                SetlistCachedSong { id: "s2".into(), name: "002 Second Song".into() },
            ];
            status.last_song = Some(SongState {
                id: "s2".into(),
                name: "002 Second Song".into(),
                prefix: "002".into(),
                index: Some(1),
                last_seen_at: Utc::now(),
            });
        }
        let next = bridge.next_song_name().await;
        assert_eq!(next, None);
    }

    #[tokio::test]
    async fn next_song_name_returns_none_when_no_active_song() {
        let bridge = AbleSetBridge::new();
        {
            let mut status = bridge.inner.status.write().await;
            status.setlist_songs = vec![
                SetlistCachedSong { id: "s1".into(), name: "001 First Song".into() },
            ];
            // No last_song set
        }
        let next = bridge.next_song_name().await;
        assert_eq!(next, None);
    }
}
```

- [ ] **Step 7: Run tests**

```bash
cargo test -p presenter-server -- ableset --nocapture
```

Expected: 3 new tests pass.

- [ ] **Step 8: Commit**

```bash
cargo fmt --all
git add crates/presenter-server/src/ableset.rs
git commit -m "feat(ableset): cache full setlist and add next_song_name()

Cache the complete setlist from AbleSet's /api/setlist response
instead of just the active song. Add next_song_name() method that
returns the sanitized name of the song after the active one."
```

---

## Task 3: Wire `next_song_name` into Stage Snapshot Builder

**Files:**
- Modify: `crates/presenter-server/src/state/broadcasting.rs:81-96`
- Modify: `crates/presenter-server/src/state/mod.rs` (to access `ableset_bridge`)

- [ ] **Step 1: Add `next_song_name` resolution to `publish_stage_context`**

In `crates/presenter-server/src/state/broadcasting.rs`, update `publish_stage_context()` (line 81-96) to resolve `next_song_name` before building the snapshot:

```rust
    async fn publish_stage_context(&self, context: &StageContext) -> anyhow::Result<()> {
        let code = self.stage_layout_code().await;
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

        let mut context = context.clone();
        if context.resolution.next_song_name.is_none() {
            context.resolution.next_song_name = self.resolve_next_song_name(&context.resolution).await;
        }

        let snapshot = build_stage_snapshot(layout, &context);
        self.publish_stage_update(snapshot);
        Ok(())
    }
```

- [ ] **Step 2: Add `resolve_next_song_name` method**

Add a new method to the `impl AppState` block in `broadcasting.rs`:

```rust
    async fn resolve_next_song_name(&self, resolution: &StageResolution) -> Option<String> {
        // Try AbleSet first
        if let Some(name) = self.ableset_bridge.next_song_name().await {
            return Some(name);
        }

        // Fall back to playlist: find the next presentation after the current one
        let current_id = resolution.presentation_id?;
        let entries = resolution.playlist_entries.as_ref()?;
        let current_idx = entries.iter().position(|e| e.presentation_id == Some(current_id))?;
        let next_entry = entries.get(current_idx + 1)?;
        if next_entry.entry_type == "presentation" && !next_entry.name.is_empty() {
            Some(next_entry.name.clone())
        } else {
            None
        }
    }
```

Note: `next_entry.name` is already sanitized via `sanitize_song_title()` in `build_stage_playlist_entries()`.

- [ ] **Step 3: Verify it compiles**

```bash
cargo check -p presenter-server
```

Expected: compiles.

- [ ] **Step 4: Run existing tests to verify no regressions**

```bash
cargo test -p presenter-server --nocapture
```

Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/presenter-server/src/state/broadcasting.rs
git commit -m "feat(stage): populate next_song_name from AbleSet or playlist

Resolve next_song_name in publish_stage_context: try AbleSet's
next song first (from cached setlist), fall back to the next
presentation in the active Presenter playlist."
```

---

## Task 4: Update CSS for Song Name Boxes

**Files:**
- Modify: `crates/presenter-ui/styles/stage.css:21-71, 382-397`

- [ ] **Step 1: Reposition group pills from centered to left-aligned**

In `crates/presenter-ui/styles/stage.css`, update `.stage__current-group` (line 21-33):

```css
.stage__current-group {
    position: absolute;
    left: 2%;
    top: 1%;
    width: 35%;
    height: 5%;
    display: flex;
    align-items: stretch;
    justify-content: center;
    overflow: hidden;
    padding: 0;
    margin: 0;
}
```

Update `.stage__next-group` (line 47-59):

```css
.stage__next-group {
    position: absolute;
    left: 2%;
    top: 56%;
    width: 35%;
    height: 4%;
    display: flex;
    align-items: stretch;
    justify-content: center;
    overflow: hidden;
    padding: 0;
    margin: 0;
}
```

- [ ] **Step 2: Add song name box CSS**

Add after `.stage__next-group` (after line 59):

```css
.stage__current-song {
    position: absolute;
    right: 2%;
    top: 1%;
    width: 35%;
    height: 5%;
    display: flex;
    align-items: stretch;
    justify-content: center;
    overflow: hidden;
    padding: 0;
    margin: 0;
}

.stage__next-song {
    position: absolute;
    right: 2%;
    top: 56%;
    width: 35%;
    height: 4%;
    display: flex;
    align-items: stretch;
    justify-content: center;
    overflow: hidden;
    padding: 0;
    margin: 0;
}

.stage__song-name-text {
    width: 100%;
    height: 100%;
    overflow: hidden;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    font-weight: 600;
    text-align: center;
    line-height: 0.95;
    padding: 0;
    margin: 0;
    color: #fbbf24;
    background: rgba(251, 191, 36, 0.1);
}
```

- [ ] **Step 3: Add song name boxes to debug border rule**

Update the debug border rule (line 384-397) to include the new classes:

```css
.stage__current-group,
.stage__current-slide,
.stage__next-group,
.stage__next-slide,
.stage__current-song,
.stage__next-song,
.stage__clock,
.stage__song-number,
.stage__live-pill,
.stage__connection,
.stage-pp__slides-area,
.stage-pp__playlist-sidebar,
.stage-timer__display,
.stage-preach__display {
    border: 1px solid rgba(255, 255, 255, 0.08);
}
```

- [ ] **Step 4: Commit**

```bash
cargo fmt --all
git add crates/presenter-ui/styles/stage.css
git commit -m "feat(stage): add CSS for song name boxes in worship-snv

Reposition group pills from centered (left:25%) to left-aligned
(left:2%). Add .stage__current-song and .stage__next-song boxes
on the right side with amber (#fbbf24) styling."
```

---

## Task 5: Add Song Name Boxes to Leptos Component

**Files:**
- Modify: `crates/presenter-ui/src/components/stage/worship_snv.rs`

- [ ] **Step 1: Add song name signals and refs**

In `crates/presenter-ui/src/components/stage/worship_snv.rs`, add after `NEXT_GROUP_MAX_FONT` (line 11):

```rust
const CURRENT_SONG_MAX_FONT: f64 = 200.0;
const NEXT_SONG_MAX_FONT: f64 = 200.0;
```

After the `next_group_ref` (line 23), add:

```rust
    let current_song_ref = NodeRef::<leptos::html::Div>::new();
    let next_song_ref = NodeRef::<leptos::html::Div>::new();
```

- [ ] **Step 2: Add song name reactive closures**

After the `next_group_text` closure (line 83), add:

```rust
    let current_song_text = move || {
        ctx.snapshot
            .get()
            .and_then(|s| s.song_name)
            .unwrap_or_default()
    };

    let next_song_text = move || {
        ctx.snapshot
            .get()
            .and_then(|s| s.next_song_name)
            .unwrap_or_default()
    };
```

- [ ] **Step 3: Wire up autofit effects**

After the existing `autofit_effect` calls (line 92), add:

```rust
    autofit_effect(current_song_ref, CURRENT_SONG_MAX_FONT, current_song_text.clone());
    autofit_effect(next_song_ref, NEXT_SONG_MAX_FONT, next_song_text.clone());
```

- [ ] **Step 4: Add song name boxes to the view template**

In the `view!` macro, add after the `stage__current-group` div (after line 101):

```rust
            <div class="stage__current-song">
                <span class="stage__debug-label">"current-song"</span>
                <div node_ref=current_song_ref class="stage__song-name-text">
                    {current_song_text}
                </div>
            </div>
```

Add after the `stage__next-group` div (after line 115):

```rust
            <div class="stage__next-song">
                <span class="stage__debug-label">"next-song"</span>
                <div node_ref=next_song_ref class="stage__song-name-text">
                    {next_song_text}
                </div>
            </div>
```

- [ ] **Step 5: Verify WASM build compiles**

```bash
cd crates/presenter-ui && trunk build 2>&1 | tail -5
```

Expected: build succeeds.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add crates/presenter-ui/src/components/stage/worship_snv.rs
git commit -m "feat(stage): add current-song and next-song boxes to worship-snv

Add two new amber-colored boxes to the worship-snv Leptos component
showing song_name (current) and next_song_name. Both use autofit
text scaling."
```

---

## Task 6: E2E Test

**Files:**
- Create: `tests/e2e/stage-song-names.spec.ts`

- [ ] **Step 1: Write E2E test**

Create `tests/e2e/stage-song-names.spec.ts`:

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

async function openStageDisplay(context: BrowserContext) {
  await context.request.post(new URL("/stage/layout", baseURL).toString(), {
    data: { code: "worship-snv" },
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

test("worship-snv shows current song name in amber box", async ({
  context,
  request,
}) => {
  const consoleMessages: string[] = [];

  // Create a library with a numbered presentation
  const libResp = await request.post(
    new URL("/libraries", baseURL).toString(),
    { data: { name: `SongName Lib ${Date.now()}` } },
  );
  expect(libResp.ok()).toBeTruthy();
  const library: { id: string } = await libResp.json();

  const presResp = await request.post(
    new URL(`/libraries/${library.id}/presentations`, baseURL).toString(),
    { data: { name: "042 Hodny Chvaly" } },
  );
  expect(presResp.ok()).toBeTruthy();
  const presPayload: {
    presentation: { id: string; slides: Array<{ id: string }> };
  } = await presResp.json();
  const presentationId = presPayload.presentation.id;
  const slideId = presPayload.presentation.slides[0].id;

  // Trigger the slide
  await request.post(new URL("/stage/state", baseURL).toString(), {
    data: { presentationId, currentSlideId: slideId },
  });

  // Open stage display
  const stagePage = await openStageDisplay(context);
  stagePage.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  // Wait for the current song box to show the sanitized song name
  const currentSongBox = stagePage.locator(".stage__current-song .stage__song-name-text");
  await expect(currentSongBox).toBeVisible({ timeout: 10_000 });
  await expect(currentSongBox).toContainText("Hodny Chvaly", { timeout: 10_000 });

  // Verify amber color (check computed style)
  const color = await currentSongBox.evaluate(
    (el) => window.getComputedStyle(el).color,
  );
  expect(color).toBe("rgb(251, 191, 36)"); // #fbbf24

  // Verify next-song box exists (empty since no AbleSet/playlist)
  const nextSongBox = stagePage.locator(".stage__next-song .stage__song-name-text");
  await expect(nextSongBox).toBeVisible({ timeout: 5_000 });

  // Verify group pills are left-aligned (not centered)
  const groupLeft = await stagePage.locator(".stage__current-group").evaluate(
    (el) => window.getComputedStyle(el).left,
  );
  // left:2% of viewport — should not be 25%
  expect(parseInt(groupLeft)).toBeLessThan(100); // 2% of ~1280px = ~26px

  await stagePage.close();
  expect(consoleMessages).toEqual([]);
});

test("worship-snv shows next song from playlist", async ({
  context,
  request,
}) => {
  const consoleMessages: string[] = [];

  // Create a library with two presentations
  const libResp = await request.post(
    new URL("/libraries", baseURL).toString(),
    { data: { name: `NextSong Lib ${Date.now()}` } },
  );
  const library: { id: string } = await libResp.json();

  const pres1Resp = await request.post(
    new URL(`/libraries/${library.id}/presentations`, baseURL).toString(),
    { data: { name: "001 First Song" } },
  );
  const pres1: {
    presentation: { id: string; slides: Array<{ id: string }> };
  } = await pres1Resp.json();

  const pres2Resp = await request.post(
    new URL(`/libraries/${library.id}/presentations`, baseURL).toString(),
    { data: { name: "002 Second Song" } },
  );
  const pres2: {
    presentation: { id: string; slides: Array<{ id: string }> };
  } = await pres2Resp.json();

  // Create a playlist with both presentations
  const playlistResp = await request.post(
    new URL("/playlists", baseURL).toString(),
    {
      data: {
        name: `Test Playlist ${Date.now()}`,
        entries: [
          { type: "presentation", presentationId: pres1.presentation.id },
          { type: "presentation", presentationId: pres2.presentation.id },
        ],
      },
    },
  );
  expect(playlistResp.ok()).toBeTruthy();
  const playlist: { id: string } = await playlistResp.json();

  // Trigger first song with playlist context
  await request.post(new URL("/stage/state", baseURL).toString(), {
    data: {
      presentationId: pres1.presentation.id,
      currentSlideId: pres1.presentation.slides[0].id,
      playlistId: playlist.id,
    },
  });

  // Open stage display
  const stagePage = await openStageDisplay(context);
  stagePage.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  // Current song should show "First Song"
  const currentSongBox = stagePage.locator(".stage__current-song .stage__song-name-text");
  await expect(currentSongBox).toContainText("First Song", { timeout: 10_000 });

  // Next song should show "Second Song" (from playlist)
  const nextSongBox = stagePage.locator(".stage__next-song .stage__song-name-text");
  await expect(nextSongBox).toContainText("Second Song", { timeout: 10_000 });

  await stagePage.close();
  expect(consoleMessages).toEqual([]);
});
```

- [ ] **Step 2: Run E2E test locally**

```bash
npm run test:playwright -- stage-song-names
```

Expected: both tests pass.

- [ ] **Step 3: Commit**

```bash
git add tests/e2e/stage-song-names.spec.ts
git commit -m "test(e2e): add stage song name boxes Playwright tests

Verify: current song name displayed in amber box, next song from
playlist fallback, group pills left-aligned, zero console errors."
```

---

## Task 7: Build, Deploy Dev, Verify, Push

- [ ] **Step 1: Build WASM and server locally**

```bash
cd crates/presenter-ui && trunk build --release 2>&1 | tail -5
cd /home/newlevel/devel/presenter/presenter-dev2
cargo build --release -p presenter-server 2>&1 | tail -5
```

Expected: both succeed.

- [ ] **Step 2: Deploy to dev locally**

```bash
sudo systemctl stop presenter-dev
sudo cp target/release/presenter-server /opt/presenter-dev/presenter-server
sudo systemctl start presenter-dev
sleep 2
curl -s http://10.77.8.134:8080/healthz | jq .
```

Expected: healthz returns ok with current version.

- [ ] **Step 3: Visual verification with Playwright**

Open the dev stage display in Playwright and verify:
1. Current song box appears in amber on the right
2. Group pill is left-aligned
3. Next song box is present (even if empty without AbleSet/playlist)

- [ ] **Step 4: Run full test suite**

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all
cargo test --workspace --nocapture
```

Fix any issues.

- [ ] **Step 5: Push to dev**

```bash
git push origin dev
```

- [ ] **Step 6: Monitor CI**

```bash
gh run list --branch dev --limit 3
```

Wait for all jobs to pass. Fix any failures.

- [ ] **Step 7: Commit version bump if needed**

Check if version needs bumping per the version policy. If `dev` version matches `main`:

```bash
# Bump patch version in Cargo.toml [workspace.package].version
git add Cargo.toml Cargo.lock
git commit -m "chore: bump version to X.Y.Z"
git push origin dev
```

---

## Verification Summary

| Check | How to verify |
|-------|---------------|
| Current song box visible | Stage display shows amber box with sanitized song name (number prefix stripped) |
| Next song from AbleSet | When AbleSet is tracking, next song shows the song after active in setlist |
| Next song from playlist | When playlist is active (no AbleSet), next song shows next presentation |
| No next song | When neither AbleSet nor playlist is active, next-song box is empty |
| Group pills repositioned | Group pills are left-aligned (left:2%) not centered (left:25%) |
| Amber styling | Song name text is #fbbf24, background has subtle amber tint |
| Auto-fit text | Long song names scale down to fit the box |
| Other layouts unaffected | worship-pp, timer, preach, bible, ndi layouts unchanged |
| Zero console errors | No browser console errors or warnings |
| Existing tests pass | All unit and E2E tests still green |
