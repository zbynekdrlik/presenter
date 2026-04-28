# Worship-PP Follow-ups Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix two follow-up issues after worship-pp regression PR — drag from search to playlist doesn't add an entry, and the stage worship-pp sidebar wastes space and is unreadable from a distance.

**Architecture:** (1) Investigate the search-drag failure live in Playwright, identify root cause, apply targeted fix in `playlist_list.rs` (most likely `set_drop_effect("copy")` in the dragover handler). (2) Pure CSS resize of the worship-pp layout in `stage.css` — sidebar 30%→22%, slides 70%→78%, entry font-size 0.9vw→2.6vh, padding rebalanced so 12 entries fit in 92vh.

**Tech Stack:** Rust + Leptos 0.7 (WASM), CSS, Playwright/TypeScript.

**Spec:** `docs/superpowers/specs/2026-04-28-worship-pp-followups-design.md` (commit `3d04457`).

---

## Context

Previous PR #268 ("worship-pp baseline + drag-drop fixes + three layout regressions") is mergeable & clean awaiting user merge. This plan stacks new work on `dev` on top of those commits — they'll either land in PR #268 (if not yet merged) or in a fresh PR (if merged).

`presenter-ui` is excluded from the root workspace and has its OWN `Cargo.lock`. Tests run via `cargo test -p presenter-ui --target x86_64-unknown-linux-gnu`. Clippy via `cd crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown -- -D warnings -W clippy::all`. Local Rust builds are allowed on this dev machine.

The dev server is at `http://10.77.8.134:8080`. The dev DB is replaced from prod on every deploy by design — that's intentional and out of scope.

### Existing data-role attributes (relevant for E2E)

- `[data-role="global-search-query"]` — search input in `header.rs:163`
- `[data-role="search-result-item"]` — search result `<div>` in `search.rs:299`
- `[data-role="playlist-list"]` — operator's dashboard playlist `<ul>`
- `[data-role="playlist-item"]` — playlist row `<button>` inside the `<li>` (the `<li>` itself carries `data-playlist-id` and the on:drop handler)
- `[data-role="presentation-item"]` — presentation row in operator's right column

### Existing drag-drop hooks (for issue 1 investigation)

- Search dragstart (`search.rs:307-317`): sets `application/x-presentation-id` AND `application/x-presenter-search` on dataTransfer; sets `effectAllowed = "copy"`.
- Playlist row dragover (`playlist_list.rs:137-158`): reads `dataTransfer.types`, accepts the two MIME types above, calls `prevent_default`. **Does NOT call `set_drop_effect("copy")`** — this is the most likely cause.
- Playlist row drop (`playlist_list.rs:169-234`): reads `application/x-presentation-id` (or x-presenter-search fallback), spawns `replace_entries` async task.

---

## File Structure

| File | Responsibility |
|------|----------------|
| `crates/presenter-ui/src/components/playlist_list.rs` | Add `set_drop_effect("copy")` in the dragover handler (or other targeted fix once root cause is confirmed). |
| `crates/presenter-ui/styles/stage.css` | Resize `.stage-pp__slides-area` (78%) and `.stage-pp__playlist-sidebar` (22%); enlarge `.stage-pp__playlist-entry` and re-tune `.stage-pp__playlist-entry--active`. |
| `tests/e2e/wasm-playlist-operations.spec.ts` | New "drag from search result to playlist" E2E. |
| `tests/e2e/stage-worship-pp.spec.ts` | Extended assertions for sidebar width and entry font-size. |
| `Cargo.toml` (workspace) | Version 0.4.37 → 0.4.38. |
| `crates/presenter-ui/Cargo.toml` | presenter-ui patch bump (e.g. 0.1.6 → 0.1.7). |

---

## Task 1: Bump version to 0.4.38

**Files:**
- Modify: `Cargo.toml` (workspace `[workspace.package]`)
- Modify: `crates/presenter-ui/Cargo.toml`

- [ ] **Step 1: Confirm versions**

```bash
git fetch origin
grep '^version' Cargo.toml | head -1
grep '^version' crates/presenter-ui/Cargo.toml | head -1
gh release list --limit 1
```

Expected: workspace 0.4.37, presenter-ui 0.1.6, latest release v0.4.26. Bump targets: 0.4.38 / 0.1.7.

