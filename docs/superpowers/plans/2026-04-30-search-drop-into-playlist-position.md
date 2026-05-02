# Drag Search Result into Open Playlist at Specific Position Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When the operator drags a presentation from the search panel and hovers over an entry inside the open playlist, show a blue insertion line above or below that entry indicating where the dropped presentation will land; on drop, insert the new entry at the chosen position.

**Architecture:** Single-file change to `crates/presenter-ui/src/components/presentation_list.rs`. Extend the existing entry-level `dragover` and `drop` handlers (currently only handle within-playlist `application/x-entry-id` reorder) to ALSO handle search drags identified by `op.dragging_from_search`. On `dragover`: compute cursor Y vs. entry's bounding-box midline and set `data-drop-position="before"` or `"after"` — the existing CSS at `operator.css:685-706` draws the 3px blue line. On `drop`: insert a new `PlaylistEntryPayload::Presentation` at the computed index in the rebuilt entries Vec, call `replace_entries()`. No server change, no CSS change, no `search.rs` change.

**Tech Stack:** Rust + Leptos 0.7 (WASM), Playwright/TypeScript.

**Spec:** `docs/superpowers/specs/2026-04-30-search-drop-into-playlist-position-design.md` (commit `d147232`).

---

## Context

PR #277 (issue #275 follow-up) merged today — main is at v0.4.47. This work piles onto `dev` for a new PR.

`presenter-ui` is excluded from the root workspace and has its own `Cargo.lock`.
- Tests: `cd crates/presenter-ui && cargo test --target x86_64-unknown-linux-gnu --lib`
- Clippy: `cd crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings`
- Format: `cd crates/presenter-ui && cargo fmt`
- Workspace fmt check (root): `cargo fmt --all --check`

