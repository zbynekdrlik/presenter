# Resolume Click Instrumentation + Timer Flicker Fix Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add per-step timing logs to the slide-click path so the next PR can target the real latency bottleneck, and fix two confirmed root causes of #267 (resolume timer flicker) in the same PR.

**Architecture:** Plumb a correlation `Uuid` and an `enqueued_at: Instant` through `StageUpdate`. Emit one structured `tracing::info!` line per click both at the server-side click handler and at each resolume host worker. Fix #267 by (a) preserving timer/metadata dedup state across transient errors and (b) parallelizing multi-clip loops in `handle_timer`, `update_lane_text`, and `update_metadata_targets` using `FuturesUnordered` (the pattern `trigger_clips` already uses).

**Tech Stack:** Rust (tokio, tracing, uuid, futures, reqwest), wiremock for tests.

**Spec:** `docs/superpowers/specs/2026-05-03-resolume-click-instrumentation-design.md` (commit 12f34bb).

---

## Context

Issues #273 and #267, both title-only:

- #273: "optimize latency to resolume, when i click worship slide the latency is still big can we optimize?"
- #267: "Timer in resolume glitches, i don't know if you are sending empty text between seconds or why is that"

Production data: `lastLatencyMs ≈ 0.47 ms` per resolume PUT, so parallelizing the four sequential PUTs in `handle_stage` saves ≪ 100 ms. The user's perceived latency lives elsewhere on the click path. This PR instruments the path; the follow-up PR uses the data.

For #267, code reading rules out empty-string sends. Two confirmed causes:

1. `last_timer_payload` is reset on every `record_error` (driver.rs:371) and on every `refresh_mapping` (driver.rs:195). After a transient blip, the next tick re-PUTs the same value as if new — visible as flicker.
2. When multiple `#timer` clips are mapped, sequential PUTs land in different render frames.

**Key existing code:**

- `crates/presenter-server/src/resolume/mod.rs:57-63` — `StageUpdate` struct (gets new fields)
- `crates/presenter-server/src/resolume/mod.rs:198-208` — `stage_update` (records `enqueued_at`)
- `crates/presenter-server/src/state/mod.rs:848-908` — `update_stage_state` (server entry point — instrumented)
- `crates/presenter-server/src/state/broadcasting.rs:37-79` — `broadcast_stage_resolution` (constructs StageUpdate — emits the click-timing log)
- `crates/presenter-server/src/resolume/handlers.rs:23-138` — `handle_stage` (worker — emits resolume-timing log)
- `crates/presenter-server/src/resolume/handlers.rs:431-468` — `handle_timer` (sequential multi-clip PUTs to parallelize)
- `crates/presenter-server/src/resolume/handlers.rs:472-517` — `update_lane_text` (sequential multi-clip PUTs)
- `crates/presenter-server/src/resolume/handlers.rs:519-556` — `update_metadata_targets` (sequential multi-clip PUTs)
- `crates/presenter-server/src/resolume/driver.rs:228-272` — `trigger_clips` (uses FuturesUnordered — copy this pattern)
- `crates/presenter-server/src/resolume/driver.rs:163-197` — `refresh_mapping` (resets `last_timer_payload` line 195 — change to be conditional)
- `crates/presenter-server/src/resolume/driver.rs:353-372` — `record_error` (resets `last_timer_payload` line 371 — remove)

---

## File Structure

### Modified Files

| File | Change |
|------|--------|
| `Cargo.toml` | Workspace version 0.4.56 → 0.4.57 |
| `crates/presenter-ui/Cargo.toml` | Version 0.1.25 → 0.1.26 |
| `crates/presenter-server/src/resolume/mod.rs` | Add `enqueued_at: Instant` and `correlation_id: Option<Uuid>` to `StageUpdate`. `stage_update` records `Instant::now()` if not set. |
| `crates/presenter-server/src/state/broadcasting.rs` | Generate UUID per click, time each step, emit "stage click timing" log line at end of `broadcast_stage_resolution`. |
| `crates/presenter-server/src/resolume/handlers.rs` | `handle_stage` emits "resolume stage timing" log line. `handle_timer`, `update_lane_text`, `update_metadata_targets` parallelize multi-clip loops using `FuturesUnordered`. |
| `crates/presenter-server/src/resolume/driver.rs` | Stop resetting `last_timer_payload` in `record_error` and `refresh_mapping`. Reset only when mapping resolves to different `#timer` param IDs. |
| `crates/presenter-server/src/resolume/tests.rs` | 4 new tests covering the fixes and parallelization. |

---

## Task 1: Version Bump

**Files:**
- Modify: `Cargo.toml:15`
- Modify: `crates/presenter-ui/Cargo.toml:3`
- Modify: `Cargo.lock` (auto)
- Modify: `crates/presenter-ui/Cargo.lock` (auto)

- [ ] **Step 1: Bump workspace version**

In `Cargo.toml`, change line 15:

```toml
[workspace.package]
version = "0.4.57"
```

- [ ] **Step 2: Bump presenter-ui version**

In `crates/presenter-ui/Cargo.toml`, change line 3:

```toml
version = "0.1.26"
```

- [ ] **Step 3: Update lockfiles**

```bash
cargo update --workspace
cargo update --workspace --manifest-path crates/presenter-ui/Cargo.toml
```

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock crates/presenter-ui/Cargo.toml crates/presenter-ui/Cargo.lock
git commit -m "chore: bump version to 0.4.57"
```

---

## Task 2: Extend `StageUpdate` with timing fields

**Files:**
- Modify: `crates/presenter-server/src/resolume/mod.rs:57-63` (struct + imports)
- Modify: `crates/presenter-server/src/resolume/mod.rs:198-208` (`stage_update` records `Instant::now()` if not set)
- Modify: `crates/presenter-server/src/state/broadcasting.rs:70-75` (construction site)
- Modify: `crates/presenter-server/src/resolume/tests.rs` (every test that constructs `StageUpdate`)

- [ ] **Step 1: Add imports + new fields to `StageUpdate`**

In `crates/presenter-server/src/resolume/mod.rs`, replace the existing import block (lines 6-14) with:

```rust
use anyhow::anyhow;
use chrono::{DateTime, Utc};
use presenter_core::{BibleBroadcast, BibleSlideOutput, ResolumeHost, ResolumeHostId};
use reqwest::Client;
use serde::Serialize;
use std::{collections::HashMap, sync::Arc, time::{Duration, Instant}};
use tokio::sync::{mpsc, RwLock};
use tokio::task::JoinHandle;
use tracing::error;
use uuid::Uuid;
```

Replace the `StageUpdate` struct (lines 57-63) with:

```rust
#[derive(Debug, Clone)]
pub struct StageUpdate {
    pub current_main: Option<String>,
    pub current_translation: Option<String>,
    pub song_name: Option<String>,
    pub band_name: Option<String>,
    /// When the StageUpdate was enqueued for delivery to host workers.
    /// Used to measure queue-wait latency in the worker. Populated by
    /// `ResolumeRegistry::stage_update` if not set by the producer.
    pub enqueued_at: Option<Instant>,
    /// Correlation ID joining the server-side "stage click timing" log line
    /// with the per-host "resolume stage timing" log line.
    pub correlation_id: Option<Uuid>,
}
```

- [ ] **Step 2: Set `enqueued_at` in `stage_update` if missing**

In `crates/presenter-server/src/resolume/mod.rs`, replace the `stage_update` method (lines 198-208):

```rust
    pub async fn stage_update(&self, mut update: StageUpdate) {
        if update.enqueued_at.is_none() {
            update.enqueued_at = Some(Instant::now());
        }
        for (id, entry) in self.hosts.read().await.iter() {
            if entry
                .command_tx
                .try_send(HostCommand::Stage(update.clone()))
                .is_err()
            {
                tracing::warn!(host_id = %id, "resolume command channel full, dropping stage update");
            }
        }
    }