- [ ] **Step 2: Bump workspace version**

In `Cargo.toml` under `[workspace.package]`, change `version = "0.4.37"` to `version = "0.4.38"`.

- [ ] **Step 3: Bump presenter-ui version**

In `crates/presenter-ui/Cargo.toml` under `[package]`, change `version = "0.1.6"` to `version = "0.1.7"`.

- [ ] **Step 4: Refresh Cargo.lock files**

```bash
cargo check -p presenter-server 2>&1 | tail -5
cd crates/presenter-ui && cargo check --target wasm32-unknown-unknown 2>&1 | tail -5 && cd ../..
```

Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock crates/presenter-ui/Cargo.toml crates/presenter-ui/Cargo.lock
git commit -m "chore: bump version to 0.4.38 (#worship-pp-followups)"
```

---

## Task 2: Investigate the search-drag failure live

**Files:** none yet — this is a **read-only investigation task**. Output is a finding that informs Task 3.

- [ ] **Step 1: Start dev server (or use the running one at 10.77.8.134:8080)**

The dev server is already running at `http://10.77.8.134:8080` per CLAUDE.md. If it's down, use any running e2e fixture. Do not start a fresh server in production paths.

- [ ] **Step 2: Open Playwright and dispatch a search-to-playlist drag**

Use the Playwright MCP tools (`mcp__plugin_playwright_playwright__browser_navigate`, `browser_evaluate`, `browser_console_messages`) — NOT the test runner. We're debugging, not testing.

```bash
# Navigate
mcp__plugin_playwright_playwright__browser_navigate http://10.77.8.134:8080/ui/operator
```

In `browser_evaluate`, run this script. It performs the same dispatch the real user does and reports what blocks at each step:

```js
async () => {
  // Wait for WASM ready
  let t=0;
  while(t++<60 && document.body.dataset.wasmReady !== 'true') await new Promise(r=>setTimeout(r,200));

  // Create a fresh playlist via API to drop into
  const create = await fetch('/playlists', {
    method: 'POST', headers: {'Content-Type': 'application/json'},
    body: JSON.stringify({ name: 'Search Drag Probe ' + Date.now(), showInDashboard: true })
  });
  const playlist = await create.json();

  // Wait for the new playlist row to appear in the dashboard
  let t2=0;
  while(t2++<150 && !document.querySelector(`[data-role="playlist-list"] [data-playlist-id="${playlist.id}"]`)) await new Promise(r=>setTimeout(r,200));
  const targetRow = document.querySelector(`[data-role="playlist-list"] [data-playlist-id="${playlist.id}"]`);
  if (!targetRow) return { ok: false, step: 'playlist-not-rendered' };

  // Type into the search input to populate results
  const search = document.querySelector('[data-role="global-search-query"]');
  if (!search) return { ok: false, step: 'no-search-input' };
  search.focus();
  search.value = 'a'; // any letter — should match many results
  search.dispatchEvent(new Event('input', { bubbles: true }));

  // Wait for at least one search result
  let t3=0;
  while(t3++<60 && !document.querySelector('[data-role="search-result-item"]')) await new Promise(r=>setTimeout(r,200));
  const source = document.querySelector('[data-role="search-result-item"]');
  if (!source) return { ok: false, step: 'no-search-results' };
  const presId = source.getAttribute('data-presentation-id');

  // Probe: dispatch dragstart on source, observe dataTransfer state
  const dt = new DataTransfer();
  source.dispatchEvent(new DragEvent('dragstart', { bubbles:true, cancelable:true, dataTransfer:dt }));
  const typesAfterDragstart = Array.from(dt.types);

  // Probe: dispatch dragover on target, observe whether prevent_default was called
  const dragoverEv = new DragEvent('dragover', { bubbles:true, cancelable:true, dataTransfer:dt });
  let dragoverDefaultPrevented = false;
  targetRow.dispatchEvent(dragoverEv);
  dragoverDefaultPrevented = dragoverEv.defaultPrevented;

  // Probe: dispatch drop, observe completion
  const dropEv = new DragEvent('drop', { bubbles:true, cancelable:true, dataTransfer:dt });
  targetRow.dispatchEvent(dropEv);
  source.dispatchEvent(new DragEvent('dragend', { bubbles:true, cancelable:true, dataTransfer:dt }));

  // Poll the playlist via API to see whether the entry was added
  let result = { entries: -1 };
  for (let i=0; i<20; i++) {
    await new Promise(r=>setTimeout(r,300));
    const r = await fetch(`/playlists/${playlist.id}`);
    if (r.status === 200) {
      const body = await r.json();
      result.entries = Array.isArray(body.entries) ? body.entries.length : -1;
      if (result.entries >= 1) break;
    }
  }

  // Cleanup
  await fetch(`/playlists/${playlist.id}`, { method: 'DELETE' });

  return {
    presId,
    typesAfterDragstart,
    dragoverDefaultPrevented,
    finalEntries: result.entries,
    ok: result.entries >= 1
  };
}
```