**Existing infrastructure (verified):**
- `crates/presenter-ui/src/components/search.rs:299-316` — search results draggable with `data-kind="presentation"`. dragstart sets MIME `application/x-presentation-id` + `application/x-presenter-search`, sets `op.search_dragging = true` and `op.dragging_from_search = true`.
- `crates/presenter-ui/src/components/presentation_list.rs:165-485` — playlist entries (Separator and Presentation kinds) carry `data-entry-id` and `data-entry-index=idx`. Each has dragstart (sets `application/x-entry-id`), dragover (preventDefault if `get_dragging_entry().is_some()`), and drop (existing reorder: remove dragged entry, insert at target's slot).
- `crates/presenter-ui/styles/operator.css:685-706` — CSS draws a 3px blue line above (`::before`) or below (`::after`) `[data-drop-position]` entries. **Currently dead — no Rust component sets the attribute. This plan wires it up.**
- `crates/presenter-ui/src/api/playlists.rs` — `PlaylistEntryPayload` enum (Presentation/Separator with optional `entry_id` for existing entries; `None` for new entries) and `replace_entries(playlist_id, Vec<PlaylistEntryPayload>) -> Result<Playlist, ApiError>` (server uses Vec ordering as positions).
- Helper functions already in `presentation_list.rs`: `entry_to_payload(entry: &PlaylistEntry) -> PlaylistEntryPayload`, `get_entry_id(payload: &PlaylistEntryPayload) -> Option<&str>`.
- `OperatorState` (in `crates/presenter-ui/src/state/operator.rs`) — `search_dragging: RwSignal<bool>`, `dragging_from_search: RwSignal<bool>`. The latter is the discriminator: true during a search drag, false during a within-playlist reorder.

**E2E test infrastructure:**
- `tests/e2e/wasm-drag-drop.spec.ts` — describe block `"WASM Operator Drag-Drop"` with helpers `initPage(page)` for opening the operator, fixtures via `support.ts` (`startTestServer`, `deriveTestConfig`, etc.).
- Playwright's `locator.dragTo(targetLocator)` dispatches synthesized `dragstart` / `dragover` / `drop` events — works for HTML5 drag-and-drop with custom handlers when the application uses standard `event.dataTransfer.setData/getData`.

---

## File Structure

| File | Change |
|------|--------|
| `Cargo.toml` (workspace `[workspace.package]`) | `0.4.47` → `0.4.48` |
| `crates/presenter-ui/Cargo.toml` | `0.1.16` → `0.1.17` |
| `crates/presenter-ui/src/components/presentation_list.rs` | Extend each entry's `dragover` to also accept search drags + set `data-drop-position`. Extend each entry's `drop` to handle the search-drag MIME and insert at the computed index. Extend each entry with a new `dragleave` handler that clears `data-drop-position`. Applies to BOTH the Separator branch (~line 184-241) and the Presentation branch (~line 354-419). |
| `tests/e2e/wasm-drag-drop.spec.ts` | New test verifying drag from search → drop on a specific entry → insertion at the right index. |

No changes to:
- `crates/presenter-ui/src/components/search.rs` (dragstart already correct)
- `crates/presenter-ui/src/components/playlist_list.rs` (whole-playlist append handler unchanged)
- The server (`PUT /playlists/{id}/entries` already accepts arbitrary Vec ordering)
- `operator.css` (`data-drop-position` styling already covers this)
- `crates/presenter-ui/src/api/playlists.rs` (`replace_entries` signature is sufficient)

---

## Task 1: Bump version 0.4.47 → 0.4.48

**Files:**
- Modify: `Cargo.toml`
- Modify: `crates/presenter-ui/Cargo.toml`

- [ ] **Step 1: Sync with remote**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
git fetch origin
git status -sb
```
Expected: clean working tree on `dev`, ahead/behind status known.

- [ ] **Step 2: Bump workspace version**

In `/home/newlevel/devel/presenter/presenter-dev2/Cargo.toml` under `[workspace.package]`, change `version = "0.4.47"` to `version = "0.4.48"`.

- [ ] **Step 3: Bump presenter-ui version**

In `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui/Cargo.toml` under `[package]`, change `version = "0.1.16"` to `version = "0.1.17"`.

- [ ] **Step 4: Refresh both Cargo.lock files**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
cargo check -p presenter-server 2>&1 | tail -3
cd crates/presenter-ui && cargo check --target wasm32-unknown-unknown 2>&1 | tail -3 && cd ../..
```
Expected: both `Finished ... target(s)`.

- [ ] **Step 5: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
git add Cargo.toml Cargo.lock crates/presenter-ui/Cargo.toml crates/presenter-ui/Cargo.lock
git commit -m "chore: bump version to 0.4.48 (#274)"
```

---

## Task 2: Add failing E2E test

**Files:**
- Modify: `tests/e2e/wasm-drag-drop.spec.ts`

- [ ] **Step 1: Read the existing test file structure**

Open `/home/newlevel/devel/presenter/presenter-dev2/tests/e2e/wasm-drag-drop.spec.ts` and locate the `test.describe("WASM Operator Drag-Drop", () => { ... })` block. Find the existing `"playlist accepts presentation drop via test helper"` test — this is the closest analog. Add the new test inside the same describe block, just BEFORE the closing `});` of the describe.

- [ ] **Step 2: Add the new failing test**

Append this test inside the `test.describe(...)` block:

```typescript
  // Regression guard for issue #274: dragging a search result over a
  // specific entry inside the open playlist must show the line indicator
  // and insert the new entry at that exact position on drop.
  test("drag search result into specific position in open playlist (#274)", async ({
    page,
  }) => {
    const consoleMessages: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
      }
    });

    await initPage(page);

    // Pick the first playlist and open it.
    const playlist = page.locator('[data-role="playlist-item"]').first();
    const playlistCount = await playlist.count();
    if (playlistCount === 0) {
      test.skip(true, "No playlists available for this test");
      return;
    }
    await playlist.click();

    // Wait for the playlist to become active and entries to render.
    await page.waitForFunction(
      () =>
        document.querySelectorAll(
          '[data-role="presentation-item"][data-entry-index]',
        ).length >= 2,
      { timeout: 15_000 },
    );

    // Snapshot the playlist entries before drop.
    const entriesBefore = await page.evaluate(() =>
      Array.from(
        document.querySelectorAll(
          '[data-role="presentation-item"][data-entry-index]',
        ),
      ).map((el) => ({
        entryIndex: el.getAttribute("data-entry-index"),
        presentationId: el.getAttribute("data-presentation-id"),
      })),
    );
    if (entriesBefore.length < 2) {
      test.skip(true, "Need at least 2 entries in playlist for this test");
      return;
    }

    // Search for ANY presentation. We will drag the first search-result
    // presentation onto entry index 1 in the playlist (above the second
    // entry) and assert it lands at index 1 of the resulting list.
    const searchInput = page.locator('[data-role="global-search-input"]');
    await searchInput.fill("a"); // broad query; 1+ results expected
    await page.waitForSelector(
      '[data-role="search-result-item"][data-kind="presentation"]',
      { timeout: 10_000 },
    );

    const searchResult = page
      .locator('[data-role="search-result-item"][data-kind="presentation"]')
      .first();
    const draggedPresId = await searchResult.getAttribute(
      "data-presentation-id",
    );
    expect(draggedPresId, "search result must carry data-presentation-id")
      .not.toBeNull();

    const targetEntry = page.locator(
      '[data-role="presentation-item"][data-entry-index="1"]',
    );
    await expect(targetEntry).toBeVisible();

    // Drag the search result over the second entry. dragTo dispatches
    // dragstart on the source, dragover/dragenter on the target, and drop
    // on the target — exercising the real handler stack.
    await searchResult.dragTo(targetEntry, {
      // Drop in the TOP HALF of the target so the handler sets
      // data-drop-position="before" → insertion at index 1.
      targetPosition: { x: 50, y: 5 },
    });

    // Wait until the playlist re-renders with one more entry.
    await page.waitForFunction(
      (expectedCount) =>
        document.querySelectorAll(
          '[data-role="presentation-item"][data-entry-index]',
        ).length === expectedCount,
      entriesBefore.length + 1,
      { timeout: 10_000 },
    );

    // Snapshot AFTER drop.
    const entriesAfter = await page.evaluate(() =>
      Array.from(
        document.querySelectorAll(
          '[data-role="presentation-item"][data-entry-index]',
        ),
      ).map((el) => ({
        entryIndex: el.getAttribute("data-entry-index"),
        presentationId: el.getAttribute("data-presentation-id"),
      })),
    );

    // Expect: original entry that was at index 0 still at 0; the dropped
    // presentation now at index 1; original index-1 pushed to index 2.
    expect(entriesAfter).toHaveLength(entriesBefore.length + 1);
    expect(entriesAfter[0].presentationId).toBe(entriesBefore[0].presentationId);
    expect(entriesAfter[1].presentationId).toBe(draggedPresId);
    expect(entriesAfter[2].presentationId).toBe(entriesBefore[1].presentationId);

    // Browser console must remain clean.
    expect(consoleMessages).toEqual([]);
  });