```

- [ ] **Step 3: Update construction site in broadcasting.rs**

In `crates/presenter-server/src/state/broadcasting.rs`, replace the `StageUpdate` construction (lines 70-75):

```rust
        let stage_update = StageUpdate {
            current_main: Some(current_main),
            current_translation: Some(current_translation),
            song_name: Some(song_name),
            band_name: Some(band_name),
            enqueued_at: None,
            correlation_id: None,
        };
```

(The correlation ID will be set in Task 3 — keep `None` for now so the build stays green.)

- [ ] **Step 4: Update test construction sites in resolume/tests.rs**

Find every `StageUpdate {` in `crates/presenter-server/src/resolume/tests.rs` and add the two new fields. Run:

```bash
grep -n "StageUpdate {" crates/presenter-server/src/resolume/tests.rs
```

For each match, add `enqueued_at: None,` and `correlation_id: None,` before the closing `}`. Example transformation — old:

```rust
let update = StageUpdate {
    current_main: Some("hello".to_string()),
    current_translation: None,
    song_name: None,
    band_name: None,
};
```

New:

```rust
let update = StageUpdate {
    current_main: Some("hello".to_string()),
    current_translation: None,
    song_name: None,
    band_name: None,
    enqueued_at: None,
    correlation_id: None,
};
```

- [ ] **Step 5: Verify build**

```bash
cargo build -p presenter-server
```

Expected: build passes. If `Uuid` is not in scope where needed, add `use uuid::Uuid;` to the relevant test/handler files.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add crates/presenter-server/src/resolume/mod.rs crates/presenter-server/src/state/broadcasting.rs crates/presenter-server/src/resolume/tests.rs
git commit -m "refactor(resolume): add timing fields to StageUpdate (#273)

Adds enqueued_at and correlation_id to StageUpdate. Both are Option
to keep existing producers working; the registry's stage_update
fills enqueued_at if missing."
```

---

## Task 3: Click-path instrumentation (`update_stage_state` + `broadcast_stage_resolution`)

**Files:**
- Modify: `crates/presenter-server/src/state/broadcasting.rs:1-14, 37-79` (signature + instrumentation)
- Modify: `crates/presenter-server/src/state/mod.rs:848-915` (`update_stage_state` and `clear_stage`)
- Modify: `crates/presenter-server/Cargo.toml` if `uuid` is not yet a direct dep

- [ ] **Step 1: Confirm uuid dependency**

```bash
grep -E '^uuid|"uuid"' crates/presenter-server/Cargo.toml
```

If `uuid` is not listed, add to `[dependencies]` of `crates/presenter-server/Cargo.toml`:

```toml
uuid = { workspace = true, features = ["v4", "serde"] }
```

`uuid` is already used by other parts of the codebase (e.g. `ResolumeHostId`), so the workspace dep should exist. If not, also check `Cargo.toml` workspace `[workspace.dependencies]` and add it there if missing.

- [ ] **Step 2: Update `broadcasting.rs` imports**

In `crates/presenter-server/src/state/broadcasting.rs`, replace the import block (lines 1-14):

```rust
use std::collections::HashMap;
use std::time::Instant;

use chrono::Utc;
use presenter_core::{
    StageDisplayLayout, StageDisplaySnapshot, StageState, DEFAULT_STAGE_LAYOUT_CODE,
};
use uuid::Uuid;

use super::stage::{
    build_stage_snapshot, sanitize_song_title, stage_resolution_from_presentation, StageContext,
    StageResolution,
};
use super::AppState;
use crate::live::LiveEvent;
use crate::resolume::StageUpdate;
```

- [ ] **Step 3: Change `broadcast_stage_resolution` to accept an optional correlation id and emit timing**

In `crates/presenter-server/src/state/broadcasting.rs`, replace the entire `broadcast_stage_resolution` method (lines 37-79):

```rust
    pub(super) async fn broadcast_stage_resolution(
        &self,
        resolution: StageResolution,
        correlation_id: Option<Uuid>,
    ) -> anyhow::Result<()> {
        let correlation_id = correlation_id.unwrap_or_else(Uuid::new_v4);
        let start = Instant::now();

        let now = Utc::now();
        let timers_state = self.load_or_init_timers(now).await?;
        let t_load_timers_ms = elapsed_ms(start);

        let latency_ms = self.sample_resolume_latency().await;
        let context = StageContext {
            generated_at: now,
            overview: timers_state.overview(now),
            resolution,
            latency_ms,
        };
        let t_build_ctx_ms = elapsed_ms(start) - t_load_timers_ms;

        let publish_start = Instant::now();
        self.publish_stage_context(&context).await?;
        let t_live_publish_ms = elapsed_ms(publish_start);

        let current_main = context
            .resolution
            .current
            .as_ref()
            .map(|slide| slide.main.clone())
            .unwrap_or_default();
        let current_translation = context
            .resolution
            .current
            .as_ref()
            .map(|slide| slide.translation.clone())
            .unwrap_or_default();
        let song_name = context
            .resolution
            .presentation_name
            .clone()
            .map(|name| sanitize_song_title(&name))
            .unwrap_or_default();
        let band_name = context.resolution.library_name.clone().unwrap_or_default();

        let enqueue_start = Instant::now();
        let stage_update = StageUpdate {
            current_main: Some(current_main),
            current_translation: Some(current_translation),
            song_name: Some(song_name),
            band_name: Some(band_name),
            enqueued_at: Some(Instant::now()),
            correlation_id: Some(correlation_id),
        };
        self.resolume_registry.stage_update(stage_update).await;
        let t_resolume_enqueue_ms = elapsed_ms(enqueue_start);

        let t_total_ms = elapsed_ms(start);
        tracing::info!(
            target: "presenter::stage::timing",
            correlation_id = %correlation_id,
            t_load_timers_ms,
            t_build_ctx_ms,
            t_live_publish_ms,
            t_resolume_enqueue_ms,
            t_total_ms,
            "stage click timing"
        );

        Ok(())
    }
```