- [ ] **Step 3: Capture the result**

Look at the returned object. Three signals to record:
- `typesAfterDragstart`: should include `"application/x-presentation-id"`. If empty → search dragstart never wrote the data → bug is in `search.rs`.
- `dragoverDefaultPrevented`: should be `true`. If `false` → playlist's dragover handler didn't call `prevent_default` → bug is in `playlist_list.rs:137-158` (the type check failed or didn't match search's two types).
- `finalEntries`: should be `1`. If `0` and the previous two are correct → drop handler ran but the spawn_local task failed (server returned non-200, network race, etc).

Also check `mcp__plugin_playwright_playwright__browser_console_messages` for any errors during the dispatch.

- [ ] **Step 4: Document the finding**

Write the finding (1–2 sentences + the relevant probe values) into a comment in `crates/presenter-ui/src/components/playlist_list.rs` directly above the `on:dragover` handler. Example:

```rust
// Investigation 2026-04-28: drag from [data-role="search-result-item"] failed
// because dataTransfer.dropEffect was "none" (browser default) and the spec's
// allowed/dropEffect negotiation rejected the drop. Setting drop_effect("copy")
// in dragover when types match makes both presentation-row and search-result
// drags work consistently.
```

(Adapt the comment to whatever you actually find. If the cause is something other than dropEffect, the comment must accurately describe what was wrong and what fixed it.)

This task does NOT commit. Proceed directly to Task 3 with the finding in hand.

---

## Task 3: Fix the search-drag root cause

**Files:**
- Modify: `crates/presenter-ui/src/components/playlist_list.rs:137-158` (most likely; or wherever Task 2 found the root cause)

- [ ] **Step 1: Apply the fix based on Task 2's finding**

If Task 2 found that `dragoverDefaultPrevented = false` because of the dropEffect / effectAllowed mismatch (most likely), apply this fix in `crates/presenter-ui/src/components/playlist_list.rs`. Replace the existing dragover handler at lines 137–158:

OLD:
```rust
                                        on:dragover=move |ev: web_sys::DragEvent| {
                                            // Accept presentation drops
                                            if let Some(dt) = ev.data_transfer() {
                                                let types = dt.types();
                                                let accepts = (0..types.length())
                                                    .any(|i| {
                                                        let t = types.get(i).as_string().unwrap_or_default();
                                                        t == "application/x-presentation-id" || t == "application/x-presenter-search"
                                                    });
                                                if accepts {
                                                    ev.prevent_default();
                                                    if let Some(target) = ev.target() {
                                                        if let Ok(el) = target.dyn_into::<web_sys::Element>() {
                                                            let _ = el.closest(".operator__list-item")
                                                                .ok()
                                                                .flatten()
                                                                .map(|li| li.class_list().add_1("drag-over"));
                                                        }
                                                    }
                                                }
                                            }
                                        }
```

NEW:
```rust
                                        on:dragover=move |ev: web_sys::DragEvent| {
                                            // Accept presentation drops (from library list OR search results).
                                            if let Some(dt) = ev.data_transfer() {
                                                let types = dt.types();
                                                let accepts = (0..types.length())
                                                    .any(|i| {
                                                        let t = types.get(i).as_string().unwrap_or_default();
                                                        t == "application/x-presentation-id" || t == "application/x-presenter-search"
                                                    });
                                                if accepts {
                                                    ev.prevent_default();
                                                    // Search results dragstart with effectAllowed="copy", and Chromium
                                                    // requires dropEffect to match for the drop to be accepted.
                                                    // Without this, dragging from [data-role="search-result-item"]
                                                    // visually drags but the drop is silently rejected.
                                                    dt.set_drop_effect("copy");
                                                    if let Some(target) = ev.target() {
                                                        if let Ok(el) = target.dyn_into::<web_sys::Element>() {
                                                            let _ = el.closest(".operator__list-item")
                                                                .ok()
                                                                .flatten()
                                                                .map(|li| li.class_list().add_1("drag-over"));
                                                        }
                                                    }
                                                }
                                            }
                                        }
```