```

- [ ] **Step 3: Verify the test fails before the implementation**

Run the new test against the current dev server build. Before the implementation in Task 3, the dragover handler does NOT preventDefault for search drags, so the drop bubbles up to the playlist-card handler which appends to the END of the playlist — `entriesAfter` will have the dragged presentation at index `entriesBefore.length`, not at index 1. The assertion `entriesAfter[1].presentationId === draggedPresId` will fail.

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
PRESENTER_E2E_BASE_URL=http://10.77.8.134:8080 npx playwright test wasm-drag-drop.spec.ts -g "drag search result into specific position" --reporter=line 2>&1 | tail -20
```

Expected: FAIL. Common failure modes:
- `entriesAfter[1].presentationId` doesn't match `draggedPresId` — drop fell through to the playlist-card handler and appended (this is the bug we're fixing).
- `dragTo` didn't trigger any visible insertion — handler never fired; in Task 3 we add the dragover preventDefault that Playwright needs.

Either failure mode confirms the test is detecting the absence of the feature.

- [ ] **Step 4: Commit the failing test**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
git add tests/e2e/wasm-drag-drop.spec.ts
git commit -m "test(e2e): regression test for search drop into playlist position (#274)

Add a Playwright test that drags a search-result presentation over the
second entry in the open playlist and asserts the new entry lands at
index 1 (between the first two existing entries). The test fails with
the current implementation because the entry-level dragover does not
preventDefault for search drags, so drops fall through to the
playlist-card handler which appends to the end."
```

---

## Task 3: Wire search drop into playlist position

**Files:**
- Modify: `crates/presenter-ui/src/components/presentation_list.rs`

The change extends two near-identical handler blocks: one on the **Separator** entry rendering (~line 184-241) and one on the **Presentation** entry rendering (~line 354-419). The structure of the change is the same in both blocks. To keep the plan self-contained, the full code goes into the Presentation block (since it's the more common case); the Separator block follows the same recipe.

- [ ] **Step 1: Add the cursor-vs-midline helper**

At the top of `crates/presenter-ui/src/components/presentation_list.rs`, just below the `get_dragging_entry()` function (around line 18), add:

```rust
/// Compute "before"/"after" insertion side from the cursor's Y position
/// relative to the target entry's bounding-box midline. Returns `"before"`
/// if the cursor is in the top half, `"after"` if in the bottom half.
fn drop_side_for_event(ev: &web_sys::DragEvent, target: &web_sys::Element) -> &'static str {
    let rect = target.get_bounding_client_rect();
    let midline = rect.top() + rect.height() / 2.0;
    if (ev.client_y() as f64) < midline {
        "before"
    } else {
        "after"
    }
}

