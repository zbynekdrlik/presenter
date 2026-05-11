# Worship Slide Editor: Drop Save Button + "Saved ✓" Indicator Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove the misleading Save button on the worship slide editor; add a transient per-slide "Saved ✓" indicator so the operator sees that blur-time persistence happened.

**Architecture:** Add a per-slide save-status map to `OperatorState`. Wire the existing blur-time save path to update that map (Saving → Saved → fade, or Failed). Render the badge in the slide header. Remove the Save button from `operator__slide-controls`. A monotonic token prevents an earlier fade-timer from clearing a later save's entry.

**Tech Stack:** Rust + Leptos (WASM), CSS, Playwright (TypeScript).

**Spec:** `docs/superpowers/specs/2026-05-11-worship-slide-saved-indicator-design.md` (commit f7f65b4)

---

## Context

Issue #313: the operator thinks they must click a Save button on the worship slide editor to persist edits. In reality, the `on:blur` handler on each textarea already saves via `save_all_fields_from_dom(...)`. The fix is to (a) remove the misleading button and (b) add a visible badge confirming each save.

**Key existing code:**

- `crates/presenter-ui/src/state/operator.rs` — `OperatorState` struct (53 fields incl. focused_slide_id, pending_focus, etc.). New `SaveStatus` enum + `save_status` HashMap field go here.
- `crates/presenter-ui/src/components/slide_list.rs:82-142` — `save_all_fields_from_dom` function. Spawns async PUT, ignores result. Must be rewired to update `save_status`.
- `crates/presenter-ui/src/components/slide_list.rs:614-635` — the `<button data-action="save">` to remove.
- `crates/presenter-ui/src/components/slide_list.rs:583-606` — slide-header markup where the badge `<Show>` block is inserted.
- `crates/presenter-ui/src/components/slide_list.rs:856+` — Group `<input>` blur handler that also calls `update_slide_with_group` inline. Must use the same status helper.
- No existing tests reference `data-action="save"` outside the component file itself (verified by grep). No tests to delete.

---

## File Structure

### Created files

| File | Responsibility |
|------|----------------|
| `tests/e2e/operator-slide-save-indicator.spec.ts` | 3-scenario E2E covering: Saved ✓ appears+fades on success, no Save button in controls, "Save failed" sticky on 500. |

### Modified files

| File | Change |
|------|--------|
| `Cargo.toml` (workspace) | bump `[workspace.package].version` 0.4.73 → 0.4.74 |
| `crates/presenter-ui/src/state/operator.rs` | add `SaveStatus` enum + `save_status: RwSignal<HashMap<String, (SaveStatus, u64)>>` field + constructor init |
| `crates/presenter-ui/src/components/slide_list.rs` | extract `save_with_status` helper, rewire `save_all_fields_from_dom` + Group blur to use it; remove the Save button; add badge `<Show>` to slide header |
| `crates/presenter-ui/styles/operator.css` | add `.operator__slide-save-indicator` rules with status-attribute color variants |

---

## Task 1: Version bump 0.4.73 → 0.4.74

**Files:**
- Modify: `Cargo.toml` (workspace `[workspace.package].version`)

- [ ] **Step 1: Edit version**

In `/home/newlevel/devel/presenter/presenter-dev2/Cargo.toml`, change:

```toml
version = "0.4.73"
```

to:

```toml
version = "0.4.74"
```

- [ ] **Step 2: Refresh Cargo.lock**

Run: `cargo update -w`

Expected: lines for each workspace member updating from `v0.4.73` → `v0.4.74`. No errors.

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "chore: bump version to 0.4.74"
```

---

## Task 2: Write the regression-test-first Playwright E2E

This task lands BEFORE the implementation, so scenario 2 fails RED (button still present) on this commit and turns GREEN once Task 5 removes it. Scenarios 1 and 3 also start RED (no indicator element exists yet) and turn GREEN after Tasks 3-6 ship the indicator. The plan-check + completion-report `✅ Regression test:` line cites scenario 2 by file:line and the RED + GREEN SHAs.

**Files:**
- Create: `tests/e2e/operator-slide-save-indicator.spec.ts`

- [ ] **Step 1: Pick the closest reference test**

Read `tests/e2e/operator-slide-edit.spec.ts` if it exists. If not, use `tests/e2e/stage-api-ndi.spec.ts` for the `beforeAll`/`afterAll` server bootstrap pattern (already used successfully in the prior NFC PR). Either way the spec needs the same `deriveTestConfig`/`startTestServer`/`stopServer` plumbing from `tests/e2e/support`.

- [ ] **Step 2: Create the test file**

Create `/home/newlevel/devel/presenter/presenter-dev2/tests/e2e/operator-slide-save-indicator.spec.ts`:

```typescript
import { test, expect } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