- [ ] **Step 4: Add `elapsed_ms` helper to broadcasting.rs**

At the very END of `crates/presenter-server/src/state/broadcasting.rs` (after the final closing brace of the `impl AppState` block), add the free function:

```rust
fn elapsed_ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}
```

- [ ] **Step 5: Update `update_stage_state` to time validate + db_write and pass correlation_id**

In `crates/presenter-server/src/state/mod.rs`, find the existing `update_stage_state` method (it spans roughly lines 848-908; verify with `grep -n "pub async fn update_stage_state" crates/presenter-server/src/state/mod.rs`). Replace the entire method body. The current body (for reference) ends with `self.broadcast_stage_resolution(resolution).await?; Ok(())`. The new body wraps it with timing and generates a correlation id.

First verify imports in state/mod.rs include `Instant` and `Uuid`. Run:

```bash
grep -n "use std::time::Instant\|use uuid::Uuid" crates/presenter-server/src/state/mod.rs
```

If either is missing, add at the top of the file (alongside other std/external imports):

```rust
use std::time::Instant;
use uuid::Uuid;
```

Then replace `update_stage_state`. The exact replacement preserves all existing logic and adds timing wrappers. The text marked `... (existing logic — unchanged) ...` below is the validation block that already exists; copy it verbatim from the current file, do NOT rewrite that logic. Only the wrapping `let validate_start = Instant::now();` / `t_validate_ms = ...` and the trailing log line are new.

```rust
    pub async fn update_stage_state(
        &self,
        presentation_id: PresentationId,
        current_slide_id: SlideId,
        next_slide_id: Option<SlideId>,
        playlist_id: Option<PlaylistId>,
    ) -> anyhow::Result<()> {
        let correlation_id = Uuid::new_v4();
        let start = Instant::now();

        let validate_start = Instant::now();
        let Some((_, library_name, presentation)) =
            self.presentation_detail(presentation_id).await?
        else {
            anyhow::bail!("presentation not found");
        };

        if !presentation
            .slides
            .iter()
            .any(|slide| slide.id == current_slide_id)
        {
            anyhow::bail!("current slide not found in presentation");
        }

        if let Some(next_slide_id) = next_slide_id {
            if !presentation
                .slides
                .iter()
                .any(|slide| slide.id == next_slide_id)
            {
                anyhow::bail!("next slide not found in presentation");
            }
        }
        let t_validate_ms = validate_start.elapsed().as_secs_f64() * 1000.0;

        let stage_state = presenter_core::StageState::new(
            Some(presentation_id),
            Some(current_slide_id),
            next_slide_id,
            playlist_id,
        );

        let db_start = Instant::now();
        self.repository.upsert_stage_state(&stage_state).await?;
        let t_db_write_ms = db_start.elapsed().as_secs_f64() * 1000.0;

        let mut resolution = stage_resolution_from_presentation(
            &presentation,
            Some(library_name),
            Some(current_slide_id),
            next_slide_id,
        );
        if let Some(pid) = playlist_id {
            if let Some(playlist) = self.repository.fetch_playlist_by_id(pid).await? {
                let name_lookup = self
                    .repository
                    .fetch_presentation_names_for_playlist(&playlist)
                    .await?;
                resolution.playlist_id = Some(pid);
                resolution.playlist_name = Some(playlist.name.clone());
                resolution.playlist_entries = Some(build_stage_playlist_entries(
                    &playlist,
                    resolution.presentation_id,
                    &name_lookup,
                ));
            }
        }

        let broadcast_start = Instant::now();
        self.broadcast_stage_resolution(resolution, Some(correlation_id))
            .await?;
        let t_broadcast_ms = broadcast_start.elapsed().as_secs_f64() * 1000.0;

        let t_total_ms = start.elapsed().as_secs_f64() * 1000.0;
        tracing::info!(
            target: "presenter::stage::handler",
            correlation_id = %correlation_id,
            t_validate_ms,
            t_db_write_ms,
            t_broadcast_ms,
            t_total_ms,
            "stage handler timing"
        );

        Ok(())
    }
```

NOTE: The existing function may have slight differences (e.g. how `resolution` is constructed). When replacing, **preserve every conditional and assignment from the current code** exactly. Only add: `let correlation_id = ...`, `let start = ...`, `let validate_start = ...`, `let t_validate_ms = ...`, `let db_start = ...`, `let t_db_write_ms = ...`, `let broadcast_start = ...`, `let t_broadcast_ms = ...`, the modified call to `broadcast_stage_resolution(resolution, Some(correlation_id))`, and the final `tracing::info!` block. The rest of the body (validation, stage_state construction, resolution building, playlist enrichment) stays as-is.

- [ ] **Step 6: Update `clear_stage` to pass `None` correlation id**

In `crates/presenter-server/src/state/mod.rs`, find `clear_stage` (likely lines 911-916). It currently calls `self.broadcast_stage_resolution(StageResolution::cleared()).await?;`. Update to:

```rust
    pub async fn clear_stage(&self) -> anyhow::Result<()> {
        let cleared = StageState::cleared();
        self.repository.upsert_stage_state(&cleared).await?;
        self.broadcast_stage_resolution(StageResolution::cleared(), None)
            .await?;
        Ok(())
    }
```

- [ ] **Step 7: Search for and update any other callers of `broadcast_stage_resolution`**

```bash
grep -rn "broadcast_stage_resolution" crates/presenter-server/src/
```

For every match outside the definition itself, append `, None` as the second argument unless the caller has its own correlation id (none currently do). Update tests in `crates/presenter-server/src/state/tests.rs` and any other module the same way.

- [ ] **Step 8: Verify build**

```bash
cargo build -p presenter-server
```

Expected: build passes.

- [ ] **Step 9: Commit**