/// Read the `data-drop-position` attribute the dragover handler set on
/// `target`, then clear it. Returns the position string or "after" as a
/// safe default if the attribute is missing.
fn take_drop_position(target: &web_sys::Element) -> String {
    let pos = target
        .get_attribute("data-drop-position")
        .unwrap_or_else(|| "after".to_string());
    let _ = target.remove_attribute("data-drop-position");
    pos
}
```

These helpers do NOT touch existing behavior; they are pure utilities used by the new branches in Step 3.

- [ ] **Step 2: Add `wasm_bindgen::JsCast` import if not already present**

Look near the top of the file. If you see `use wasm_bindgen::JsCast;` already, skip this step. Otherwise add:

```rust
use wasm_bindgen::JsCast;
```

This is needed because we cast `EventTarget` to `Element` to call `get_bounding_client_rect` and `set_attribute`.

- [ ] **Step 3: Extend the Presentation entry's `on:dragover` handler**

In `presentation_list.rs`, find the Presentation entry's `on:dragover` (around line 384-388, inside the Presentation branch starting around line 328). It currently reads:

```rust
                                                on:dragover=move |ev: web_sys::DragEvent| {
                                                    if get_dragging_entry().is_some() {
                                                        ev.prevent_default();
                                                    }
                                                }
```

Replace with:

```rust
                                                on:dragover={
                                                    let op_for_dragover = op.clone();
                                                    move |ev: web_sys::DragEvent| {
                                                        let is_reorder = get_dragging_entry().is_some();
                                                        let is_search = op_for_dragover.dragging_from_search.get_untracked();
                                                        if !is_reorder && !is_search {
                                                            return;
                                                        }
                                                        ev.prevent_default();
                                                        // Only the search-drag path renders the line indicator.
                                                        // Within-playlist reorder uses slot-replacement (existing behavior).
                                                        if !is_search {
                                                            return;
                                                        }
                                                        if let Some(target) = ev
                                                            .current_target()
                                                            .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
                                                        {
                                                            let side = drop_side_for_event(&ev, &target);
                                                            let _ = target.set_attribute("data-drop-position", side);
                                                        }
                                                    }
                                                }
```

- [ ] **Step 4: Add a `on:dragleave` handler on the Presentation entry**

Just below the new `on:dragover`, add:

```rust
                                                on:dragleave=move |ev: web_sys::DragEvent| {
                                                    if let Some(target) = ev
                                                        .current_target()
                                                        .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
                                                    {
                                                        let _ = target.remove_attribute("data-drop-position");
                                                    }
                                                }