test.describe.configure({ timeout: 180_000 });

const ALLOWED_CONSOLE_NOISE = [
  /integrity.*ignored.*preload/i,
  /ResizeObserver loop/i,
];

function collectConsoleErrors(
  page: import("@playwright/test").Page,
  extraAllowed: RegExp[] = [],
): string[] {
  const messages: string[] = [];
  const allowed = [...ALLOWED_CONSOLE_NOISE, ...extraAllowed];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      const text = msg.text();
      if (!allowed.some((pattern) => pattern.test(text))) {
        messages.push(`[${msg.type()}] ${text}`);
      }
    }
  });
  return messages;
}

let server: ServerHandle | undefined;
let baseURL = "";
let dbUrl = "";
let port = 0;

test.beforeAll(async ({}, testInfo) => {
  const cfg = deriveTestConfig(testInfo);
  baseURL = cfg.baseURL;
  dbUrl = cfg.dbUrl;
  port = cfg.port;
  await refreshDevData(dbUrl);
  server = await startTestServer(port, dbUrl, cfg.oscPort);
});

test.afterAll(async () => {
  await stopServer(server);
  server = undefined;
});

/**
 * Helper: open the operator, select the first available presentation, put its
 * first slide into edit mode. Returns the slide_id of the slide we're editing.
 */