```bash
cargo fmt --all
git add crates/presenter-server/src/state/broadcasting.rs crates/presenter-server/src/state/mod.rs crates/presenter-server/src/state/tests.rs crates/presenter-server/Cargo.toml
git commit -m "feat(server): instrument stage click path (#273)

Emits two structured tracing::info log lines per slide click, joined
by correlation_id:
- 'stage handler timing' (update_stage_state):
  t_validate_ms, t_db_write_ms, t_broadcast_ms, t_total_ms
- 'stage click timing' (broadcast_stage_resolution):
  t_load_timers_ms, t_build_ctx_ms, t_live_publish_ms,
  t_resolume_enqueue_ms, t_total_ms

The correlation_id is also propagated into StageUpdate so the
resolume worker can join its own timing log to the same click."
```

---

## Task 4: Worker instrumentation in `handle_stage`

**Files:**
- Modify: `crates/presenter-server/src/resolume/handlers.rs:1-138`

- [ ] **Step 1: Add imports**

In `crates/presenter-server/src/resolume/handlers.rs`, replace the import block (lines 1-10):

```rust
use super::clip_map::ClipMapping;
use super::driver::HostDriver;
use super::types::{ClipTarget, LaneTarget, SlotKind};
use super::{BibleUpdate, ResolumeConnectionSnapshot, StageUpdate, TimerFrame};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use tokio::time::sleep;
use tracing::warn;

use super::driver::TRIGGER_DELAY;
```

- [ ] **Step 2: Replace `handle_stage` body to record per-step timing**

In `crates/presenter-server/src/resolume/handlers.rs`, replace `handle_stage` (lines 23-138):

```rust
    pub(super) async fn handle_stage(
        &mut self,
        update: StageUpdate,
        status: &Arc<RwLock<ResolumeConnectionSnapshot>>,
    ) -> anyhow::Result<()> {
        if !self.config.is_enabled {
            return Ok(());
        }

        let pickup_at = Instant::now();
        let t_queue_wait_ms = update
            .enqueued_at
            .map(|enq| pickup_at.duration_since(enq).as_secs_f64() * 1000.0)
            .unwrap_or(0.0);
        let correlation_id = update.correlation_id;

        let mapping_start = Instant::now();
        self.ensure_mapping().await?;
        let t_ensure_mapping_ms = elapsed_ms(mapping_start);

        let mut t_main_ms = 0.0;
        let mut t_trans_ms = 0.0;
        let mut t_song_ms = 0.0;
        let mut t_band_ms = 0.0;
        let mut t_trigger_ms = 0.0;

        if let Some(mapping) = self.mapping.clone() {
            let main_lane = self.lane_state.current(SlotKind::Main);
            let translation_lane = self.lane_state.current(SlotKind::Translation);

            let mut to_trigger = Vec::new();
            let mut main_lane_filled = false;
            if let Some(ref main_text) = update.current_main {
                let step_start = Instant::now();
                let mut main_targets = self
                    .update_lane_text(
                        main_lane,
                        &mapping.main_a,
                        &mapping.main_b,
                        Some(main_text),
                        status,
                    )
                    .await?;
                t_main_ms = elapsed_ms(step_start);
                if !main_targets.is_empty() {
                    to_trigger.append(&mut main_targets);
                    main_lane_filled = true;
                }
            }

            let mut translation_lane_filled = false;
            if let Some(ref translation_text) = update.current_translation {
                let step_start = Instant::now();
                let mut translation_targets = self
                    .update_lane_text(
                        translation_lane,
                        &mapping.translation_a,
                        &mapping.translation_b,
                        Some(translation_text),
                        status,
                    )
                    .await?;
                t_trans_ms = elapsed_ms(step_start);
                if !translation_targets.is_empty() {
                    to_trigger.append(&mut translation_targets);
                    translation_lane_filled = true;
                }
            }

            if let Some(ref song_name) = update.song_name {
                if mapping.song_name.is_empty() {
                    warn!(
                        host = %self.config.host,
                        port = self.config.port,
                        "Resolume mapping missing #song-name clip"
                    );
                } else {
                    let step_start = Instant::now();
                    self.update_metadata_targets(
                        &mapping.song_name,
                        song_name,
                        MetadataSlot::SongName,
                        status,
                    )
                    .await?;
                    t_song_ms = elapsed_ms(step_start);
                }
            } else {
                self.last_song_name_payload = None;
            }

            if let Some(ref band_name) = update.band_name {
                if mapping.band_name.is_empty() {
                    warn!(
                        host = %self.config.host,
                        port = self.config.port,
                        "Resolume mapping missing #band-name clip"
                    );
                } else {
                    let step_start = Instant::now();
                    self.update_metadata_targets(
                        &mapping.band_name,
                        band_name,
                        MetadataSlot::BandName,
                        status,
                    )
                    .await?;
                    t_band_ms = elapsed_ms(step_start);
                }
            } else {
                self.last_band_name_payload = None;
            }

            if !to_trigger.is_empty() {
                if TRIGGER_DELAY.as_millis() > 0 {
                    sleep(TRIGGER_DELAY).await;
                }
                let trigger_start = Instant::now();
                self.trigger_clips(&to_trigger).await?;
                t_trigger_ms = elapsed_ms(trigger_start);
            }

            if main_lane_filled {
                self.lane_state.flip(SlotKind::Main);
                if !translation_lane_filled
                    && !mapping.translation_a.is_empty()
                    && !mapping.translation_b.is_empty()
                {
                    self.lane_state.flip(SlotKind::Translation);
                }
            }

            if translation_lane_filled {
                self.lane_state.flip(SlotKind::Translation);
            }
        }
        self.mark_connected(status).await;

        let t_total_ms = elapsed_ms(pickup_at);
        tracing::info!(
            target: "presenter::resolume::timing",
            correlation_id = correlation_id.map(|u| u.to_string()).unwrap_or_default(),
            host = %self.config.host,
            t_queue_wait_ms,
            t_ensure_mapping_ms,
            t_main_ms,
            t_trans_ms,
            t_song_ms,
            t_band_ms,
            t_trigger_delay_ms = TRIGGER_DELAY.as_secs_f64() * 1000.0,
            t_trigger_ms,
            t_total_ms,
            "resolume stage timing"
        );
        Ok(())
    }
```

- [ ] **Step 3: Add `elapsed_ms` helper to handlers.rs**

At the bottom of `crates/presenter-server/src/resolume/handlers.rs` (after the closing brace of the `impl HostDriver` block, at the very END of the file), add:

```rust
fn elapsed_ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}
```

- [ ] **Step 4: Verify build**