If Task 2 found a different root cause, apply the matching fix instead. Either way the fix is small and localized — do NOT restructure the handler.

- [ ] **Step 2: Build the WASM crate**

```bash
cd crates/presenter-ui
cargo build --target wasm32-unknown-unknown 2>&1 | tail -5
cargo clippy --target wasm32-unknown-unknown -- -D warnings 2>&1 | tail -10
cd ../..
```

Expected: clean build, zero warnings.

- [ ] **Step 3: Re-run the Task 2 probe to verify the fix**

Repeat the Playwright probe from Task 2. The probe must now end with `finalEntries: 1` and `dragoverDefaultPrevented: true`. If it still fails, return to Task 2 step 4 and re-investigate before committing.

- [ ] **Step 4: Commit**

```bash
cargo fmt --all
git add crates/presenter-ui/src/components/playlist_list.rs
git commit -m "fix(ui): accept search-result drops on playlist row (#worship-pp-followups)

Set dataTransfer.dropEffect to \"copy\" in the playlist row's dragover
handler when types match, so search results (which dragstart with
effectAllowed=\"copy\") can be dropped. Without the matching dropEffect,
Chromium silently rejects the drop and no playlist entry is added."
```

(Adapt the commit message body if Task 2 found a different root cause.)

---

## Task 4: Sidebar resize + bigger song fonts

**Files:**
- Modify: `crates/presenter-ui/styles/stage.css:332-368`

- [ ] **Step 1: Update the worship-pp CSS rules**

In `crates/presenter-ui/styles/stage.css`, find and replace these rules. Use grep to confirm line numbers first:

```bash
grep -n "stage-pp__slides-area\|stage-pp__playlist-sidebar\|stage-pp__playlist-entry" crates/presenter-ui/styles/stage.css
```

Replace `.stage-pp__slides-area` (currently width 70%):

```css
.stage-pp__slides-area {
    position: absolute;
    left: 0;
    top: 0;
    width: 78%;
    height: 92%;
    overflow: hidden;
    box-sizing: border-box;
}
```

Replace `.stage-pp__playlist-sidebar` (currently width 30%):

```css
.stage-pp__playlist-sidebar {
    position: absolute;
    right: 0;
    top: 0;
    width: 22%;
    height: 92%;
    overflow-y: auto;
    border-left: 1px solid rgba(255, 255, 255, 0.1);
    padding: 1% 1.5%;
    box-sizing: border-box;
}
```

Replace `.stage-pp__playlist-entry`:

```css
.stage-pp__playlist-entry {
    padding: 0.6vh 0.8rem;
    color: #94a3b8;
    /* 12 entries × ~7vh row height ≈ 84vh fits in 92vh sidebar; readable from across a worship space. */
    font-size: 2.6vh;
    line-height: 1.1;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    border-radius: 4px;
    margin-bottom: 2px;
}
```

Replace `.stage-pp__playlist-entry--active`:

```css
.stage-pp__playlist-entry--active {
    background: #38bdf8;
    color: #0f172a;
    font-weight: 700;
    border-left: 4px solid #0ea5e9;
    /* Compensate for the 4px border so text alignment with non-active rows
       stays consistent. Base padding-left is 0.8rem; subtract the border width. */
    padding-left: calc(0.8rem - 4px);
}
```

- [ ] **Step 2: Commit**

```bash
git add crates/presenter-ui/styles/stage.css
git commit -m "fix(stage): worship-pp sidebar narrower + bigger song text (#worship-pp-followups)

Sidebar 30% → 22%, slides-area 70% → 78% so the main slide content
gets ~11% more horizontal space. Entry font-size 0.9vw → 2.6vh
(≈28px @ 1080p, ≈37px @ 1440p) so song titles are legible from
the back of a worship space. Padding now in vh so 12 entries fit
the 92vh sidebar exactly; >12 scroll, <12 keep their per-row size."
```

---