```

This clears the line indicator when the cursor leaves the entry without dropping.

- [ ] **Step 5: Extend the Presentation entry's `on:drop` handler**

In the same Presentation branch, find the existing `on:drop` (around line 389-419). It currently reads:

```rust
                                                on:drop={
                                                    let target_entry_id = entry_id_drop.clone();
                                                    let playlist_id = playlist_id_reorder.clone();
                                                    let selected_playlist = ctx.selected_playlist;
                                                    let playlists = ctx.playlists;
                                                    move |ev: web_sys::DragEvent| {
                                                        ev.prevent_default();
                                                        if let Some(dragged_id) = get_dragging_entry() {
                                                            if dragged_id == target_entry_id { return; }
                                                            let playlist_id = playlist_id.clone();
                                                            let target_entry_id = target_entry_id.clone();
                                                            leptos::task::spawn_local(async move {
                                                                let current = selected_playlist.get_untracked();
                                                                if let Some(pl) = current {
                                                                    let mut entries: Vec<_> = pl.entries.iter().map(entry_to_payload).collect();
                                                                    let drag_pos = entries.iter().position(|e| get_entry_id(e) == Some(&dragged_id));
                                                                    let target_pos = entries.iter().position(|e| get_entry_id(e) == Some(&target_entry_id));
                                                                    if let (Some(from), Some(to)) = (drag_pos, target_pos) {
                                                                        let item = entries.remove(from);
                                                                        entries.insert(to, item);
                                                                        if let Ok(updated) = crate::api::playlists::replace_entries(&playlist_id, entries).await {
                                                                            selected_playlist.set(Some(updated.clone()));
                                                                        }
                                                                        if let Ok(pls) = crate::api::playlists::list_playlists().await {
                                                                            playlists.set(pls);
                                                                        }
                                                                    }
                                                                }
                                                            });
                                                        }
                                                        set_dragging_entry(None);
                                                    }
                                                }
```

Replace with:

```rust
                                                on:drop={
                                                    let target_entry_id = entry_id_drop.clone();
                                                    let playlist_id = playlist_id_reorder.clone();
                                                    let selected_playlist = ctx.selected_playlist;
                                                    let playlists = ctx.playlists;
                                                    let toast_message = ctx.toast_message;
                                                    let toast_variant = ctx.toast_variant;
                                                    let op_for_drop = op.clone();
                                                    let target_index = idx;
                                                    move |ev: web_sys::DragEvent| {
                                                        ev.prevent_default();

                                                        // ----- New: search-drag path (issue #274) -----
                                                        if op_for_drop.dragging_from_search.get_untracked() {
                                                            let drop_position = ev
                                                                .current_target()
                                                                .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
                                                                .map(|target| take_drop_position(&target))
                                                                .unwrap_or_else(|| "after".to_string());

                                                            let presentation_id = ev
                                                                .data_transfer()
                                                                .and_then(|dt| {
                                                                    dt.get_data("application/x-presentation-id").ok()
                                                                })
                                                                .filter(|s| !s.is_empty());

                                                            if let Some(presentation_id) = presentation_id {
                                                                let insert_idx = if drop_position == "before" {
                                                                    target_index
                                                                } else {
                                                                    target_index + 1
                                                                };
                                                                let playlist_id = playlist_id.clone();
                                                                leptos::task::spawn_local(async move {
                                                                    let current = selected_playlist.get_untracked();
                                                                    if let Some(pl) = current {
                                                                        let mut entries: Vec<_> = pl.entries.iter().map(entry_to_payload).collect();
                                                                        let insert_idx = insert_idx.min(entries.len());
                                                                        entries.insert(
                                                                            insert_idx,
                                                                            crate::api::playlists::PlaylistEntryPayload::Presentation {
                                                                                entry_id: None,
                                                                                presentation_id,
                                                                            },
                                                                        );
                                                                        match crate::api::playlists::replace_entries(&playlist_id, entries).await {
                                                                            Ok(updated) => {
                                                                                selected_playlist.set(Some(updated));
                                                                                toast_variant.set("success".to_string());
                                                                                toast_message.set(Some(
                                                                                    "Added presentation to playlist".to_string(),
                                                                                ));
                                                                                if let Ok(pls) = crate::api::playlists::list_playlists().await {
                                                                                    playlists.set(pls);
                                                                                }
                                                                            }
                                                                            Err(e) => {
                                                                                toast_variant.set("error".to_string());
                                                                                toast_message.set(Some(format!("Error: {e}")));
                                                                            }
                                                                        }
                                                                    }
                                                                });
                                                            }

                                                            op_for_drop.dragging_from_search.set(false);
                                                            op_for_drop.search_dragging.set(false);
                                                            return;
                                                        }

                                                        // ----- Existing: within-playlist reorder path -----
                                                        if let Some(dragged_id) = get_dragging_entry() {
                                                            if dragged_id == target_entry_id { return; }
                                                            let playlist_id = playlist_id.clone();
                                                            let target_entry_id = target_entry_id.clone();
                                                            leptos::task::spawn_local(async move {
                                                                let current = selected_playlist.get_untracked();
                                                                if let Some(pl) = current {
                                                                    let mut entries: Vec<_> = pl.entries.iter().map(entry_to_payload).collect();
                                                                    let drag_pos = entries.iter().position(|e| get_entry_id(e) == Some(&dragged_id));
                                                                    let target_pos = entries.iter().position(|e| get_entry_id(e) == Some(&target_entry_id));
                                                                    if let (Some(from), Some(to)) = (drag_pos, target_pos) {
                                                                        let item = entries.remove(from);
                                                                        entries.insert(to, item);
                                                                        if let Ok(updated) = crate::api::playlists::replace_entries(&playlist_id, entries).await {
                                                                            selected_playlist.set(Some(updated.clone()));
                                                                        }
                                                                        if let Ok(pls) = crate::api::playlists::list_playlists().await {
                                                                            playlists.set(pls);
                                                                        }
                                                                    }
                                                                }
                                                            });
                                                        }
                                                        set_dragging_entry(None);
                                                    }
                                                }