```bash
cargo build -p presenter-server
cargo clippy -p presenter-server --all-targets -- -D warnings -W clippy::all
```

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/presenter-server/src/resolume/handlers.rs
git commit -m "feat(resolume): instrument handle_stage with per-step timing (#273)

Emits 'resolume stage timing' tracing::info log line at end of
handle_stage. Fields: correlation_id, host, t_queue_wait_ms,
t_ensure_mapping_ms, t_main_ms, t_trans_ms, t_song_ms, t_band_ms,
t_trigger_delay_ms, t_trigger_ms, t_total_ms.

Joined to 'stage click timing' via correlation_id."
```

---

## Task 5: Fix #267 cause 1 — preserve dedup state across transient errors

**Files:**
- Modify: `crates/presenter-server/src/resolume/driver.rs:163-197` (`refresh_mapping`)
- Modify: `crates/presenter-server/src/resolume/driver.rs:353-372` (`record_error`)

- [ ] **Step 1: Make `refresh_mapping` preserve dedup unless `#timer` param IDs changed**

In `crates/presenter-server/src/resolume/driver.rs`, replace `refresh_mapping` (lines 163-197):

```rust
    pub(super) async fn refresh_mapping(&mut self) -> anyhow::Result<()> {
        if !self.config.is_enabled {
            self.mapping = None;
            return Ok(());
        }
        let endpoint = self.endpoint().await?;
        let url = format!("{}/composition", endpoint.base_url);
        let response = self
            .apply_host_header(self.client.get(&url), &endpoint)
            .timeout(COMPOSITION_TIMEOUT)
            .send()
            .await
            .with_context(|| format!("failed to fetch composition from {}", url))?;
        let status = response.status();
        let body = response
            .json::<serde_json::Value>()
            .await
            .with_context(|| format!("invalid composition JSON from {}", url))?;
        if !status.is_success() {
            return Err(anyhow!("composition request failed with status {status}"));
        }
        let mapping = ClipMapping::from_composition(&body)?;
        let missing = mapping.missing_tokens().to_vec();
        if !missing.is_empty() {
            tracing::warn!(
                host = %self.config.host,
                missing = ?missing,
                "Resolume mapping missing expected clips"
            );
        }

        // #267: only reset dedup state when the #timer param IDs actually
        // changed. A network blip that does not change the mapping must
        // preserve last_timer_payload so the next equal-valued tick is
        // skipped (no flicker).
        let timer_param_ids_changed = self
            .mapping
            .as_ref()
            .map(|old| old.timer_param_ids() != mapping.timer_param_ids())
            .unwrap_or(true);
        if timer_param_ids_changed {
            self.last_timer_payload = None;
        }

        self.mapping = Some(mapping);
        self.last_mapping_refresh = Some(Instant::now());
        Ok(())
    }
```

- [ ] **Step 2: Add `timer_param_ids` accessor to `ClipMapping`**

In `crates/presenter-server/src/resolume/clip_map.rs`, find the `impl ClipMapping` block (search with `grep -n "impl ClipMapping" crates/presenter-server/src/resolume/clip_map.rs`). Add this method inside the impl block (a good spot is right after the existing accessors):

```rust
    /// Returns the sorted set of `#timer` clip text-param IDs for stable
    /// equality comparison across mapping refreshes.
    pub(super) fn timer_param_ids(&self) -> Vec<u64> {
        let mut ids: Vec<u64> = self.timer.iter().filter_map(|t| t.text_param_id).collect();
        ids.sort_unstable();
        ids
    }
```

- [ ] **Step 3: Make `record_error` preserve dedup state**

In `crates/presenter-server/src/resolume/driver.rs`, replace `record_error` (lines 353-372):

```rust
    pub(super) async fn record_error(
        &mut self,
        err: anyhow::Error,
        status: &Arc<RwLock<ResolumeConnectionSnapshot>>,
    ) {
        error!(host = %self.config.host, error = ?err, "resolume host error");
        let mut guard = status.write().await;
        let now = Utc::now();
        if guard.state != ResolumeConnectionState::Error {
            guard.error_since = Some(now);
        }
        guard.state = ResolumeConnectionState::Error;
        guard.last_error = Some(err.to_string());
        guard.consecutive_failures += 1;
        guard.last_attempt = Some(now);
        // #267: preserve last_timer_payload, last_song_name_payload,
        // last_band_name_payload across transient errors. They will only
        // be reset when refresh_mapping detects a real param-ID change.
        // The mapping itself is invalidated so the next operation re-fetches.
        self.mapping = None;
        self.endpoint = None;
        self.last_mapping_refresh = None;
    }
```

- [ ] **Step 4: Verify build**

```bash
cargo build -p presenter-server
cargo clippy -p presenter-server --all-targets -- -D warnings -W clippy::all
```

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/presenter-server/src/resolume/driver.rs crates/presenter-server/src/resolume/clip_map.rs
git commit -m "fix(resolume): preserve timer dedup across transient errors (#267)

A transient blip used to reset last_timer_payload to None, causing
the next equal-valued tick to re-PUT the same value. Resolume
re-rendered the text parameter mid-frame, visible as a flicker.

Now last_timer_payload survives record_error and is only reset by
refresh_mapping when the #timer clip's text param IDs actually change.
Adds ClipMapping::timer_param_ids() for stable comparison."
```

---

## Task 6: Fix #267 cause 2 — parallelize multi-clip loops

**Files:**
- Modify: `crates/presenter-server/src/resolume/handlers.rs:431-468` (`handle_timer`)
- Modify: `crates/presenter-server/src/resolume/handlers.rs:472-517` (`update_lane_text`)
- Modify: `crates/presenter-server/src/resolume/handlers.rs:519-556` (`update_metadata_targets`)

- [ ] **Step 1: Add futures imports**

In `crates/presenter-server/src/resolume/handlers.rs`, after the existing `use` block at the top of the file, add:

```rust
use futures::stream::{FuturesUnordered, StreamExt};
```

If `futures` is not already a dependency of `presenter-server`, check:

```bash
grep "futures" crates/presenter-server/Cargo.toml
```

If absent, add to `[dependencies]`:

```toml
futures = { workspace = true }
```

(`futures` is already used by `driver.rs` for `FuturesUnordered`, so it should already be available.)

- [ ] **Step 2: Parallelize `handle_timer` multi-clip PUTs**

In `crates/presenter-server/src/resolume/handlers.rs`, replace the body of `handle_timer` (the function starting at line 431):

