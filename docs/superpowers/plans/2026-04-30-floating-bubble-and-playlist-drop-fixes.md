# Floating Song Bubble + Playlist Drop Edge-Case Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix 3 drag-drop edge cases (empty playlist, drop above first entry, drop below last entry) reported by the user after PR #282 merged, and replace the slides toolbar with a floating song-name bubble (draggable into playlists) + floating "+" add-slide button per issue #272.

**Architecture:** Reuse the `handle_search_drop` helper added in PR #282 for all the edge cases — head/tail spacers and the empty-state `<li>` all call it directly with hardcoded `target_index`. The new floating bubble emits the same `application/x-presentation-id` MIME as search results, so the existing drop infrastructure handles it for free. The "Line limit" input moves to a new "Preferences" card in `/ui/settings`, persisted via `localStorage["lineLimit"]` exactly as today.

**Tech Stack:** Rust + Leptos 0.7 (WASM), TypeScript/Playwright, vanilla JS in the SSR settings page.

**Spec:** `docs/superpowers/specs/2026-04-30-floating-bubble-and-playlist-drop-fixes-design.md` (commit `d37c08a`).

---

## Context

Builds on PR #282 (issue #274 search drop). `dev` is currently at `2990687` and main at `6000cf0`. New work piles onto `dev` for a new PR.

**Existing infrastructure (verified):**
- `presentation_list.rs:55-115` — `fn handle_search_drop(ev, target_index, playlist_id, selected_playlist, playlists, toast_message, toast_variant)` already implemented. Reads `data-drop-position` from the target element, parses the dragged presentation id from the dataTransfer, inserts at `target_index` (before) or `target_index+1` (after), calls `replace_entries()`. Reuse from all three new drop targets (empty-state, head spacer, tail spacer).
- `presentation_list.rs:264-268` — `<li class="empty">"Playlist is empty…"` rendered when `playlist.entries.is_empty()`. Currently has no event handlers.
- `slide_list.rs:209` — `let add_slide = move |_| { ... }` closure. Reuse on the new floating "+" button.
- `slide_list.rs:238-260` — existing `.operator__slides-toolbar` block with "Line limit" input and "+" button. Delete entirely.
- `slide_list.rs:266-303` — slides scroll container `<div class="operator__slides">`. Wrap in a `position: relative` parent so the floating elements can absolute-position over it.
- `OperatorState::new()` (in `crates/presenter-ui/src/state/operator.rs:58-60`) — reads `lineLimit` from localStorage. No change needed; the settings page just writes to the same key.
- `crates/presenter-server/src/ui/settings.rs:742+` — server-rendered settings page using `<section class="settings__card">` pattern.
- `crates/presenter-server/src/settings_script.js` — vanilla JS for the settings page; runs as IIFE at `(function () { ... })();`.
- CSS line-indicator at `operator.css:685-706` — `[data-drop-position="before"]::before` and `[data-drop-position="after"]::after` already render the 3px blue line.

**`presenter-ui` Cargo workspace:**
- Tests: `cd crates/presenter-ui && cargo test --target x86_64-unknown-linux-gnu --lib`
- Clippy: `cd crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings`
- Format: `cd crates/presenter-ui && cargo fmt`
- Workspace fmt check (root): `cargo fmt --all --check`

---

## File Structure

| File | Change |
|------|--------|
| `Cargo.toml` (workspace `[workspace.package]`) | `0.4.48` → `0.4.49` |
| `crates/presenter-ui/Cargo.toml` | `0.1.17` → `0.1.18` |
| `crates/presenter-ui/src/components/presentation_list.rs` | Add dragover/dragleave/drop handlers to the empty-state `<li>` (drop → insert at 0). Add head spacer + tail spacer `<li>` rendering inside the `if has_playlist { ... }` block. Both spacers reuse `handle_search_drop`. |
| `crates/presenter-ui/src/components/slide_list.rs` | Delete the `.operator__slides-toolbar` block (lines 238-260). Wrap the slides scroll container in `<div class="operator__slides-area">` with `position: relative`. Inside the wrapper, before the slides, add `<div class="operator__slides-bubble">` (draggable, top-left) and `<button class="operator__slides-add-floating">` (top-right). |
| `crates/presenter-ui/styles/operator.css` | Add `.operator__slides-area`, `.operator__slides-bubble`, `.operator__slides-add-floating`, `.operator__list-spacer` rules. Remove `.operator__slides-toolbar`, `.operator__line-limit`, `.operator__slides-add` rules (defunct). |
| `crates/presenter-server/src/ui/settings.rs` | Add a new "Preferences" `<section class="settings__card">` with the line-limit number input. |
| `crates/presenter-server/src/settings_script.js` | Add a small block that reads `localStorage["lineLimit"]` on load and writes on input. |
| `tests/e2e/wasm-drag-drop.spec.ts` | Add 4 new tests (empty playlist drop, head spacer, tail spacer, bubble drag from slides). |

---

## Task 1: Bump version 0.4.48 → 0.4.49

**Files:**
- Modify: `Cargo.toml`
- Modify: `crates/presenter-ui/Cargo.toml`

- [ ] **Step 1: Sync with remote**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
git fetch origin
git status -sb
```
Expected: clean working tree on `dev`.

- [ ] **Step 2: Bump workspace version**

In `/home/newlevel/devel/presenter/presenter-dev2/Cargo.toml` under `[workspace.package]`:
```toml
version = "0.4.49"
```
(was `0.4.48`).

- [ ] **Step 3: Bump presenter-ui version**

In `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui/Cargo.toml` under `[package]`:
```toml
version = "0.1.18"
```
(was `0.1.17`).

- [ ] **Step 4: Refresh both Cargo.lock files**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
cargo check -p presenter-server 2>&1 | tail -3
cd crates/presenter-ui && cargo check --target wasm32-unknown-unknown 2>&1 | tail -3 && cd ../..
```
Expected: both `Finished ...`.