```

The new branch sits at the top of the closure body (so it short-circuits before the reorder branch). The reorder branch is BYTE-FOR-BYTE the same as the original — no behavioral change for the existing within-playlist drag.

- [ ] **Step 6: Apply the same three changes to the Separator entry**

Find the Separator branch (starting around line 175). It has its own `on:dragover` (~line 202-206) and `on:drop` (~line 207-240) — same structure as the Presentation branch. Apply the SAME three modifications:

1. Replace the existing `on:dragover` with the same extended version from Step 3 (the closure body is identical — references `op` via `op_for_dragover`).
2. Add the `on:dragleave` handler from Step 4 between `on:dragover` and `on:drop`.
3. Replace the existing `on:drop` with the same extended version from Step 5 — the only difference is that the Separator branch already has captures for `target_entry_id`, `playlist_id_reorder`, `selected_playlist`, `playlists`, and the local `idx` is in scope for `target_index = idx` exactly the same way.

After these edits, both Separator and Presentation entries handle search drops identically.

- [ ] **Step 7: Verify it compiles**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui
cargo check --target wasm32-unknown-unknown 2>&1 | tail -10
```

Expected: clean compile. Common errors and fixes:
- `cannot find type Element / set_attribute` → ensure `use wasm_bindgen::JsCast;` is at the top of the file (Step 2).
- `op moved into closure` → make sure each new closure that needs `op` clones it via `let op_for_dragover = op.clone();` and `let op_for_drop = op.clone();` BEFORE the `move |...|`.

- [ ] **Step 8: Build the dev bundle locally and replace the running dev server**

The dev server at `http://10.77.8.134:8080` runs from `/opt/presenter-dev/`. The CI deploy on push to `dev` will rebuild and redeploy automatically — but for the E2E test in Task 4 to pass locally before push, a fresh trunk build of the WASM bundle PLUS a release build of the server are needed:

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui
trunk build --release 2>&1 | tail -3
cd /home/newlevel/devel/presenter/presenter-dev2
cargo build --release -p presenter-server 2>&1 | tail -3
```

Expected: both `Finished ... target(s)`. Total time on this machine: ~2-4 min.

- [ ] **Step 9: Run the failing E2E test from Task 2 — confirm it now PASSES**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
PRESENTER_E2E_BASE_URL=http://10.77.8.134:8080 npx playwright test wasm-drag-drop.spec.ts -g "drag search result into specific position" --reporter=line 2>&1 | tail -15
```