## Task 5: E2E — drag from search result to playlist

**Files:**
- Modify: `tests/e2e/wasm-playlist-operations.spec.ts`

- [ ] **Step 1: Add the new test**

Append a new test inside the existing `test.describe("WASM Operator Playlist Operations", ...)` block in `tests/e2e/wasm-playlist-operations.spec.ts`. Place it after the existing "drop a presentation onto a playlist row appends an entry" test:

```typescript
  test("drop a search-result onto a playlist row appends an entry", async ({
    page,
  }) => {
    // Regression guard for #worship-pp-followups: dragging from
    // [data-role="search-result-item"] (which dragstart with
    // effectAllowed="copy") onto a playlist row was silently
    // rejected because the playlist's dragover never set a
    // matching dropEffect.

    const consoleErrors: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        const t = msg.text();
        if (!t.includes("favicon")) consoleErrors.push(`[${msg.type()}] ${t}`);
      }
    });

    // 1. Create the drop-target playlist via API.
    const targetName = `Search Drop Test ${Date.now()}`;
    const createResp = await page.request.post(
      new URL("/playlists", baseURL).toString(),
      { data: { name: targetName, showInDashboard: true } },
    );
    expect(createResp.status()).toBe(200);
    const created = await createResp.json();
    const targetPlaylistId = created.id as string;
    expect(targetPlaylistId).toBeTruthy();

    // 2. Load operator UI.
    await initPage(page);
    await page.waitForFunction(
      (id: string) =>
        !!document.querySelector(
          `[data-role="playlist-list"] [data-playlist-id="${id}"]`,
        ),
      targetPlaylistId,
      { timeout: 30_000 },
    );

    // 3. Type into the search box to populate results.
    const searchInput = page.locator('[data-role="global-search-query"]');
    await searchInput.click();
    await searchInput.fill("a");
    await page.waitForSelector('[data-role="search-result-item"]', {
      timeout: 15_000,
    });

    // 4. Programmatically drag a search result onto the playlist row.
    //    Pre-populate the DataTransfer (synthetic-event quirk — see
    //    the existing drop test for the same workaround).
    const dragResult = await page.evaluate((id: string) => {
      const source = document.querySelector(
        '[data-role="search-result-item"][data-presentation-id]',
      ) as HTMLElement | null;
      const targetRow = document.querySelector(
        `[data-role="playlist-list"] [data-playlist-id="${id}"]`,
      ) as HTMLElement | null;
      if (!source || !targetRow) {
        return {
          error: "missing source or target",
          hasSource: !!source,
          hasTarget: !!targetRow,
        };
      }
      const sourceId = source.getAttribute("data-presentation-id") || "";
      const dt = new DataTransfer();
      dt.setData("text/plain", sourceId);
      dt.setData("application/x-presentation-id", sourceId);
      dt.setData("application/x-presenter-search", sourceId);
      source.dispatchEvent(
        new DragEvent("dragstart", {
          bubbles: true,
          cancelable: true,
          dataTransfer: dt,
        }),
      );
      targetRow.dispatchEvent(
        new DragEvent("dragover", {
          bubbles: true,
          cancelable: true,
          dataTransfer: dt,
        }),
      );
      targetRow.dispatchEvent(
        new DragEvent("drop", {
          bubbles: true,
          cancelable: true,
          dataTransfer: dt,
        }),
      );
      source.dispatchEvent(
        new DragEvent("dragend", {
          bubbles: true,
          cancelable: true,
          dataTransfer: dt,
        }),
      );
      return { sourceId };
    }, targetPlaylistId);

    expect(dragResult.error, JSON.stringify(dragResult)).toBeUndefined();
    expect(dragResult.sourceId).toBeTruthy();

    // 5. Confirm via API that the playlist gained an entry.
    await expect
      .poll(
        async () => {
          const apiResp = await page.request.get(
            new URL(`/playlists/${targetPlaylistId}`, baseURL).toString(),
          );
          if (apiResp.status() !== 200) return -1;
          const body = await apiResp.json();
          return Array.isArray(body.entries) ? body.entries.length : -1;
        },
        { timeout: 15_000, intervals: [500, 1000, 2000] },
      )
      .toBeGreaterThanOrEqual(1);

    // 6. Cleanup.
    await page.request.delete(
      new URL(`/playlists/${targetPlaylistId}`, baseURL).toString(),
    );

    expect(consoleErrors).toEqual([]);
  });
```