- [ ] **Step 5: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
git add Cargo.toml Cargo.lock crates/presenter-ui/Cargo.toml crates/presenter-ui/Cargo.lock
git commit -m "chore: bump version to 0.4.49 (#272 #274)"
```

---

## Task 2: Add 4 failing E2E tests

**Files:**
- Modify: `tests/e2e/wasm-drag-drop.spec.ts`

- [ ] **Step 1: Append the 4 new tests inside the existing `test.describe("WASM Operator Drag-Drop", ...)` block, before its closing `});`**

Open `/home/newlevel/devel/presenter/presenter-dev2/tests/e2e/wasm-drag-drop.spec.ts`. The describe block contains the existing `"drag search result into specific position in open playlist (#274)"` test added in PR #282 commit `303feb6`. Append these 4 tests right before the describe's closing `});`:

```typescript
  // Edge case from #274 follow-up: dropping a search result on an
  // empty open playlist must insert at index 0.
  test("drag search result into empty playlist (#274 followup)", async ({
    page,
  }) => {
    const consoleMessages: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
      }
    });

    await initPage(page);

    // Find a playlist with zero entries. The fixtures may or may not have
    // one; if none exist, skip.
    const emptyPlaylist = page.evaluate(() => {
      const helpers = (window as any).__presenterOperatorTestHelpers;
      const playlists =
        (helpers?.listPlaylists && helpers.listPlaylists()) || [];
      const empty = playlists.find(
        (p: any) => Array.isArray(p.entries) && p.entries.length === 0,
      );
      return empty?.id ?? null;
    });
    const emptyPlaylistId = await emptyPlaylist;
    if (!emptyPlaylistId) {
      test.skip(true, "No empty playlists in fixtures");
      return;
    }

    // Click the empty playlist in the sidebar.
    await page
      .locator(`[data-role="playlist-item"][data-playlist-id="${emptyPlaylistId}"]`)
      .click();

    // Wait for the empty-state <li> to render.
    await expect(
      page.locator(
        '[data-view-panel="worship"] [data-role="presentation-empty-drop"]',
      ),
    ).toBeVisible({ timeout: 10_000 });

    // Search and drag the first presentation result onto the empty-state.
    const searchInput = page.locator('[data-role="global-search-input"]');
    await searchInput.fill("a");
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
    expect(draggedPresId).not.toBeNull();

    const emptyTarget = page.locator(
      '[data-view-panel="worship"] [data-role="presentation-empty-drop"]',
    );
    await searchResult.dragTo(emptyTarget);

    // Wait for the entries list to render with exactly 1 entry.
    await page.waitForFunction(
      () =>
        document.querySelectorAll(
          '[data-role="presentation-item"][data-entry-index]',
        ).length === 1,
      { timeout: 10_000 },
    );

    const firstEntryId = await page
      .locator('[data-role="presentation-item"][data-entry-index="0"]')
      .getAttribute("data-presentation-id");
    expect(firstEntryId).toBe(draggedPresId);
    expect(consoleMessages).toEqual([]);
  });

  // Edge case: dropping a search result on the head spacer above
  // entry 0 must insert at index 0.
  test("drag search result onto head spacer (#274 followup)", async ({
    page,
  }) => {
    const consoleMessages: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
      }
    });

    await initPage(page);

    const playlist = page.locator('[data-role="playlist-item"]').first();
    if ((await playlist.count()) === 0) {
      test.skip(true, "No playlists available");
      return;
    }
    await playlist.click();

    await page.waitForFunction(
      () =>
        document.querySelectorAll(
          '[data-role="presentation-item"][data-entry-index]',
        ).length >= 1,
      { timeout: 15_000 },
    );

    const entriesBefore = await page.evaluate(() =>
      Array.from(
        document.querySelectorAll(
          '[data-role="presentation-item"][data-entry-index]',
        ),
      ).map((el) => el.getAttribute("data-presentation-id")),
    );
    if (entriesBefore.length === 0) {
      test.skip(true, "Need at least 1 entry");
      return;
    }

    const searchInput = page.locator('[data-role="global-search-input"]');
    await searchInput.fill("a");
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

    const headSpacer = page.locator('[data-role="head-spacer"]');
    await expect(headSpacer).toBeAttached({ timeout: 5_000 });
    await searchResult.dragTo(headSpacer);

    await page.waitForFunction(
      (expected) =>
        document.querySelectorAll(
          '[data-role="presentation-item"][data-entry-index]',
        ).length === expected,
      entriesBefore.length + 1,
      { timeout: 10_000 },
    );

    const firstEntryId = await page
      .locator('[data-role="presentation-item"][data-entry-index="0"]')
      .getAttribute("data-presentation-id");
    expect(firstEntryId).toBe(draggedPresId);
    expect(consoleMessages).toEqual([]);
  });

  // Edge case: dropping a search result on the tail spacer below
  // the last entry must insert at the END.
  test("drag search result onto tail spacer (#274 followup)", async ({
    page,
  }) => {
    const consoleMessages: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
      }
    });

    await initPage(page);

    const playlist = page.locator('[data-role="playlist-item"]').first();
    if ((await playlist.count()) === 0) {
      test.skip(true, "No playlists available");
      return;
    }
    await playlist.click();

    await page.waitForFunction(
      () =>
        document.querySelectorAll(
          '[data-role="presentation-item"][data-entry-index]',
        ).length >= 1,
      { timeout: 15_000 },
    );

    const entriesBefore = await page.evaluate(() =>
      Array.from(
        document.querySelectorAll(
          '[data-role="presentation-item"][data-entry-index]',
        ),
      ).map((el) => el.getAttribute("data-presentation-id")),
    );
    if (entriesBefore.length === 0) {
      test.skip(true, "Need at least 1 entry");
      return;
    }

    const searchInput = page.locator('[data-role="global-search-input"]');
    await searchInput.fill("a");
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

    const tailSpacer = page.locator('[data-role="tail-spacer"]');
    await expect(tailSpacer).toBeAttached({ timeout: 5_000 });
    await searchResult.dragTo(tailSpacer);

    await page.waitForFunction(
      (expected) =>
        document.querySelectorAll(
          '[data-role="presentation-item"][data-entry-index]',
        ).length === expected,
      entriesBefore.length + 1,
      { timeout: 10_000 },
    );

    const lastIndex = entriesBefore.length;
    const lastEntryId = await page
      .locator(
        `[data-role="presentation-item"][data-entry-index="${lastIndex}"]`,
      )
      .getAttribute("data-presentation-id");
    expect(lastEntryId).toBe(draggedPresId);
    expect(consoleMessages).toEqual([]);
  });

  // Issue #272: the floating song-name bubble in the slides area is
  // draggable; dropping it onto a playlist entry must insert at that
  // position, just like a search-result drag.
  test("drag song bubble from slides into playlist position (#272)", async ({
    page,
  }) => {
    const consoleMessages: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
      }
    });

    await initPage(page);

    // Open a library and click the first presentation to populate the
    // slides area (which renders the bubble).
    await page.locator('[data-role="library-item"]').first().click();
    await page.waitForSelector('[data-role="presentation-item"]', {
      timeout: 15_000,
    });
    await page.locator('[data-role="presentation-item"]').first().click();

    // The floating song bubble should appear once a presentation is
    // selected.
    const bubble = page.locator('[data-role="slides-song-bubble"]');
    await expect(bubble).toBeVisible({ timeout: 10_000 });
    const bubblePresId = await bubble.getAttribute("data-presentation-id");
    expect(bubblePresId).not.toBeNull();

    // Now click a playlist with at least 1 entry.
    const playlist = page.locator('[data-role="playlist-item"]').first();
    if ((await playlist.count()) === 0) {
      test.skip(true, "No playlists available");
      return;
    }
    await playlist.click();
    await page.waitForFunction(
      () =>
        document.querySelectorAll(
          '[data-role="presentation-item"][data-entry-index]',
        ).length >= 1,
      { timeout: 15_000 },
    );

    const entriesBefore = await page.evaluate(() =>
      Array.from(
        document.querySelectorAll(
          '[data-role="presentation-item"][data-entry-index]',
        ),
      ).map((el) => el.getAttribute("data-presentation-id")),
    );

    // Drag the bubble onto entry index 0 (top half → insert before).
    const targetEntry = page.locator(
      '[data-role="presentation-item"][data-entry-index="0"]',
    );
    await bubble.dragTo(targetEntry, { targetPosition: { x: 50, y: 5 } });

    await page.waitForFunction(
      (expected) =>
        document.querySelectorAll(
          '[data-role="presentation-item"][data-entry-index]',
        ).length === expected,
      entriesBefore.length + 1,
      { timeout: 10_000 },
    );

    const firstEntryId = await page
      .locator('[data-role="presentation-item"][data-entry-index="0"]')
      .getAttribute("data-presentation-id");
    expect(firstEntryId).toBe(bubblePresId);
    expect(consoleMessages).toEqual([]);
  });