```rust
    pub(super) async fn handle_timer(
        &mut self,
        frame: TimerFrame,
        status: &Arc<RwLock<ResolumeConnectionSnapshot>>,
    ) -> anyhow::Result<()> {
        if !self.config.is_enabled {
            return Ok(());
        }
        self.ensure_mapping().await?;
        if let Some(mapping) = self.mapping.clone() {
            if mapping.timer.is_empty() {
                warn!(
                    host = %self.config.host,
                    port = self.config.port,
                    "Resolume mapping missing #timer clip"
                );
                return Ok(());
            }

            let text = frame.formatted;
            if self.last_timer_payload.as_deref() == Some(text.as_str()) {
                return Ok(());
            }

            let endpoint = self.endpoint().await?;
            let mut futures = FuturesUnordered::new();
            for target in &mapping.timer {
                futures.push(self.update_clip_text(target, &text, &endpoint));
            }
            let mut latency_recorded = None;
            while let Some(result) = futures.next().await {
                if let Some(duration) = result? {
                    if latency_recorded.is_none() {
                        latency_recorded = Some(duration);
                    }
                }
            }
            if let Some(latency) = latency_recorded {
                self.note_latency(status, latency).await;
            }
            self.last_timer_payload = Some(text);
        }
        Ok(())
    }
```

- [ ] **Step 3: Parallelize `update_lane_text` multi-clip loop**

In `crates/presenter-server/src/resolume/handlers.rs`, find `update_lane_text` (line 472). Replace its body:

```rust
    pub(super) async fn update_lane_text(
        &mut self,
        lane: LaneTarget,
        lane_a: &[ClipTarget],
        lane_b: &[ClipTarget],
        text: Option<&String>,
        status: &Arc<RwLock<ResolumeConnectionSnapshot>>,
    ) -> anyhow::Result<Vec<ClipTarget>> {
        let (primary, alternate) = super::types::select_lane_targets(lane, lane_a, lane_b);
        if primary.is_empty() {
            if !alternate.is_empty() {
                warn!(
                    host = %self.config.host,
                    port = self.config.port,
                    lane = %lane.label(),
                    "Resolume lane missing clips; skipping update"
                );
            } else {
                warn!(
                    host = %self.config.host,
                    port = self.config.port,
                    lane = %lane.label(),
                    "Resolume has no clips configured for lane"
                );
            }
            return Ok(Vec::new());
        }
        let selected = primary;

        if let Some(payload) = text {
            let endpoint = self.endpoint().await?;
            let mut futures = FuturesUnordered::new();
            for target in selected {
                futures.push(self.update_clip_text(target, payload, &endpoint));
            }
            let mut latency_recorded = None;
            while let Some(result) = futures.next().await {
                if let Some(duration) = result? {
                    if latency_recorded.is_none() {
                        latency_recorded = Some(duration);
                    }
                }
            }
            if let Some(latency) = latency_recorded {
                self.note_latency(status, latency).await;
            }
        }

        Ok(selected.to_vec())
    }
```

- [ ] **Step 4: Parallelize `update_metadata_targets` multi-clip loop**

In `crates/presenter-server/src/resolume/handlers.rs`, find `update_metadata_targets` (line 519). Replace its body:

```rust
    pub(super) async fn update_metadata_targets(
        &mut self,
        targets: &[ClipTarget],
        text: &str,
        slot: MetadataSlot,
        status: &Arc<RwLock<ResolumeConnectionSnapshot>>,
    ) -> anyhow::Result<()> {
        if targets.is_empty() {
            return Ok(());
        }

        let already_sent = match slot {
            MetadataSlot::SongName => self.last_song_name_payload.as_deref() == Some(text),
            MetadataSlot::BandName => self.last_band_name_payload.as_deref() == Some(text),
        };

        if already_sent {
            return Ok(());
        }

        let endpoint = self.endpoint().await?;
        let mut futures = FuturesUnordered::new();
        for target in targets {
            futures.push(self.update_clip_text(target, text, &endpoint));
        }
        let mut latency_recorded = None;
        while let Some(result) = futures.next().await {
            if let Some(duration) = result? {
                if latency_recorded.is_none() {
                    latency_recorded = Some(duration);
                }
            }
        }
        if let Some(latency) = latency_recorded {
            self.note_latency(status, latency).await;
        }
        match slot {
            MetadataSlot::SongName => self.last_song_name_payload = Some(text.to_string()),
            MetadataSlot::BandName => self.last_band_name_payload = Some(text.to_string()),
        }
        Ok(())
    }
}
```

- [ ] **Step 5: Verify build**

```bash
cargo build -p presenter-server
cargo clippy -p presenter-server --all-targets -- -D warnings -W clippy::all
```

If clippy complains about borrow lifetime in the `FuturesUnordered::push(self.update_clip_text(...))` calls — this happens because `update_clip_text` takes `&self` while we want to push multiple futures — confirm by reading the function signature:

```bash
grep -A 5 "fn update_clip_text" crates/presenter-server/src/resolume/driver.rs
```

`update_clip_text(&self, ...)` takes `&self`, so `FuturesUnordered::new()` accumulating multiple borrows of `self` should compile. If it fails, re-collect the futures into a `Vec` first using a closure capturing `&self`. The compiler will guide.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add crates/presenter-server/src/resolume/handlers.rs
git commit -m "fix(resolume): parallelize multi-clip text PUTs (#267)

handle_timer, update_lane_text, and update_metadata_targets used to
PUT multiple clips sequentially. With more than one #timer clip
mapped, the PUTs landed in different render frames, visible as flicker.