- [ ] **Step 2: Run the new test locally**

```bash
npx playwright test tests/e2e/wasm-playlist-operations.spec.ts --grep "drop a search-result" 2>&1 | tail -20
```

Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add tests/e2e/wasm-playlist-operations.spec.ts
git commit -m "test(e2e): drop a search-result onto a playlist row (#worship-pp-followups)

Regression guard for the search→playlist drop fix. Mirrors the
existing presentation-row drop test but sources from
[data-role=\"search-result-item\"] in the global search results."
```

---

## Task 6: E2E — sidebar width and entry font-size assertions

**Files:**
- Modify: `tests/e2e/stage-worship-pp.spec.ts`

- [ ] **Step 1: Add a new test inside the existing describe block**

Append after the existing tests inside `test.describe("Stage worship-pp layout", () => { ... })`:

```typescript
  test("sidebar is narrower (~22%) and entries have projector-readable font", async ({
    page,
  }) => {
    const consoleErrors: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        const t = msg.text();
        if (!t.includes("favicon")) consoleErrors.push(`[${msg.type()}] ${t}`);
      }
    });

    // Set worship-pp layout
    const layoutResp = await page.request.post(
      new URL("/stage/layout", baseURL).toString(),
      { data: { code: "worship-pp" } },
    );
    expect(layoutResp.ok()).toBeTruthy();

    // Seed a playlist with one entry and trigger it so the sidebar has content.
    const libsResp = await page.request.get(
      new URL("/libraries", baseURL).toString(),
    );
    const libs = (await libsResp.json()) as Array<{
      id: string;
      presentations?: Array<{ id: string; slides?: Array<{ id: string }> }>;
    }>;
    const presentation = libs
      .flatMap((lib) => lib.presentations ?? [])
      .find((p) => (p.slides?.length ?? 0) > 0);
    if (!presentation || !presentation.slides || !presentation.slides[0]) {
      test.skip(true, "test fixture has no presentation with slides");
      return;
    }
    const slideId = presentation.slides[0].id;

    const playlistResp = await page.request.post(
      new URL("/playlists", baseURL).toString(),
      {
        data: {
          name: `Sidebar Width Test ${Date.now()}`,
          showInDashboard: true,
        },
      },
    );
    const playlist = (await playlistResp.json()) as { id: string };
    await page.request.put(
      new URL(`/playlists/${playlist.id}/entries`, baseURL).toString(),
      {
        data: {
          entries: [{ type: "presentation", presentationId: presentation.id }],
        },
      },
    );
    await page.request.post(
      new URL("/stage/state", baseURL).toString(),
      {
        data: {
          presentationId: presentation.id,
          currentSlideId: slideId,
          playlistId: playlist.id,
        },
      },
    );

    await page.goto(new URL("/stage", baseURL).toString());
    await page.waitForFunction(
      () => document.body.dataset.wasmReady === "true",
      { timeout: 30_000 },
    );
    await page.waitForFunction(
      () => document.body.dataset.layoutCode === "worship-pp",
      { timeout: 30_000 },
    );

    // Wait for the playlist sidebar to render with at least one entry.
    await page.waitForSelector(".stage-pp__playlist-entry", {
      timeout: 15_000,
    });

    // Read sidebar width and entry font-size from computed styles.
    const measurements = await page.evaluate(() => {
      const sidebar = document.querySelector(
        ".stage-pp__playlist-sidebar",
      ) as HTMLElement | null;
      const entry = document.querySelector(
        ".stage-pp__playlist-entry",
      ) as HTMLElement | null;
      if (!sidebar || !entry) return { error: "missing element" } as const;
      const sidebarRect = sidebar.getBoundingClientRect();
      const viewportWidth = window.innerWidth;
      const entryStyle = getComputedStyle(entry);
      return {
        sidebarRatio: sidebarRect.width / viewportWidth,
        entryFontSizePx: parseFloat(entryStyle.fontSize),
      } as const;
    });

    expect("error" in measurements, JSON.stringify(measurements)).toBe(false);
    if ("sidebarRatio" in measurements) {
      // Sidebar must be ~22% (allow ±3% slack for borders/scrollbar).
      expect(measurements.sidebarRatio).toBeGreaterThan(0.19);
      expect(measurements.sidebarRatio).toBeLessThan(0.25);
      // Entry font must be readable from the back of a room — sanity floor at 24px.
      expect(measurements.entryFontSizePx).toBeGreaterThanOrEqual(24);
    }

    // Cleanup
    await page.request.delete(
      new URL(`/playlists/${playlist.id}`, baseURL).toString(),
    );

    expect(consoleErrors).toEqual([]);
  });