```

The tests reference 4 new selectors that don't exist yet on the dev server:
- `[data-role="presentation-empty-drop"]` (Task 3 adds it)
- `[data-role="head-spacer"]` (Task 3 adds it)
- `[data-role="tail-spacer"]` (Task 3 adds it)
- `[data-role="slides-song-bubble"]` (Task 4 adds it)

Until the implementation tasks deploy, the tests fail.

- [ ] **Step 2: Verify the new tests are listed**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
npx playwright test wasm-drag-drop.spec.ts --list 2>&1 | grep -E "empty playlist|head spacer|tail spacer|song bubble" | head
```
Expected: 4 lines listing the new tests.

- [ ] **Step 3: Run them against the dev server (they should fail)**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
PRESENTER_E2E_BASE_URL=http://10.77.8.134:8080 npx playwright test wasm-drag-drop.spec.ts -g "(#274 followup)|(#272)" --reporter=line 2>&1 | tail -25
```
Expected: 4 failures or skips. The 4 selectors don't exist, so the tests time out waiting for them.

- [ ] **Step 4: Commit the failing tests**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
git add tests/e2e/wasm-drag-drop.spec.ts
git commit -m "test(e2e): regression tests for #274 edge cases + #272 bubble drag

Adds 4 new tests upfront (TDD red):
- empty playlist drop (#274 followup)
- head spacer drop above first entry (#274 followup)
- tail spacer drop below last entry (#274 followup)
- floating song bubble drag from slides into playlist (#272)

Tests fail on the current code because the four selectors
([data-role=presentation-empty-drop|head-spacer|tail-spacer|
slides-song-bubble]) don't exist yet. Tasks 3 and 4 add them."
```

---

## Task 3: Bug fixes — empty-playlist drop + head/tail spacers

**Files:**
- Modify: `crates/presenter-ui/src/components/presentation_list.rs`
- Modify: `crates/presenter-ui/styles/operator.css`

This task makes E2E tests 1 / 2 / 3 pass.

- [ ] **Step 1: Add an empty-state drop helper at the top of `presentation_list.rs`**

Open `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui/src/components/presentation_list.rs`. Find the existing `take_drop_position` helper (around line 35-43). Just BELOW the existing `handle_search_drop` function (around line 115), add:

```rust
/// Insert at a fixed position. Used by the head spacer (target_index=0,
/// drop_position="before"), the tail spacer (target_index=entries.len(),
/// drop_position="before"), and the empty-state placeholder
/// (target_index=0, drop_position="before"). Reads the dragged
/// presentation id from the dataTransfer and calls replace_entries with
/// the new presentation inserted at insert_idx. Shows success/error
/// toast.
fn handle_search_drop_at_fixed(
    ev: &web_sys::DragEvent,
    insert_idx: usize,
    playlist_id: String,
    selected_playlist: RwSignal<Option<presenter_core::Playlist>>,
    playlists: RwSignal<Vec<presenter_core::Playlist>>,
    toast_message: RwSignal<Option<String>>,
    toast_variant: RwSignal<String>,
) {
    // Clear any data-drop-position attribute we may have set during dragover.
    if let Some(target) = ev
        .current_target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
    {
        let _ = target.remove_attribute("data-drop-position");
    }

    let presentation_id = ev
        .data_transfer()
        .and_then(|dt| dt.get_data("application/x-presentation-id").ok())
        .filter(|s| !s.is_empty());

    let Some(presentation_id) = presentation_id else {
        toast_variant.set("error".to_string());
        toast_message.set(Some("Drag payload missing presentation id".to_string()));
        return;
    };

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
                    toast_message.set(Some("Added presentation to playlist".to_string()));
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
```

- [ ] **Step 2: Replace the empty-state `<li>` with a drop-target version**

In the same file, find the empty-state branch (currently around line 264-268):

```rust
                            if playlist.entries.is_empty() {
                                return view! {
                                    <li class="empty">"Playlist is empty. Drag songs from a library or add a separator."</li>
                                }.into_any();
                            }
```

Replace with:

```rust
                            if playlist.entries.is_empty() {
                                let playlist_id = ctx.selected_playlist_id.get_untracked().unwrap_or_default();
                                let selected_playlist = ctx.selected_playlist;
                                let playlists = ctx.playlists;
                                let toast_message = ctx.toast_message;
                                let toast_variant = ctx.toast_variant;
                                let op_for_dragover = op.clone();
                                let op_for_dragleave = op.clone();
                                let op_for_drop = op.clone();
                                let pl_id_for_drop = playlist_id.clone();
                                return view! {
                                    <li
                                        class="empty operator__list-empty-drop"
                                        data-role="presentation-empty-drop"
                                        on:dragover=move |ev: web_sys::DragEvent| {
                                            if !op_for_dragover.dragging_from_search.get_untracked() {
                                                return;
                                            }
                                            ev.prevent_default();
                                            if let Some(target) = ev
                                                .current_target()
                                                .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
                                            {
                                                let _ = target.set_attribute("data-drop-position", "before");
                                            }
                                        }
                                        on:dragleave=move |ev: web_sys::DragEvent| {
                                            if !op_for_dragleave.dragging_from_search.get_untracked() {
                                                return;
                                            }
                                            if let Some(target) = ev
                                                .current_target()
                                                .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
                                            {
                                                let _ = target.remove_attribute("data-drop-position");
                                            }
                                        }
                                        on:drop=move |ev: web_sys::DragEvent| {
                                            ev.prevent_default();
                                            if op_for_drop.dragging_from_search.get_untracked() {
                                                handle_search_drop_at_fixed(
                                                    &ev,
                                                    0,
                                                    pl_id_for_drop.clone(),
                                                    selected_playlist,
                                                    playlists,
                                                    toast_message,
                                                    toast_variant,
                                                );
                                                op_for_drop.dragging_from_search.set(false);
                                                op_for_drop.search_dragging.set(false);
                                            }
                                        }
                                    >
                                        "Playlist is empty. Drag songs from a library or add a separator."
                                    </li>
                                }.into_any();
                            }
```

- [ ] **Step 3: Render head + tail spacers around the entries**

Find where `playlist.entries.iter().enumerate().map(...)` is iterated (around line 269). The current code returns the iterator directly. Wrap it so a head spacer comes first, the entries come second, and a tail spacer comes last.

Specifically, change:

```rust
                            return playlist.entries.iter().enumerate().map(|(idx, entry)| {
                                ... (current entry rendering)
                            }).collect::<Vec<_>>().into_any();
```

(or whatever the exact `.collect()` shape is) to:

```rust
                            let entries_view: Vec<_> = playlist.entries.iter().enumerate().map(|(idx, entry)| {
                                ... (current entry rendering — UNCHANGED)
                            }).collect();
                            let entries_len = playlist.entries.len();
                            let playlist_id_spacer = ctx.selected_playlist_id.get_untracked().unwrap_or_default();
                            let selected_playlist = ctx.selected_playlist;
                            let playlists = ctx.playlists;
                            let toast_message = ctx.toast_message;
                            let toast_variant = ctx.toast_variant;
                            let head_view = render_list_spacer(
                                "head-spacer",
                                0,
                                op.clone(),
                                playlist_id_spacer.clone(),
                                selected_playlist,
                                playlists,
                                toast_message,
                                toast_variant,
                            );
                            let tail_view = render_list_spacer(
                                "tail-spacer",
                                entries_len,
                                op.clone(),
                                playlist_id_spacer,
                                selected_playlist,
                                playlists,
                                toast_message,
                                toast_variant,
                            );
                            return view! {
                                {head_view}
                                {entries_view}
                                {tail_view}
                            }.into_any();
```