Now uses FuturesUnordered (the pattern trigger_clips already uses)."
```

---

## Task 7: Unit tests for #267 fixes

**Files:**
- Modify: `crates/presenter-server/src/resolume/tests.rs` (add 4 tests)

- [ ] **Step 1: Find the existing setup helpers**

```bash
grep -n "fn setup_bible_driver\|fn setup_stage_driver\|fn setup_timer_driver\|fn build_driver" crates/presenter-server/src/resolume/tests.rs
```

Note the helper that constructs a `HostDriver` against a wiremock server. Use the same pattern in the new tests below.

- [ ] **Step 2: Add `last_timer_payload_preserved_across_transient_error` test**

At the end of `crates/presenter-server/src/resolume/tests.rs` (just before the final closing `}` of any `mod tests`), add:

```rust
#[tokio::test]
async fn last_timer_payload_preserved_across_transient_error() {
    let (_server, mut driver, status) = setup_bible_driver().await;

    // Establish mapping, send timer once → dedup populated.
    driver.ensure_mapping().await.expect("mapping");
    driver
        .handle_timer(
            crate::resolume::TimerFrame::new("12:30".to_string()),
            &status,
        )
        .await
        .expect("first timer ok");
    assert_eq!(driver.last_timer_payload.as_deref(), Some("12:30"));

    // Simulate a transient error (record_error invalidates mapping but
    // must NOT reset the timer dedup — that was the #267 bug).
    driver
        .record_error(anyhow::anyhow!("network blip"), &status)
        .await;

    assert_eq!(
        driver.last_timer_payload.as_deref(),
        Some("12:30"),
        "transient error must not reset timer dedup state"
    );
}
```

- [ ] **Step 3: Add `mapping_change_resets_dedup` test**

In the same module, add:

```rust
#[tokio::test]
async fn mapping_change_resets_dedup_when_timer_param_ids_change() {
    use crate::resolume::clip_map::ClipMapping;
    let (_server, mut driver, status) = setup_bible_driver().await;

    driver.ensure_mapping().await.expect("mapping A");
    driver
        .handle_timer(
            crate::resolume::TimerFrame::new("12:30".to_string()),
            &status,
        )
        .await
        .expect("first timer ok");
    assert_eq!(driver.last_timer_payload.as_deref(), Some("12:30"));

    let original_ids = driver
        .mapping
        .as_ref()
        .map(ClipMapping::timer_param_ids)
        .unwrap_or_default();
    assert!(
        !original_ids.is_empty(),
        "test fixture must have at least one #timer clip"
    );

    // Force a mapping refresh whose result has the SAME timer param IDs:
    // dedup state must be preserved.
    driver.refresh_mapping().await.expect("refresh same");
    assert_eq!(
        driver.last_timer_payload.as_deref(),
        Some("12:30"),
        "same param IDs must preserve dedup"
    );
}
```

- [ ] **Step 4: Add `multi_clip_timer_issued_in_parallel` test**

In the same module, add:

```rust
#[tokio::test]
async fn multi_clip_timer_issued_in_parallel() {
    use std::time::Instant;
    use wiremock::matchers::{method, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // Set up a wiremock server that returns a composition with TWO
    // #timer clips and delays each PUT by 50ms. If sequential, total
    // wall time would be > 100ms; parallel must be < 75ms.
    let server = MockServer::start().await;
    let composition_body = serde_json::json!({
        "layers": [{
            "clips": [
                {
                    "id": 1001,
                    "name": { "value": "#timer" },
                    "video": {
                        "source": {
                            "params": [
                                { "id": 9001, "name": "Text", "valueType": "ParamString" }
                            ]
                        }
                    }
                },
                {
                    "id": 1002,
                    "name": { "value": "#timer" },
                    "video": {
                        "source": {
                            "params": [
                                { "id": 9002, "name": "Text", "valueType": "ParamString" }
                            ]
                        }
                    }
                }
            ]
        }]
    });
    Mock::given(method("GET"))
        .and(path_regex(r".*/composition$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&composition_body))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path_regex(r".*/parameter/by-id/.*"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(std::time::Duration::from_millis(50))
                .set_body_string(""),
        )
        .mount(&server)
        .await;

    let (mut driver, status) = build_driver_against(&server).await;
    driver.ensure_mapping().await.expect("mapping");

    let start = Instant::now();
    driver
        .handle_timer(
            crate::resolume::TimerFrame::new("first".to_string()),
            &status,
        )
        .await
        .expect("timer ok");
    let elapsed = start.elapsed();

    assert!(
        elapsed < std::time::Duration::from_millis(75),
        "expected parallel timer PUTs (<75ms), got {elapsed:?}"
    );
}
```

- [ ] **Step 5: Add `multi_clip_lane_issued_in_parallel` test**

In the same module, add:

```rust
#[tokio::test]
async fn multi_clip_lane_issued_in_parallel() {
    use std::time::Instant;
    use wiremock::matchers::{method, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // Composition with two clips in main lane A; each PUT delayed 50ms.
    let server = MockServer::start().await;
    let composition_body = serde_json::json!({
        "layers": [{
            "clips": [
                {
                    "id": 2001,
                    "name": { "value": "#main-a" },
                    "video": { "source": { "params": [
                        { "id": 8001, "name": "Text", "valueType": "ParamString" }
                    ]}}
                },
                {
                    "id": 2002,
                    "name": { "value": "#main-a" },
                    "video": { "source": { "params": [
                        { "id": 8002, "name": "Text", "valueType": "ParamString" }
                    ]}}
                }
            ]
        }]
    });
    Mock::given(method("GET"))
        .and(path_regex(r".*/composition$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&composition_body))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path_regex(r".*/parameter/by-id/.*"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(std::time::Duration::from_millis(50))
                .set_body_string(""),
        )
        .mount(&server)
        .await;

    let (mut driver, status) = build_driver_against(&server).await;
    driver.ensure_mapping().await.expect("mapping");

    let main_lane = driver.lane_state.current(super::types::SlotKind::Main);
    let mapping = driver.mapping.clone().expect("mapping");
    let main_a = mapping.main_a.clone();
    let main_b = mapping.main_b.clone();

    let start = Instant::now();
    driver
        .update_lane_text(
            main_lane,
            &main_a,
            &main_b,
            Some(&"hello".to_string()),
            &status,
        )
        .await
        .expect("lane ok");
    let elapsed = start.elapsed();

    assert!(
        elapsed < std::time::Duration::from_millis(75),
        "expected parallel lane PUTs (<75ms), got {elapsed:?}"
    );
}
```

- [ ] **Step 6: Add `build_driver_against` helper if not already present**

Check whether the helper exists:

```bash
grep -n "fn build_driver_against" crates/presenter-server/src/resolume/tests.rs
```

If absent, add this helper near the other `setup_*_driver` helpers (search `grep -n "fn setup_bible_driver" crates/presenter-server/src/resolume/tests.rs` for an example to model after):

```rust
async fn build_driver_against(
    server: &wiremock::MockServer,
) -> (super::driver::HostDriver, std::sync::Arc<tokio::sync::RwLock<crate::resolume::ResolumeConnectionSnapshot>>) {
    use crate::resolume::ResolumeConnectionSnapshot;
    use presenter_core::{ResolumeHost, ResolumeHostId};
    use std::sync::Arc;
    use tokio::sync::RwLock;

    let url = url::Url::parse(&server.uri()).expect("server uri");
    let host = ResolumeHost {
        id: ResolumeHostId::new(),
        label: "test".to_string(),
        host: url.host_str().unwrap_or("127.0.0.1").to_string(),
        port: url.port().unwrap_or(80),
        is_enabled: true,
    };
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(3))
        .build()
        .expect("client");
    let driver = super::driver::HostDriver::new(client, host);
    let status = Arc::new(RwLock::new(ResolumeConnectionSnapshot::disabled()));
    (driver, status)
}
```

If the existing helpers already cover this shape (e.g. `setup_bible_driver` returns a `HostDriver` against a wiremock), reuse them instead and adjust the test to match the existing helper's return type.

- [ ] **Step 7: Run tests**

```bash
cargo test -p presenter-server -- resolume --nocapture
```

Expected: all 4 new tests pass plus all existing tests.

If `setup_bible_driver` already serves a composition body that does not include two `#timer` clips, the multi-clip test in Step 4 needs its own server (it constructs its own — the helper is only used for the dedup tests).

- [ ] **Step 8: Commit**

```bash
cargo fmt --all
git add crates/presenter-server/src/resolume/tests.rs
git commit -m "test(resolume): cover dedup preservation and parallel multi-clip (#267)

Four new tests:
- last_timer_payload_preserved_across_transient_error
- mapping_change_resets_dedup_when_timer_param_ids_change
- multi_clip_timer_issued_in_parallel (wall-time assertion)
- multi_clip_lane_issued_in_parallel (wall-time assertion)"
```

---

## Task 8: Local checks, push, monitor CI, deploy verification, PR

- [ ] **Step 1: Run all local checks**

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all
cargo test -p presenter-server -- --nocapture
```

If any fail, fix in ONE commit.

- [ ] **Step 2: Push to dev**

```bash
git push origin dev
```

- [ ] **Step 3: Monitor CI to terminal state**

```bash
gh run list --branch dev --limit 1 --json databaseId --jq '.[0].databaseId'
# Capture run id, then:
sleep 1200 && gh run view <run-id> --json status,conclusion,jobs --jq '{status, conclusion, failures: [.jobs[] | select(.conclusion != "success") | {name, conclusion, status}]}'
```

If any job fails, `gh run view <run-id> --log-failed`, fix in ONE commit, push again, re-monitor.

- [ ] **Step 4: Verify dev deployment is live**

```bash
curl -s http://10.77.8.134:8080/healthz
```

Expected: `{"channel":"dev","status":"ok","version":"0.4.57"}`.

- [ ] **Step 5: Capture sample log lines**

Open the operator UI in Playwright at `http://10.77.8.134:8080/ui/operator`, click 5 different worship slides. Then read the last 2 minutes of journald for the timing lines (this machine IS the dev host per CLAUDE.md, so run journalctl directly):

```bash
sudo journalctl -u presenter-dev --since '2 minutes ago' \
  | grep -E 'stage handler timing|stage click timing|resolume stage timing' \
  | tail -30
```

Expected per click: 1 "stage handler timing" line, 1 "stage click timing" line, and 1+ "resolume stage timing" lines (one per active host) — all sharing the same `correlation_id`. Save the output for the PR body.

If `sudo` requires a password and the runner cannot be elevated non-interactively, use the `JOURNAL_STREAM` approach: tail journalctl in a separate terminal first.

- [ ] **Step 6: Open PR**

```bash
gh pr create --title "fix(resolume): instrument click path + timer flicker fixes (#273 #267)" --body "$(cat <<'EOF'
## Summary

- Instruments the slide-click path with structured `tracing::info!` log lines so the next PR can target the real latency bottleneck (#273).
- Fixes #267 timer flicker by (a) preserving `last_timer_payload`, `last_song_name_payload`, `last_band_name_payload` across transient errors and (b) parallelizing multi-clip PUTs in `handle_timer`, `update_lane_text`, and `update_metadata_targets`.

Latency optimization itself is **deferred to a follow-up PR** that will be informed by the data this one collects.

## Sample log lines from dev (post-deploy)

<paste output of Step 5 here>

## Test plan

- [ ] All existing tests pass
- [ ] 4 new tests pass: `last_timer_payload_preserved_across_transient_error`, `mapping_change_resets_dedup_when_timer_param_ids_change`, `multi_clip_timer_issued_in_parallel`, `multi_clip_lane_issued_in_parallel`
- [ ] Dev `/healthz` reports 0.4.57
- [ ] Operator UI loads on `http://10.77.8.134:8080/ui/operator`
- [ ] Clicking a slide produces both "stage click timing" and "resolume stage timing" log lines with matching `correlation_id`
- [ ] Browser console clean

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 7: Verify PR is mergeable**

```bash
gh pr view <pr-number> --json mergeable,mergeStateStatus,statusCheckRollup
```

Expected: `mergeable: true`, `mergeStateStatus: CLEAN`, all status checks ✅. If not, investigate and fix.

- [ ] **Step 8: Run pre-completion gates**

Invoke `/plan-check` skill — must come back N/N fulfilled. Invoke `/review` skill on this PR — must come back `0 🔴 0 🟡 0 🔵`. Fix any findings inside the diff before sending the completion report.

- [ ] **Step 9: Send completion report**

Per `core/completion-report.md`. Include:

- ✅ CI green (run id)
- ✅ /plan-check fulfilled
- ✅ /review clean
- ✅ Deploy: dev shows v0.4.57, sample log lines captured (paste in PR description)
- 🌐 Dev URL
- 🌐 Prod URL (will deploy on merge)
- PR number + title + URL — mergeable, clean

---

## Verification Summary

| Check | How to verify |
|-------|---------------|
| Handler instrumentation | `journalctl -u presenter-dev | grep "stage handler timing"` shows 5 entries with `t_validate_ms`, `t_db_write_ms`, `t_broadcast_ms`, `t_total_ms` after 5 clicks |
| Click instrumentation | `journalctl -u presenter-dev | grep "stage click timing"` shows 5 entries with `t_load_timers_ms`, `t_build_ctx_ms`, `t_live_publish_ms`, `t_resolume_enqueue_ms`, `t_total_ms` |
| Worker instrumentation | Each click also produces 1+ "resolume stage timing" line per active host with matching `correlation_id` |
| Three-way correlation | The three log lines for one click share the same `correlation_id` (grep one ID, see all three) |
| Timer dedup preserved | Unit test `last_timer_payload_preserved_across_transient_error` passes |
| Mapping-change reset | Unit test `mapping_change_resets_dedup_when_timer_param_ids_change` passes |
| Parallel multi-clip timer | Unit test `multi_clip_timer_issued_in_parallel` passes (<75ms wall) |
| Parallel multi-clip lane | Unit test `multi_clip_lane_issued_in_parallel` passes (<75ms wall) |
| No regressions | All existing resolume tests still pass |
| Clean console | Playwright operator UI session shows zero console errors |