```

- [ ] **Step 2: Run the new test locally**

```bash
npx playwright test tests/e2e/stage-worship-pp.spec.ts --grep "sidebar is narrower" 2>&1 | tail -20
```

Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add tests/e2e/stage-worship-pp.spec.ts
git commit -m "test(e2e): assert worship-pp sidebar width and entry font-size (#worship-pp-followups)

Regression guard for the sidebar resize:
- sidebar width is ~22% of viewport (±3% slack)
- entry computed font-size is at least 24px"
```

---

## Task 7: Local checks + push + monitor pipeline CI

- [ ] **Step 1: Format / lint / test (workspace)**

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all 2>&1 | tail -10
cargo test --workspace 2>&1 | tail -10
```

Expected: every command exits 0, clippy zero warnings.

- [ ] **Step 2: presenter-ui specifically**

```bash
cd crates/presenter-ui
cargo clippy --target wasm32-unknown-unknown -- -D warnings 2>&1 | tail -10
cargo test --target x86_64-unknown-linux-gnu 2>&1 | tail -10
cd ../..
```

Expected: clean.

- [ ] **Step 3: Push**

```bash
git push origin dev
```

- [ ] **Step 4: Monitor pipeline**

```bash
gh run list --branch dev --limit 3 --json databaseId,name,event,headSha,status,conclusion --jq '.[] | select(.name == "Pipeline" and .event == "push")' | head -1
```

Then background-poll once:

```bash
sleep 1500 && gh run view <RUN_ID> --json status,conclusion,jobs --jq '{status, conclusion, jobs: [.jobs[] | {name, status, conclusion}]}'
```

Per `ci-monitoring.md` — do NOT poll repeatedly. One background command, react when it returns.

Expected: ALL jobs green (Format, Clippy, Test, Build, Code Coverage, Mutation Testing, Playwright E2E 1/3+2/3+3/3, Merge E2E Reports, Deploy to Dev). If anything fails, fetch the failed log (`gh run view <RUN_ID> --log-failed`), fix the root cause in ONE commit, push, monitor again.

---

## Task 8: Verify on dev + open / update PR

- [ ] **Step 1: Verify dev healthz**

```bash
curl -s http://10.77.8.134:8080/healthz
```

Expected: `{"channel":"dev","status":"ok","version":"0.4.38"}`.

- [ ] **Step 2: Live verify the two fixes**

Use Playwright MCP to:
1. Open `/ui/operator`. Type a search query, drag a result onto a playlist row, confirm the entry persists via `GET /playlists/{id}`. Browser console: zero errors.
2. Open `/stage` with worship-pp active. Confirm sidebar visibly narrower (≈22%) and song text large (≈30px+). Active highlight pill still visible.

- [ ] **Step 3: Open or update the PR**

Check the existing PR state:

```bash
gh pr list --base main --head dev --state open --json number,title,mergeable,mergeStateStatus,url
```

If PR #268 is still open (not yet merged), update its title and body via REST API to cover the combined scope (worship-pp baseline + drag-drop fixes + three layout regressions + these followups):

```bash
cat > /tmp/pr-body.md <<'EOF'
## Summary

Worship-pp baseline + drag-drop infrastructure fixes + layout regressions + follow-up polish.

### Stage worship-pp baseline (#268)
- Adopts worship-snv improvements (autofit text, song-name boxes, six-region layout)
- Keeps playlist sidebar; derives next_song_text from Presenter playlist instead of AbleSet

### Drag-drop infrastructure
- Added missing GET /playlists/{id} route
- Fixed WASM PlaylistEntryPayload field renames (presentationId/entryId)

### Layout regressions
- Wrapped six worship-pp regions in .stage-pp__slides-area to stop sidebar overlap
- Replaced 15%-opacity tint with solid sky-blue + 4px accent bar (visible from projector)
- Server enriches playlist response with presentation_name; operator reads it directly; deleted obsolete rebuild_playlist_presentations_with_signal helper