(The exact wrapping syntax depends on what's currently there — the implementer should look at the existing return statement and preserve its structure. The key change is: ONE head spacer before the entries, the entries unchanged, ONE tail spacer after.)

- [ ] **Step 4: Add the `render_list_spacer` helper**

In `presentation_list.rs`, just below `handle_search_drop_at_fixed`, add:

```rust
/// Render a transparent ~16px-tall <li> that captures search-drag dragover
/// in the dead zone above the first entry (head) or below the last entry
/// (tail). On drop, inserts at the fixed insert_idx using
/// handle_search_drop_at_fixed.
#[allow(clippy::too_many_arguments)]
fn render_list_spacer(
    role: &'static str,
    insert_idx: usize,
    op: OperatorState,
    playlist_id: String,
    selected_playlist: RwSignal<Option<presenter_core::Playlist>>,
    playlists: RwSignal<Vec<presenter_core::Playlist>>,
    toast_message: RwSignal<Option<String>>,
    toast_variant: RwSignal<String>,
) -> impl IntoView {
    let op_for_dragover = op.clone();
    let op_for_dragleave = op.clone();
    let op_for_drop = op.clone();
    let pl_id_for_drop = playlist_id;
    // Head spacer wants the line at its bottom (visually = before entry 0)
    // → "after". Tail spacer wants the line at its top (visually = below
    // last entry) → "before".
    let drop_side = if role == "head-spacer" { "after" } else { "before" };
    view! {
        <li
            class="operator__list-spacer"
            data-role=role
            on:dragover=move |ev: web_sys::DragEvent| {
                if !op_for_dragover.dragging_from_search.get_untracked() {
                    return;
                }
                ev.prevent_default();
                if let Some(target) = ev
                    .current_target()
                    .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
                {
                    let _ = target.set_attribute("data-drop-position", drop_side);
                }
            }
            on:dragleave=move |ev: web_sys::DragEvent| {
                if !op_for_dragleave.dragging_from_search.get_untracked() {
                    return;
                }
                if let Some(target) = ev
                    .current_target()
                    .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
                {
                    let _ = target.remove_attribute("data-drop-position");
                }
            }
            on:drop=move |ev: web_sys::DragEvent| {
                ev.prevent_default();
                if op_for_drop.dragging_from_search.get_untracked() {
                    handle_search_drop_at_fixed(
                        &ev,
                        insert_idx,
                        pl_id_for_drop.clone(),
                        selected_playlist,
                        playlists,
                        toast_message,
                        toast_variant,
                    );
                    op_for_drop.dragging_from_search.set(false);
                    op_for_drop.search_dragging.set(false);
                }
            }
        >
        </li>
    }
}
```

- [ ] **Step 5: Add CSS for the spacers and empty-drop state**

In `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui/styles/operator.css`, add at the end of the file (or in a logical location near `.operator__presentation-item[data-drop-position]`):

```css
/* #274 follow-up: head/tail spacers and empty-state drop target */
.operator__list-spacer {
    list-style: none;
    margin: 0;
    padding: 0;
    height: 16px;
    background: transparent;
    border: 0;
    position: relative;
}

.operator__list-spacer[data-drop-position="before"]::before,
.operator__list-spacer[data-drop-position="after"]::after {
    content: "";
    position: absolute;
    left: 8px;
    right: 8px;
    height: 3px;
    background: rgba(59, 124, 255, 0.85);
    border-radius: 2px;
    pointer-events: none;
}

.operator__list-spacer[data-drop-position="before"]::before {
    top: -2px;
}

.operator__list-spacer[data-drop-position="after"]::after {
    bottom: -2px;
}

.operator__list-empty-drop[data-drop-position="before"]::before {
    content: "";
    position: absolute;
    left: 8px;
    right: 8px;
    height: 3px;
    background: rgba(59, 124, 255, 0.85);
    border-radius: 2px;
    pointer-events: none;
    bottom: -2px;
}

.operator__list-empty-drop {
    position: relative;
}
```

- [ ] **Step 6: Verify it compiles**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui
cargo check --target wasm32-unknown-unknown 2>&1 | tail -10
```

Expected: clean. Common errors:
- `cannot find type RwSignal in scope` → ensure `use leptos::prelude::*;` is at the top of the file.
- `presenter_core::Playlist not found` → the existing `handle_search_drop` already references this; copy whatever import alias it uses.
- Double `op.clone()` errors → re-check the empty-state edit; the closures all need their own clone of `op` BEFORE the `move` keyword.

- [ ] **Step 7: Clippy + fmt**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui
cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings 2>&1 | tail -8
cargo fmt
```

Expected: clean.

- [ ] **Step 8: Run UI lib tests**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui
cargo test --target x86_64-unknown-linux-gnu --lib 2>&1 | tail -10
```

Expected: pass (no test added; this is a sanity check).

- [ ] **Step 9: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
git add crates/presenter-ui/src/components/presentation_list.rs \
        crates/presenter-ui/styles/operator.css
git commit -m "fix(ui): empty-playlist drop + head/tail spacers (#274 followup)

Three drag-drop edge cases reported after PR #282 merged:

1. Empty playlist: the existing 'Playlist is empty…' <li> now has
   dragover/drop handlers that insert at index 0 on search drag.
2. Drop above first entry: a transparent 16px head-spacer <li>
   captures the dragover dead zone above entry 0 and inserts at 0.
3. Drop below last entry: a tail-spacer <li> captures the dragover
   dead zone below the last entry and inserts at entries.len().

All three reuse a new handle_search_drop_at_fixed helper that
shares logic with handle_search_drop (PR #282) — only the insert
index is hardcoded per call site. The existing CSS line indicator
fires on the spacers via data-drop-position. New CSS for
.operator__list-spacer and .operator__list-empty-drop."
```

---

## Task 4: Floating bubble + "+" + remove toolbar

**Files:**
- Modify: `crates/presenter-ui/src/components/slide_list.rs`
- Modify: `crates/presenter-ui/styles/operator.css`

This task makes E2E test 4 pass.

- [ ] **Step 1: Delete the existing slides toolbar**

Open `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui/src/components/slide_list.rs`. Find the `<div class="operator__slides-toolbar">` block (around lines 238-260):

```rust
            <div class="operator__slides-toolbar">
                <label class="operator__line-limit" title="Maximum characters per line">
                    <span>"Line limit"</span>
                    <input
                        type="number"
                        min="10"
                        max="120"
                        step="1"
                        data-role="line-limit"
                        prop:value=move || op.line_limit.get().to_string()
                        on:input=on_line_limit_change
                    />
                </label>
                <button
                    type="button"
                    class="operator__slides-add"
                    data-role="add-slide"
                    title="Add slide"
                    on:click=add_slide
                >
                    "+"
                </button>
            </div>
```

DELETE the entire `<div class="operator__slides-toolbar">...</div>` block.

After deletion, also DELETE the `on_line_limit_change` closure (around line 227-232) since the line-limit input is gone:

```rust
    let on_line_limit_change = move |ev| {
        let val = event_target_value(&ev);
        if let Ok(n) = val.parse::<u32>() {
            op.line_limit.set(n);
        }
    };
```

But KEEP the `add_slide` closure at line 209 — the new floating "+" button will call it.

- [ ] **Step 2: Wrap the slides scroll container in a positioned area + add the two floating elements**

Find where `<div class="operator__slides" data-role="slides" ...>` starts (around line 266). The current structure is:

```rust
            {
                // Clone op for each handler that moves it into a closure
                let op_dragover = op.clone();
                let op_drop = op.clone();
                view! {
                    <div
                        class="operator__slides"
                        data-role="slides"
                        on:dragover=move |ev: web_sys::DragEvent| { ... }
                        on:drop=move |ev: web_sys::DragEvent| { ... }
                    >
                        // ... slides content ...
                    </div>
                }
            }
```

Wrap this in a new `<div class="operator__slides-area">` and insert the floating elements BEFORE the `.operator__slides` element. Replace the block above with:

```rust
            <div class="operator__slides-area">
                <SlidesFloatingBubble />
                <SlidesFloatingAdd add_slide=add_slide.clone() />
                {
                    // Clone op for each handler that moves it into a closure
                    let op_dragover = op.clone();
                    let op_drop = op.clone();
                    view! {
                        <div
                            class="operator__slides"
                            data-role="slides"
                            on:dragover=move |ev: web_sys::DragEvent| { /* unchanged */ }
                            on:drop=move |ev: web_sys::DragEvent| { /* unchanged */ }
                        >
                            // ... slides content unchanged ...
                        </div>
                    }
                }
            </div>
```

(Keep the existing dragover/drop handler bodies and slides content exactly as they are — only the wrapping changes.)

Note: `add_slide` is a `move` closure declared at line 209. To pass it to a child component, you may need to make it `Rc<dyn Fn>` or capture-by-reference. **Simpler approach: don't introduce child components — inline the bubble and "+" markup** directly inside the new `<div class="operator__slides-area">`. Skip Step 3's child components and instead inline:

```rust
            <div class="operator__slides-area">
                {
                    let ctx_for_bubble = ctx;
                    let op_for_bubble_drag = op.clone();
                    let op_for_bubble_end = op.clone();
                    move || {
                        let presentation = ctx_for_bubble.selected_presentation.get();
                        let Some(pres) = presentation else {
                            return view! { <div class="operator__slides-bubble" data-role="slides-song-bubble" data-empty="true"></div> }.into_any();
                        };
                        let pres_id = pres.id.to_string();
                        let pres_id_drag = pres_id.clone();
                        let pres_name = pres.name.clone();
                        let op_drag = op_for_bubble_drag.clone();
                        let op_end = op_for_bubble_end.clone();
                        view! {
                            <div
                                class="operator__slides-bubble"
                                data-role="slides-song-bubble"
                                data-presentation-id=pres_id.clone()
                                draggable="true"
                                title="Drag into a playlist"
                                on:dragstart=move |ev: web_sys::DragEvent| {
                                    if let Some(dt) = ev.data_transfer() {
                                        let _ = dt.set_data("text/plain", &pres_id_drag);
                                        let _ = dt.set_data("application/x-presentation-id", &pres_id_drag);
                                        dt.set_effect_allowed("copy");
                                    }
                                    op_drag.search_dragging.set(true);
                                    op_drag.dragging_from_search.set(true);
                                }
                                on:dragend=move |_| {
                                    op_end.search_dragging.set(false);
                                    op_end.dragging_from_search.set(false);
                                }
                            >
                                <span class="operator__slides-bubble-name">{pres_name}</span>
                            </div>
                        }.into_any()
                    }
                }
                {
                    let add_slide_for_btn = add_slide.clone();
                    let ctx_for_btn = ctx;
                    move || {
                        let visible = ctx_for_btn.selected_presentation.with(|p| p.is_some());
                        if !visible {
                            return view! { <></> }.into_any();
                        }
                        view! {
                            <button
                                type="button"
                                class="operator__slides-add-floating"
                                data-role="add-slide"
                                title="Add slide"
                                on:click=add_slide_for_btn.clone()
                            >
                                "+"
                            </button>
                        }.into_any()
                    }
                }
                {
                    // existing slides scroll container UNCHANGED
                    let op_dragover = op.clone();
                    let op_drop = op.clone();
                    view! {
                        <div
                            class="operator__slides"
                            data-role="slides"
                            // ... existing handlers unchanged ...
                        >
                            // ... existing children unchanged ...
                        </div>
                    }
                }
            </div>
```

(The `add_slide` closure may need to be cloned via `let add_slide = std::rc::Rc::new(add_slide);` near the top so it can be shared across renders — depends on Leptos's exact closure rules. If `add_slide` is `Copy`, `.clone()` is a no-op. Otherwise wrap in `Rc<dyn Fn(_)>` and clone the Rc.)

If the inline-closure approach causes Leptos compile errors, fall back to extracting two child components `SlidesFloatingBubble` and `SlidesFloatingAdd` that take `add_slide_callback: Callback<()>` as a prop. Choose whichever compiles cleanly.

- [ ] **Step 3: Add the floating CSS**

In `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui/styles/operator.css`, REMOVE the existing rules for `.operator__slides-toolbar` (around line 755), `.operator__line-limit` (lines 764-796), and `.operator__slides-add` (lines 1052-1064). Then ADD:

```css
/* #272: floating bubble + add button over the slides area */
.operator__slides-area {
    position: relative;
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
}

.operator__slides-bubble {
    position: absolute;
    top: 8px;
    left: 8px;
    z-index: 10;
    display: inline-flex;
    align-items: center;
    padding: 6px 14px;
    border-radius: 999px;
    background: rgba(20, 28, 50, 0.92);
    color: #fff;
    font-size: 13px;
    font-weight: 600;
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.3);
    cursor: grab;
    user-select: none;
    max-width: 60%;
    overflow: hidden;
    pointer-events: auto;
}

.operator__slides-bubble[data-empty="true"] {
    display: none;
}

.operator__slides-bubble:active {
    cursor: grabbing;
}

.operator__slides-bubble-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
}

.operator__slides-add-floating {
    position: absolute;
    top: 8px;
    right: 8px;
    z-index: 10;
    width: 36px;
    height: 36px;
    border-radius: 50%;
    border: none;
    background: rgba(59, 124, 255, 0.9);
    color: #fff;
    font-size: 22px;
    font-weight: 700;
    line-height: 1;
    cursor: pointer;
    box-shadow: 0 2px 6px rgba(0, 0, 0, 0.25);
    pointer-events: auto;
}

.operator__slides-add-floating:hover {
    background: rgba(59, 124, 255, 1);
}
```

- [ ] **Step 4: Verify compile + clippy**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui
cargo check --target wasm32-unknown-unknown 2>&1 | tail -10
cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings 2>&1 | tail -5
cargo fmt
```

Expected: clean.

- [ ] **Step 5: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
git add crates/presenter-ui/src/components/slide_list.rs \
        crates/presenter-ui/styles/operator.css
git commit -m "feat(ui): floating song bubble + add button over slides (#272)

Replace the .operator__slides-toolbar block with two absolutely-
positioned elements over the slides scroll area:

- Top-left: draggable .operator__slides-bubble showing the active
  presentation name. dragstart sets application/x-presentation-id
  + op.dragging_from_search, so the playlist drop infrastructure
  from PR #282 (and the empty/spacer drops added today) handles
  it for free.
- Top-right: .operator__slides-add-floating circular '+' button
  that calls the existing add_slide handler. Same data-role as
  before so existing E2E coverage holds.

Both elements only appear when a presentation is selected. The
slides area gains ~50px vertical height previously occupied by
the toolbar."
```

---

## Task 5: Move "Line limit" input to /ui/settings

**Files:**
- Modify: `crates/presenter-server/src/ui/settings.rs`
- Modify: `crates/presenter-server/src/settings_script.js`

- [ ] **Step 1: Add a "Preferences" section to the settings page**

Open `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-server/src/ui/settings.rs`. Find a good place to insert a new `<section class="settings__card">` — near the top of the `<main class="settings__main">` block (around line 168, after the Companion section closes around line 204) is a good fit.

Insert this block:

```rust
                    <section class="settings__card" data-role="preferences-card">
                        <header class="settings__card-header">
                            <div>
                                <h2>"Preferences"</h2>
                                <p class="settings__card-sub">
                                    "Operator-side settings stored in your browser."
                                </p>
                            </div>
                        </header>
                        <form class="settings__form settings__form--compact" autocomplete="off">
                            <div class="settings__form-row settings__form-row--compact settings__form-row--inline">
                                <label class="settings__form-control--tiny">
                                    <span>"Line limit (chars per line)"</span>
                                    <input
                                        type="number"
                                        min="10"
                                        max="120"
                                        step="1"
                                        value="32"
                                        data-role="pref-line-limit"
                                    />
                                </label>
                            </div>
                            <p class="settings__hint">
                                "Slides with longer lines show a warning marker. Reload the operator after changing."
                            </p>
                        </form>
                    </section>
```

- [ ] **Step 2: Add the JS that reads/writes localStorage["lineLimit"]**

Open `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-server/src/settings_script.js`. Find a good spot — near the end of the `(function () { ... })();` IIFE (just before its closing `})();`).

Add:

```javascript
  // #272: line-limit preference (operator-side, persisted in localStorage).
  (function bindLineLimitPref() {
    const input = document.querySelector('[data-role="pref-line-limit"]');
    if (!input) return;
    const stored = localStorage.getItem('lineLimit');
    if (stored && /^\d+$/.test(stored)) {
      input.value = stored;
    }
    input.addEventListener('input', () => {
      const raw = input.value.trim();
      if (!/^\d+$/.test(raw)) return;
      const n = Number(raw);
      if (n < 10 || n > 120) return;
      localStorage.setItem('lineLimit', String(n));
    });
  })();
```

- [ ] **Step 3: Verify the settings page renders**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
cargo check -p presenter-server 2>&1 | tail -5
```

Expected: clean. Visit `http://10.77.8.134:8080/ui/settings` after deploy and confirm the new "Preferences" card is visible with the number input.

- [ ] **Step 4: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
git add crates/presenter-server/src/ui/settings.rs \
        crates/presenter-server/src/settings_script.js
git commit -m "feat(settings): line limit preference moved from operator toolbar (#272)

Adds a 'Preferences' card to /ui/settings with a number input for
the operator's line-limit setting (default 32; range 10-120). Value
is persisted in localStorage['lineLimit'], same key the operator's
OperatorState reads on init. Reload the operator UI for the new
value to take effect (acceptable — line limit changes are rare)."
```

---

## Task 6: Local checks + push + monitor + open PR

This task is controller-handled (NOT a subagent dispatch).

- [ ] **Step 1: Final local sanity sweep**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
cargo fmt --all --check && echo "WORKSPACE FMT OK"
cd crates/presenter-ui && cargo fmt --check && echo "UI FMT OK" && cd ../..
cd crates/presenter-ui && cargo test --target x86_64-unknown-linux-gnu --lib 2>&1 | tail -5 && cd ../..
cd crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings 2>&1 | tail -3 && cd ../..
```

Expected: all four commands succeed.

- [ ] **Step 2: Push**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
git push origin dev
```

- [ ] **Step 3: Identify and monitor the new pipeline**

```bash
gh run list --branch dev --limit 2 --json databaseId,name,status,headSha
```

Then in the background:

```bash
sleep 600 && gh run view <DATABASE_ID> --json status,conclusion,jobs --jq '{status, conclusion, pending: [.jobs[] | select(.status!="completed") | .name], failed: [.jobs[] | select(.conclusion=="failure") | .name]}'
```

Re-poll every 10-15 minutes until terminal.

- [ ] **Step 4: Verify all jobs SUCCESS**

```bash
gh run view <DATABASE_ID> --json status,conclusion,jobs --jq '{status, conclusion, allSuccess: ([.jobs[] | .conclusion=="success"] | all)}'
```

If any job failed, `gh run view --log-failed`, fix in ONE commit, push, monitor again.

- [ ] **Step 5: Verify dev deploy is live and shows v0.4.49**

```bash
curl -s http://10.77.8.134:8080/healthz
```

Expected: `{"channel":"dev","status":"ok","version":"0.4.49"}`. Then open the operator in Playwright and:
- Confirm the bubble appears at the top-left when a presentation is selected.
- Confirm the "+" button appears at the top-right.
- Confirm the original toolbar is GONE.
- Pick an empty playlist (or create one), drag a search result onto the empty-state — should insert.
- Open a non-empty playlist, drag onto the head/tail dead zones — line indicator should appear, drop should land at 0/end.
- Open `/ui/settings` and confirm the Preferences card with the line-limit input shows up.

- [ ] **Step 6: Open the PR**

```bash
gh pr list --base main --head dev --json number --jq 'length'
```

If 0:

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
gh pr create --base main --head dev --title "fix+feat(ui): playlist drop edge cases + floating slides bubble (#272 #274)" --body "$(cat <<'EOF'
## Summary

Fixes 3 drag-drop edge cases reported by the user after PR #282 merged (issue #274):
- Drop on empty playlist now inserts at index 0
- Drop above the first entry inserts at index 0 (was: appended to end)
- Drop below the last entry inserts at end (was: required exact aim inside the bottom half of the last entry)

Plus implements issue #272: replaces the slides toolbar with a floating song-name bubble (top-left, draggable into playlists) and a floating + add-slide button (top-right). Line-limit input moves to /ui/settings under a new Preferences card.

## What changed

- presentation_list.rs: empty-state \`<li>\` is now a drop target; new transparent head/tail spacers cover the dead zones above/below the entry list.
- slide_list.rs: removed \`.operator__slides-toolbar\`; added \`[data-role=slides-song-bubble]\` (draggable, sets \`application/x-presentation-id\`) and floating \`[data-role=add-slide]\` button (same data-role as before, no E2E breakage).
- /ui/settings: new Preferences card with the line-limit input, persisted via \`localStorage[\"lineLimit\"]\`.
- 4 new E2E tests close the verification gap that let the original edge cases slip through.

## Test plan

- [x] 4 new Playwright tests pass (empty/head-spacer/tail-spacer/bubble drag)
- [x] Existing within-playlist reorder + middle-position search drop tests still pass
- [x] Bubble visible only when a presentation is selected
- [x] /ui/settings → Preferences input round-trips through localStorage

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

Use `gh api repos/zbynekdrlik/presenter/pulls/<NUMBER> --jq '{mergeable, mergeable_state}'` to confirm `clean`.

- [ ] **Step 7: /plan-check + /review pre-completion gate**

Run both. Fix any 🔴 / 🟡 / 🔵 inside the diff before sending the completion report.

- [ ] **Step 8: Send completion report**

Use the EXACT template from `~/devel/airuleset/modules/core/completion-report.md`.

---

## Verification Summary

| Check | How to verify |
|-------|---------------|
| Empty playlist drop works | E2E test 1 + manual: open empty playlist, drag search result, lands at index 0 |
| Drop above first entry works | E2E test 2 + manual: drop on the head dead zone, lands at index 0 |
| Drop below last entry works | E2E test 3 + manual: drop on the tail dead zone, lands at end |
| Bubble drag from slides into playlist | E2E test 4 + manual: drag the top-left bubble, drop on entry index 1 |
| Floating "+" button still adds slides | Existing E2E using `[data-role="add-slide"]` keeps passing |
| Line limit moves to /ui/settings | Visit /ui/settings; Preferences card with line-limit input visible |
| Original slides toolbar removed | Operator UI shows no `.operator__slides-toolbar` element |
| Dev shows v0.4.49 | `/healthz` and DOM both report 0.4.49 |
| All 20 CI jobs green | `gh run view <id>` reports `allSuccess: true` |