async function openOperatorWithEditingSlide(
  page: import("@playwright/test").Page,
): Promise<string> {
  await page.goto(new URL("/ui/operator", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  // First library, first presentation
  await page.locator('[data-role="library-list"] li').first().click();
  await page.locator('[data-role="presentation-list"] li').first().click();
  // Enter edit mode on the first slide
  const slideCard = page.locator('[data-role="slide-card"]').first();
  await slideCard.locator('[data-action="edit"]').click();
  // Wait for the editor to materialize
  await expect(slideCard.locator('textarea[data-field="main"]')).toBeVisible({
    timeout: 5_000,
  });
  return (await slideCard.getAttribute("data-slide-id")) ?? "";
}

test('Saved indicator appears and fades after blur', async ({ page }) => {
  const consoleMessages = collectConsoleErrors(page);

  const slideId = await openOperatorWithEditingSlide(page);
  const slideCard = page.locator(`[data-role="slide-card"][data-slide-id="${slideId}"]`);

  // Type into the main textarea
  const main = slideCard.locator('textarea[data-field="main"]');
  await main.click();
  await main.press("End");
  await main.type(" autosave probe", { delay: 20 });

  // Blur by clicking outside any editable region
  await page.locator("body").click({ position: { x: 5, y: 5 } });

  // Indicator must appear within 3s
  const indicator = slideCard.locator('[data-role="slide-save-indicator"]');
  await expect(indicator).toBeVisible({ timeout: 3_000 });
  await expect(indicator).toHaveAttribute("data-status", "saved");
  await expect(indicator).toHaveText(/Saved/);

  // Indicator must fade (disappear) within 5s after that
  await expect(indicator).toHaveCount(0, { timeout: 5_000 });

  expect(consoleMessages).toEqual([]);
});

test('Save button is absent from slide controls (regression #313)', async ({ page }) => {
  const consoleMessages = collectConsoleErrors(page);

  const slideId = await openOperatorWithEditingSlide(page);
  const slideCard = page.locator(`[data-role="slide-card"][data-slide-id="${slideId}"]`);

  // The Save button used to live in operator__slide-controls. After #313 it must be gone.
  await expect(
    slideCard.locator('.operator__slide-controls button[data-action="save"]'),
  ).toHaveCount(0);

  // Duplicate and Delete must remain.
  await expect(
    slideCard.locator('.operator__slide-controls button[data-action="duplicate"]'),
  ).toBeVisible();
  await expect(
    slideCard.locator('.operator__slide-controls button[data-action="delete"]'),
  ).toBeVisible();

  expect(consoleMessages).toEqual([]);
});

test('Save failed sticks when server returns 500', async ({ page }) => {
  // 500 on the slide-update PUT will produce a console error; allow it for this test.
  const consoleMessages = collectConsoleErrors(page, [
    /Failed to load resource.*500/i,
  ]);

  await page.route("**/presentations/*/slides/*", (route) => {
    if (route.request().method() === "PUT") {
      route.fulfill({ status: 500, body: "boom" });
    } else {
      route.continue();
    }
  });

  const slideId = await openOperatorWithEditingSlide(page);
  const slideCard = page.locator(`[data-role="slide-card"][data-slide-id="${slideId}"]`);

  const main = slideCard.locator('textarea[data-field="main"]');
  await main.click();
  await main.press("End");
  await main.type(" will fail", { delay: 20 });
  await page.locator("body").click({ position: { x: 5, y: 5 } });

  const indicator = slideCard.locator('[data-role="slide-save-indicator"]');
  await expect(indicator).toBeVisible({ timeout: 3_000 });
  await expect(indicator).toHaveAttribute("data-status", "failed");
  await expect(indicator).toHaveText(/failed/i);

  // Must NOT fade within 5s
  await page.waitForTimeout(5_000);
  await expect(indicator).toBeVisible();
  await expect(indicator).toHaveAttribute("data-status", "failed");

  expect(consoleMessages).toEqual([]);
});
```

- [ ] **Step 3: Run the test — expect it to FAIL (RED)**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
npm run test:playwright -- operator-slide-save-indicator
```

Expected: AT LEAST scenario "Save button is absent from slide controls (regression #313)" FAILS — the button still exists. Scenarios 1 and 3 likely also fail because the indicator element doesn't exist yet. This RED state is the regression-test-first baseline. **Note the test commit SHA after the next step — this is the `RED` SHA you cite in the completion report.**

- [ ] **Step 4: Commit (RED)**

```bash
git add tests/e2e/operator-slide-save-indicator.spec.ts
git commit -m "test(e2e): regression test for #313 — Save button removed + Saved indicator (RED)"
```

Record this commit SHA — it's the RED reference.

---

## Task 3: Add `SaveStatus` enum + `save_status` field to `OperatorState`

**Files:**
- Modify: `crates/presenter-ui/src/state/operator.rs`

- [ ] **Step 1: Add the import and enum at the top of the file**

Open `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui/src/state/operator.rs`. After `use leptos::prelude::*;` (line 1), add:

```rust
use std::collections::HashMap;

/// Per-slide save status for the worship slide editor's "Saved" indicator (#313).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveStatus {
    Saving,
    Saved,
    Failed,
}
```

- [ ] **Step 2: Add the field to `OperatorState`**

Inside the existing `pub struct OperatorState { ... }` block, after the last field `pub triggering_slide_id: RwSignal<Option<String>>,` (line 52), add:

```rust
    /// Per-slide save status keyed by slide_id, with a monotonic token to
    /// prevent stale fade timers from clearing a newer save's entry (#313).
    pub save_status: RwSignal<HashMap<String, (SaveStatus, u64)>>,
```

- [ ] **Step 3: Initialise it in the constructor**

In `OperatorState::new()`, after `triggering_slide_id: RwSignal::new(None),` (line 96), add a trailing line so the struct literal ends with:

```rust
            triggering_slide_id: RwSignal::new(None),
            save_status: RwSignal::new(HashMap::new()),
```

- [ ] **Step 4: Run WASM clippy**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings -W clippy::all
```

Expected: `Finished` with zero warnings.

- [ ] **Step 5: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
git add crates/presenter-ui/src/state/operator.rs
git commit -m "feat(operator): add SaveStatus + save_status map for indicator (#313)"
```

---

## Task 4: Extract `save_with_status` helper + rewire blur saves

**Files:**
- Modify: `crates/presenter-ui/src/components/slide_list.rs` (the `save_all_fields_from_dom` function and the Group `<input>` blur handler)

- [ ] **Step 1: Add the imports**

At the top of `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui/src/components/slide_list.rs`, add (or extend existing `use` lines):

```rust
use crate::state::operator::SaveStatus;
use gloo_timers::future::TimeoutFuture;
use std::sync::atomic::{AtomicU64, Ordering};
```

If `gloo_timers` isn't already a dependency, check `crates/presenter-ui/Cargo.toml`. The crate is widely used in the codebase (search for `TimeoutFuture` elsewhere). If absent, add `gloo-timers = "0.3"` to `[dependencies]` in that Cargo.toml — but verify with grep first; do not add a dep that already exists.

- [ ] **Step 2: Add the token counter near the top of the file**

After the imports, add:

```rust
/// Monotonic token used to invalidate stale fade timers (#313).
/// Each save increments this and stamps the per-slide entry; the fade
/// only removes the entry if the token still matches at fade time.
static SAVE_TOKEN: AtomicU64 = AtomicU64::new(0);
```

- [ ] **Step 3: Replace `save_all_fields_from_dom`**

Find the existing function (around line 82-142). Replace its body with the version that uses `save_with_status`:

```rust
fn save_all_fields_from_dom(
    pres_id: &str,
    slide_id: &str,
    _current_field: &str,
    _sel_start: u32,
    _sel_end: u32,
    selected_pres: RwSignal<Option<presenter_core::Presentation>>,
    op: &OperatorState,
) {
    let doc = crate::utils::window::document();

    let main = get_field_value(&doc, slide_id, "main");
    let translation = get_field_value(&doc, slide_id, "translation");
    let stage = get_field_value(&doc, slide_id, "stage");
    let group_val = get_field_value(&doc, slide_id, "group");
    let group = if group_val.trim().is_empty() {
        None
    } else {
        Some(group_val.trim().to_string())
    };

    // Skip no-op saves.
    let pres = selected_pres.get_untracked();
    if let Some(p) = &pres {
        if let Some(slide) = p.slides.iter().find(|s| s.id.to_string() == slide_id) {
            let orig = &slide.content;
            let orig_group = orig.group.as_ref().map(|g| g.name().to_string());
            if orig.main.value() == main
                && orig.translation.value() == translation
                && orig.stage.value() == stage
                && orig_group == group
            {
                return;
            }
        }
    }

    save_with_status(
        pres_id.to_string(),
        slide_id.to_string(),
        main,
        translation,
        stage,
        group,
        op.save_status,
    );
}
```

- [ ] **Step 4: Add the new `save_with_status` helper directly below `save_all_fields_from_dom`**

```rust
/// Wraps the slide-update PUT with save_status updates for the per-slide
/// "Saved" indicator (#313). Sets `Saving` immediately, then `Saved`
/// (auto-fades after 2s) on success or `Failed` (sticky) on error. Uses
/// a monotonic token so an older fade timer can't clear a newer save.
fn save_with_status(
    pres_id: String,
    slide_id: String,
    main: String,
    translation: String,
    stage: String,
    group: Option<String>,
    save_status: RwSignal<std::collections::HashMap<String, (SaveStatus, u64)>>,
) {
    let token = SAVE_TOKEN.fetch_add(1, Ordering::Relaxed) + 1;
    let key_saving = slide_id.clone();
    save_status.update(|map| {
        map.insert(key_saving, (SaveStatus::Saving, token));
    });

    let key_done = slide_id.clone();
    leptos::task::spawn_local(async move {
        let result = api::presentations::update_slide_with_group(
            &pres_id,
            &slide_id,
            &main,
            &translation,
            &stage,
            group,
        )
        .await;

        match result {
            Ok(_) => {
                let key_for_saved = key_done.clone();
                save_status.update(|map| {
                    map.insert(key_for_saved, (SaveStatus::Saved, token));
                });
                TimeoutFuture::new(2_000).await;
                save_status.update(|map| {
                    if map.get(&key_done).map(|(_, t)| *t) == Some(token) {
                        map.remove(&key_done);
                    }
                });
            }
            Err(_) => {
                save_status.update(|map| {
                    map.insert(key_done.clone(), (SaveStatus::Failed, token));
                });
            }
        }
    });
}
```

- [ ] **Step 5: Update `save_all_fields_from_dom` call sites to pass `&op`**

The three textarea `on:blur` handlers around lines 760-840 each call `save_all_fields_from_dom(&pres_id, &sid, "main", sel_start, sel_end, ctx.selected_presentation, &op);`. The signature is unchanged (still `op: &OperatorState`), so call sites work as-is. **Verify the existing 3 call sites** still compile and pass `&op`.

- [ ] **Step 6: Update the Group `<input>` blur handler**

Find the Group input's `on:blur` (around line 856+). It currently spawns its own async task that calls `update_slide_with_group` directly. Replace that async block with a call to `save_with_status` so the Group save also updates the indicator. Pseudocode:

```rust
on:blur={
    let pres_id = pres_id_edit.clone();
    let sid = slide_id_edit.clone();
    let op = op.clone();
    let selected_pres = ctx.selected_presentation;
    move |ev| {
        let (sel_start, sel_end) = capture_selection(&ev);
        op.pending_focus.set(Some((sid.clone(), "group".to_string(), sel_start, sel_end)));

        let doc = crate::utils::window::document();
        let main = get_field_value(&doc, &sid, "main");
        let translation = get_field_value(&doc, &sid, "translation");
        let stage = get_field_value(&doc, &sid, "stage");
        let group_val = get_field_value(&doc, &sid, "group");
        let group = if group_val.trim().is_empty() {
            None
        } else {
            Some(group_val.trim().to_string())
        };

        // Use the same helper so the indicator wires up on Group edits too.
        save_with_status(
            pres_id.clone(),
            sid.clone(),
            main,
            translation,
            stage,
            group,
            op.save_status,
        );

        // Refetch for group inheritance display, then restore focus.
        let pres_id_for_refetch = pres_id.clone();
        let op_for_restore = op.clone();
        leptos::task::spawn_local(async move {
            if let Ok(detail) = api::presentations::get_presentation(&pres_id_for_refetch).await {
                selected_pres.set(Some(detail.presentation));
            }
            restore_pending_focus(&op_for_restore);
        });
    }
}
```

(The refetch + focus restore retains the existing behavior; `save_with_status` runs concurrently. If race conditions surface in QA they can be addressed in a follow-up; the existing inline path was already racey.)

- [ ] **Step 7: Run WASM clippy**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings -W clippy::all
```

Expected: `Finished` with zero warnings. Fix any `unused_imports` or signature mismatches and re-run.

- [ ] **Step 8: Workspace clippy from root**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo clippy --workspace --all-targets -- -D warnings -W clippy::all
```

Expected: `Finished` with zero warnings.

- [ ] **Step 9: Commit**

```bash
git add crates/presenter-ui/src/components/slide_list.rs crates/presenter-ui/Cargo.toml
git commit -m "feat(operator): wire save-status updates into blur saves (#313)"
```

(If `Cargo.toml` wasn't touched in step 1, omit it from the `git add`.)

---

## Task 5: Render the badge in the slide header + remove the Save button

**Files:**
- Modify: `crates/presenter-ui/src/components/slide_list.rs` (slide-header block + slide-controls block)

- [ ] **Step 1: Add the badge `<Show>` block in the slide header**

Open `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui/src/components/slide_list.rs`. Find the existing slide-header block (around line 583-606). Immediately after the existing `{group_badge_text.clone().map(|g| { ... })}` block (around line 593-606), insert:

```rust
                                    {
                                        let sid_for_badge = slide_id.clone();
                                        let save_status = op.save_status;
                                        view! {
                                            <Show when=move || save_status.get().contains_key(&sid_for_badge)>
                                                {
                                                    let sid_for_render = sid_for_badge.clone();
                                                    move || {
                                                        let map = save_status.get();
                                                        let Some((status, _)) = map.get(&sid_for_render).copied() else {
                                                            return view! { <span></span> }.into_any();
                                                        };
                                                        match status {
                                                            SaveStatus::Saving => view! {
                                                                <span
                                                                    class="operator__slide-save-indicator"
                                                                    data-role="slide-save-indicator"
                                                                    data-status="saving"
                                                                >"Saving…"</span>
                                                            }.into_any(),
                                                            SaveStatus::Saved => view! {
                                                                <span
                                                                    class="operator__slide-save-indicator"
                                                                    data-role="slide-save-indicator"
                                                                    data-status="saved"
                                                                >"Saved ✓"</span>
                                                            }.into_any(),
                                                            SaveStatus::Failed => view! {
                                                                <span
                                                                    class="operator__slide-save-indicator"
                                                                    data-role="slide-save-indicator"
                                                                    data-status="failed"
                                                                >"Save failed"</span>
                                                            }.into_any(),
                                                        }
                                                    }
                                                }
                                            </Show>
                                        }
                                    }
```

(If `slide_id` isn't already in scope at this point in the closure stack, look at the existing `slide_id_edit.clone()` bindings just above and re-clone from there.)

- [ ] **Step 2: Remove the Save button**

Find the `<div class="operator__slide-controls">` block (around line 614). Remove the entire `<button type="button" data-action="save" ...>...</button>` element including its `on:click=move |_| { ... }` closure. The block becomes:

```rust
                                            <div class="operator__slide-controls">
                                                <button type="button" data-action="duplicate"
                                                    on:click=move |_| { /* existing duplicate closure */ }
                                                >"Duplicate"</button>
                                                <button type="button" data-action="delete"
                                                    on:click=move |_| { /* existing delete closure */ }
                                                >"Delete"</button>
                                            </div>
```

Also remove the now-dead `let pres_id_save = pres_id_edit.clone();`, `let slide_id_save = slide_id_edit.clone();`, and `let selected_pres_save = ctx.selected_presentation;` bindings at the start of the closure that built that view.

- [ ] **Step 3: Add `data-slide-id` to the slide card root**

The E2E test selects each slide card via `[data-role="slide-card"][data-slide-id="..."]`. Check whether the existing slide card root (the outer `<article>` or `<li>` that holds the slide) already exposes `data-slide-id`. If not, add `data-slide-id=slide_id.clone()` to it. (This is a one-line change that makes the test deterministic; if the attribute already exists, leave it.)

- [ ] **Step 4: WASM clippy**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings -W clippy::all
```

Expected: zero warnings.

- [ ] **Step 5: Workspace clippy**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo clippy --workspace --all-targets -- -D warnings -W clippy::all
```

Expected: zero warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/presenter-ui/src/components/slide_list.rs
git commit -m "feat(operator): render Saved indicator + remove Save button (#313)"
```

Record this SHA — it's the GREEN reference for the regression test.

---

## Task 6: CSS for the indicator

**Files:**
- Modify: `crates/presenter-ui/styles/operator.css`

- [ ] **Step 1: Append the new rules**

At the end of `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui/styles/operator.css`, append:

```css
/* ===== Worship slide save indicator (#313) ===== */
.operator__slide-save-indicator {
    margin-left: 0.5rem;
    font-size: 0.85em;
    font-weight: 500;
    transition: opacity 200ms ease-out;
}

.operator__slide-save-indicator[data-status="saved"] {
    color: #16a34a;
}

.operator__slide-save-indicator[data-status="saving"] {
    color: #6b7280;
}

.operator__slide-save-indicator[data-status="failed"] {
    color: #dc2626;
    font-weight: 600;
}
```

- [ ] **Step 2: Commit**

```bash
git add crates/presenter-ui/styles/operator.css
git commit -m "style(operator): saved-indicator color variants (#313)"
```

---

## Task 7: Re-run the E2E suite — must turn GREEN

- [ ] **Step 1: Run all three scenarios**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
npm run test:playwright -- operator-slide-save-indicator
```

Expected: 3 passed. If any fail:

- Scenario 1 fails on "indicator visible within 3s" → check that the badge's `data-role` is exactly `slide-save-indicator` and that `op.save_status` is the SAME signal instance shared by the save path and the renderer.
- Scenario 2 fails (still has Save button) → Task 5 step 2 was not applied.
- Scenario 3 fails (no failed state) → the PUT-route interception may not match; check the actual PUT URL path in network logs.

If a fix is needed, edit the implementation files (NOT the test) and commit as a follow-up.

- [ ] **Step 2: Capture the GREEN SHA**

The GREEN reference is the SHA at which all three E2E scenarios pass — by default this is the Task 5 commit. If a fix commit was needed between Tasks 5 and 7, the GREEN SHA is the latter.

---

## Task 8: Push, monitor CI, manual verify on dev, open PR (controller-handled)

This task is performed by the orchestrator, not a subagent.

- [ ] **Step 1: Final local checks**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all
cd crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings -W clippy::all && cd ../..
cargo test --workspace -- --nocapture
```

All must pass.

- [ ] **Step 2: Sync with main if needed and push**

```bash
git fetch origin
git merge origin/main --no-edit   # only if dev is behind
git push origin dev
```

- [ ] **Step 3: Capture run id and monitor**

```bash
gh run list --branch dev --limit 1 --json databaseId
# In a single background bash:
sleep 1500 && gh run view <run-id> --json status,conclusion,jobs
```

Wait until ALL jobs (including Mutation Testing) report `success`. If any fail, `gh run view <run-id> --log-failed`, fix in ONE commit, push once.

- [ ] **Step 4: Verify on dev**

After Deploy to Dev is green:

```bash
curl -s 'http://10.77.8.134:8080/healthz'
# Expect: {"channel":"dev","status":"ok","version":"0.4.74"}
```

Optionally drive the same Playwright flow against the live dev server to confirm the indicator works end-to-end on the deployed binary (not just the test server).

- [ ] **Step 5: Open PR**

```bash
gh pr create --base main --head dev --title "feat(operator): drop Save button + add Saved indicator on worship slides (#313)" --body "$(cat <<'EOF'
## Summary

Closes #313: the operator thought they had to click a Save button to persist worship-slide edits. Edits already saved on blur — the button was misleading. Removed the button and added a transient per-slide "Saved ✓" badge so persistence is visible.

## What changed

- `OperatorState` gains `SaveStatus` enum + `save_status: RwSignal<HashMap<String, (SaveStatus, u64)>>`. Token guards against stale fade timers.
- `save_all_fields_from_dom` and the Group `<input>` blur handler both route through a new `save_with_status` helper that sets `Saving`, then `Saved` (auto-fades after 2s) or `Failed` (sticky).
- Slide header renders the badge via a `<Show>` block reading the map.
- Save button removed from `operator__slide-controls`; Duplicate + Delete remain.
- CSS adds `.operator__slide-save-indicator` with status-attribute color variants.
- Playwright E2E `operator-slide-save-indicator.spec.ts` covers all three states (saved fades, no Save button, failed sticks).
- Version bump 0.4.73 → 0.4.74.

## Regression test

`tests/e2e/operator-slide-save-indicator.spec.ts` — scenario `Save button is absent from slide controls (regression #313)`.
RED on test commit `<task-2-sha>`; GREEN on fix commit `<task-5-sha>`.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

Replace `<task-2-sha>` and `<task-5-sha>` with the actual SHAs from Tasks 2 and 5 (or the GREEN fix commit if a later one was needed).

Wait for `mergeStateStatus: CLEAN` and `mergeable: MERGEABLE` before reporting. Do NOT merge until the user says "merge it".

---

## Verification summary

| Check | How to verify |
|-------|---------------|
| Save button removed from slide controls | Playwright scenario 2 |
| "Saved ✓" appears within 3s of blur | Playwright scenario 1 |
| "Saved ✓" fades within ~5s of appearing | Playwright scenario 1 |
| "Save failed" sticks on PUT failure | Playwright scenario 3 |
| Browser console clean across all three | `expect(consoleMessages).toEqual([])` |
| Workspace builds | `cargo build --workspace` green |
| Native clippy clean | `cargo clippy --workspace --all-targets -- -D warnings -W clippy::all` |
| WASM clippy clean | from `crates/presenter-ui`: `cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings -W clippy::all` |
| Version bump | `Cargo.toml` workspace version is `0.4.74` |
| Dev deploy | `/healthz` shows v0.4.74 |
| Regression test cited | Bug-fix PR includes `✅ Regression test:` line with test path, line, RED + GREEN SHAs |