### Follow-up polish
- Search-result drag onto playlist row now accepted (set_drop_effect("copy") in dragover)
- Worship-pp sidebar shrunk to 22% (slides 78%); entry font 0.9vw → 2.6vh so 12 entries fit at projector-readable size

## Test plan
- [x] CI pipeline green (all 21 jobs incl. Mutation Testing)
- [x] 3 core/serde unit tests for presentation_name round-trip
- [x] 2 server integration tests for response enrichment
- [x] Playwright E2E: stage-worship-pp.spec.ts (overlap, highlight, sidebar width, font-size)
- [x] Playwright E2E: wasm-playlist-operations.spec.ts (drag from presentation, drag from search, name visible after drag)
- [x] Live Playwright verification on dev (10.77.8.134:8080)
- [x] Browser console: zero errors / warnings on operator and stage
EOF
gh api -X PATCH repos/zbynekdrlik/presenter/pulls/268 \
  -f title="feat(stage): worship-pp baseline, drag-drop infra, layout regressions, follow-ups" \
  -F body=@/tmp/pr-body.md \
  --jq '{number, title, state, mergeable_state}'
```

If PR #268 was already merged, open a new PR:

```bash
gh pr create --base main --head dev \
  --title "fix(stage): worship-pp follow-ups — search drag + sidebar polish" \
  --body "$(cat <<'EOF'
## Summary
- Search-result drag onto playlist row was silently rejected (Chromium dropEffect mismatch with effectAllowed=\"copy\"). Fix: set_drop_effect(\"copy\") in playlist row's dragover when types match.
- Worship-pp sidebar shrunk to 22% (slides 78%); entry font 0.9vw → 2.6vh so up to 12 entries fit at projector-readable size.

## Test plan
- [x] CI pipeline green
- [x] Playwright E2E: drag from search result to playlist
- [x] Playwright E2E: sidebar width ~22% + entry font ≥24px
- [x] Live verification on dev
- [x] Browser console clean

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 4: Wait for the PR pipeline to be CLEAN**

```bash
gh pr view <PR_NUMBER> --json mergeable,mergeStateStatus,url
```

Expected: `{"mergeable":"MERGEABLE","mergeStateStatus":"CLEAN", ...}`. If UNSTABLE, identify which check is pending or failing (most likely Mutation Testing) and wait/fix.

- [ ] **Step 5: Send completion report and wait for explicit "merge it"**

Per `pr-merge-policy.md`, do NOT merge. Send the completion report (per `completion-report.md`) and wait for the user's explicit merge instruction.

---

## Task 9: Post-merge production verification (after user merges)

- [ ] **Step 1: Watch the post-merge main pipeline**

```bash
gh run list --branch main --limit 3 --json databaseId,name,event,status,conclusion --jq '.[] | select(.event == "push")' | head -1
```

Background-poll: `sleep 1500 && gh run view <RUN_ID> ...`. Wait for ALL jobs green incl. `Deploy to Production`.

- [ ] **Step 2: Verify production**

```bash
curl -s http://10.77.9.205/healthz
```

Expected: `{"channel":"release","status":"ok","version":"0.4.38"}`.

Then re-do the live verification from Task 8 step 2 against `http://10.77.9.205/...`. Capture: search drag adds entry, worship-pp sidebar visibly narrower with bigger song text.

- [ ] **Step 3: Final completion report**

Per `completion-report.md` — short report with URLs (dev + prod, frontend + backend), audits clean, what shipped, no follow-up question.

---

## Verification Summary

| Check | How |
|-------|-----|
| Search-drag fix works | Task 5 E2E test asserts entry persists after drag from search-result |
| Sidebar resized | Task 6 E2E test asserts width ratio is ~22% |
| Font readable from projector | Task 6 E2E test asserts computed font-size ≥ 24px |
| 12-entry layout fits | CSS math: 12 × ~7vh ≈ 84vh fits in 92vh sidebar; existing overflow-y:auto handles >12 |
| Active highlight still visible | PR #268's E2E test (background distinct from inactive) still passes |
| No regression on existing layouts | All other Playwright E2E tests still pass on CI |
| Browser console clean | Every E2E asserts `consoleErrors === []` |
| Worship-snv unaffected | No changes to worship-snv CSS or component code |