Expected: 1 passed.

If the test times out at the `dragTo` step, the dragover handler might still not be calling preventDefault — re-check Step 3 carefully (the `if !is_reorder && !is_search { return; }` short-circuit must come AFTER reading both signals).

If `entriesAfter[1].presentationId` is wrong, check Step 5's insertion-index math: `before` → `target_index` (= `idx`); `after` → `target_index + 1`.

- [ ] **Step 10: Run all WASM unit tests + clippy + fmt**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui
cargo test --target x86_64-unknown-linux-gnu --lib 2>&1 | tail -10
cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings 2>&1 | tail -5
cargo fmt
```

Expected:
- All existing tests pass (no test added to song_parser; this change is in a different file).
- Clippy clean.
- fmt produces no diff.

- [ ] **Step 11: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
git add crates/presenter-ui/src/components/presentation_list.rs
git commit -m "feat(ui): drop search result into open playlist at specific position (#274)

Extend the entry-level dragover/drop handlers in presentation_list.rs
to handle search drags (op.dragging_from_search), in addition to the
existing within-playlist reorder path. On dragover during a search
drag, compute cursor Y vs. the entry's bounding-box midline and set
data-drop-position=\"before\"/\"after\" — the existing CSS at
operator.css:685-706 draws the 3px blue line. On drop, read the
position attribute, build a fresh PlaylistEntryPayload Vec with the
new presentation inserted at target_index (before) or target_index+1
(after), and call replace_entries(). Duplicates are allowed.

Applied identically to both Separator and Presentation entry branches
so the operator can drop on any entry slot. The within-playlist
reorder branch is byte-for-byte unchanged."
```

---

## Task 4: Local checks + push + monitor pipeline + verify dev

This task is controller-handled (NOT dispatched to a subagent). The controller runs the local checks, pushes, monitors the pipeline, verifies the live dev deploy, and sends the completion report.

- [ ] **Step 1: Final local sanity sweep**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
cargo fmt --all --check && echo "WORKSPACE FMT OK"
cd crates/presenter-ui && cargo fmt --check && echo "UI FMT OK" && cd ../..
cd crates/presenter-ui && cargo test --target x86_64-unknown-linux-gnu --lib 2>&1 | tail -5 && cd ../..
cd crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings 2>&1 | tail -3 && cd ../..
```

Expected: all four commands succeed.

- [ ] **Step 2: Push to origin/dev**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
git status -sb
git push origin dev
```

The push triggers Pipeline + PR Automation runs.

- [ ] **Step 3: Identify and monitor the new pipeline**

```bash
gh run list --branch dev --limit 2 --json databaseId,name,status,headSha
```

Note the `databaseId` of the new `Pipeline` run for the latest pushed `headSha`. Monitor with single sleep + check (per `~/devel/airuleset/modules/core/ci-monitoring.md`) — DO NOT use `gh run watch`, DO NOT poll in tight loops:

```bash
sleep 600 && gh run view <DATABASE_ID> --json status,conclusion,jobs --jq '{status, conclusion, pending: [.jobs[] | select(.status!="completed") | .name], failed: [.jobs[] | select(.conclusion=="failure") | .name]}'
```

Run as a background task. Re-poll every ~10-15 minutes until `status == "completed"`.

- [ ] **Step 4: Verify all jobs are SUCCESS**

```bash
gh run view <DATABASE_ID> --json status,conclusion,jobs --jq '{status, conclusion, allSuccess: ([.jobs[] | .conclusion=="success"] | all)}'
```

Expected: `{"status":"completed","conclusion":"success","allSuccess":true}`. If any job failed:

```bash
gh run view <DATABASE_ID> --log-failed 2>&1 | tail -100
```

Fix in ONE commit, push, monitor again. Do NOT rerun-without-fix. Do NOT skip mutation testing.

- [ ] **Step 5: Verify dev deployment is live and shows v0.4.48**

```bash
curl -s http://10.77.8.134:8080/healthz
```

Expected: `{"channel":"dev","status":"ok","version":"0.4.48"}`.

Then open the dashboard in Playwright (per `post-deploy-verification.md` — version label MUST come from the live DOM, not just curl):

```bash
# Navigate to http://10.77.8.134:8080/ui/operator
# Read the version label from DOM, confirm it's v0.4.48 (dev).
```

Sanity-test the actual feature: open a playlist with ≥2 entries, search for a presentation, drag it over the second entry, watch for the blue line above that entry, drop, and verify the new entry lands at index 1.

- [ ] **Step 6: Open the PR if not already open**

```bash
gh pr list --base main --head dev --json number,title,state,mergeable,mergeStateStatus --jq '.'
```

If empty, create:

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
gh pr create --base main --head dev --title "feat(ui): drag search result into open playlist at specific position (#274)" --body "<short summary that mirrors the spec's Goal section, links the spec + plan + issue, lists what changed and the testing approach>"
```

Confirm mergeable + clean:

```bash
gh api repos/zbynekdrlik/presenter/pulls/<NUMBER> --jq '{mergeable, mergeable_state, head_sha: .head.sha}'
```

Expected: `{"mergeable": true, "mergeable_state": "clean", ...}`.

DO NOT merge. The user merges via explicit "merge it" instruction per `pr-merge-policy.md`.

- [ ] **Step 7: Run /plan-check + /review per completion-report.md pre-completion gate**

```bash
# /plan-check — ensures original prompt + plan steps fully fulfilled
# /review — must come back 0 🔴 0 🟡 0 🔵 (every 🔵 inside the diff must be fixed; the "deferred" loophole is closed)
```

Fix any findings in additional commits and re-run BEFORE sending the completion report.

- [ ] **Step 8: Send the completion report**

Use the EXACT template from `~/devel/airuleset/modules/core/completion-report.md`. Required lines:

```
## ✅ Work Complete

**Audits & deploy:**
✅ CI: pipeline <run-id> — N/N jobs green
✅ /plan-check: N/N fulfilled
✅ /review: clean — 0 🔴 0 🟡 0 🔵
✅ Deploy: dev frontend shows v0.4.48 (dev) (matches /healthz)

---

**Goal:** <one-sentence restatement of the user's ask in plain language>
**What changed:** <user-visible outcome, 1-2 sentences>

🌐 Dev:  http://10.77.8.134:8080/ui/operator
🌐 Prod: http://10.77.9.205/ui/operator

**[presenter] PR #<N>: <full PR title>**
<full PR URL> — mergeable, clean
```

---

## Verification Summary

| Check | How to verify |
|-------|---------------|
| Drag from search shows line indicator | Hover a search-result presentation over a playlist entry; blue line at top (cursor in top half) or bottom (cursor in bottom half) — `data-drop-position` attribute set on the entry |
| Drop inserts at the chosen position | New entry lands at `entry_index` (before) or `entry_index + 1` (after) — verified by the new E2E test |
| Within-playlist reorder still works | Existing E2E test in `wasm-drag-drop.spec.ts` still passes — the existing branch is byte-for-byte unchanged |
| Search drop on Separator entry works | Dropping above a separator inserts at the separator's index; below inserts at separator's index + 1 |
| Empty playlist still gets append on drop | Drop on the playlist body (no entry-level handler fires) → falls through to playlist-card handler → append. Already worked, not broken |
| Toast feedback | "Added presentation to playlist" on success; "Error: …" on failure |
| Browser console clean | E2E asserts no errors/warnings during drag-drop |
| All 20 CI jobs green | Pipeline `<run-id>` reports `allSuccess: true` |
| Dev shows v0.4.48 | `/healthz` reports `0.4.48`; operator UI DOM shows `v0.4.48 (dev)` |
