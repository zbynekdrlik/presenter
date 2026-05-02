# Operator Slide-List Scroll UX Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Three operator slide-list scroll fixes for live worship: lookahead scroll keeps next row visible, wheel scroll becomes deterministic per-notch (neutralising macOS acceleration), and opening a new song scrolls to the first slide.

**Architecture:** All changes inside `crates/presenter-ui/src/components/slide_list.rs` and one CSS line in `operator.css`. Three coordinated changes: (1) rewrite `scroll_slide_into_view` for lookahead using DOM order arithmetic (next-row anchor at active+3), (2) new Effect watching `ctx.selected_presentation_id` change → scroll-to-top, (3) `on:wheel` handler on the slides container with `prevent_default()` + linear `scroll_by`.

**Tech Stack:** Rust + Leptos 0.7 (WASM via trunk), web_sys/WheelEvent, plain CSS, Playwright (TypeScript).

**Spec:** `docs/superpowers/specs/2026-05-02-operator-slide-scroll-design.md` (commit `a0bb145`)

**Closes:** Issue #271 — operator slide-list scroll behavior during live worship.

---

## Context

### Verified pre-flight findings

- **Existing function:** `scroll_slide_into_view` at `crates/presenter-ui/src/components/slide_list.rs:909` — reactive scroll (top-align if above, bottom-align if below).
- **Existing Effect:** `slide_list.rs:215-234` watches `ctx.stage_snapshot.current_slide_id`, calls `scroll_slide_into_view` via `Timeout(0)`. Use this pattern as the template for the new presentation-change Effect.
- **Signal for load-at-start:** `ctx.selected_presentation_id: RwSignal<Option<String>>` at `crates/presenter-ui/src/state/mod.rs:42`. Watching the ID (cheap String comparison) is preferable to watching the full `selected_presentation: RwSignal<Option<Presentation>>` (deep struct compare).
- **Slides container element:** `<div class="operator__slides" data-role="slides">` at `slide_list.rs:291-293`. Already has `on:dragover` and `on:drop` handlers. Add `on:wheel` here.
- **CSS:** `.operator__slides` at `crates/presenter-ui/styles/operator.css:1081` — `display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 0.9rem; overflow-y: auto`. 3 columns hardcoded.
- **Per-slide card:** `<article data-slide-id=...>` rendered at `slide_list.rs:473`. The first such article inside `.operator__slides` provides a reliable "card height" measurement for the wheel STEP.
- **`gloo_timers::callback::Timeout::new(0, ...)`** is the project pattern for "run after next render".
- **WheelEvent in web_sys:** `delta_y() -> f64`, `prevent_default() -> ()`. Available via `web_sys::WheelEvent`.

### Three concerns share the same code surface

All three fixes live in slide_list.rs's component setup block. Tasks 2 and 3 each contribute different scroll-control code to the same file. Test in Task 4 covers all three behaviors.

---

## File Structure

### Modified files
| File | Change |
|------|--------|
| `Cargo.toml` | Workspace version 0.4.53 → 0.4.54 |
| `crates/presenter-ui/Cargo.toml` | presenter-ui version 0.1.22 → 0.1.23 |
| `crates/presenter-ui/src/components/slide_list.rs` | Rewrite `scroll_slide_into_view` for lookahead; add `scroll_slides_to_top` helper, `step_for_wheel` helper, presentation-change Effect, and `on:wheel` handler on the `.operator__slides` div. |
| `crates/presenter-ui/styles/operator.css` | Append `overscroll-behavior: contain` to `.operator__slides` rule at line 1081. |

### Created files
| File | Responsibility |
|------|---------------|
| `tests/e2e/operator-slide-scroll.spec.ts` | Playwright E2E — 3 tests covering lookahead, wheel linearisation, load-at-start. Uses existing `support.ts` helpers. |

### Lock files
- `Cargo.lock` (workspace) and `crates/presenter-ui/Cargo.lock` — auto-updated.

---

## Task 1: Bump Version (Haiku)

**Files:**
- Modify: `Cargo.toml`
- Modify: `crates/presenter-ui/Cargo.toml`
- Modify: `Cargo.lock`, `crates/presenter-ui/Cargo.lock` (regenerated)

- [ ] **Step 1: Bump workspace version**

In `/home/newlevel/devel/presenter/presenter-dev2/Cargo.toml`, under `[workspace.package]`, change:

```toml
version = "0.4.53"
```

to:

```toml
version = "0.4.54"
```

- [ ] **Step 2: Bump presenter-ui crate version**

In `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui/Cargo.toml`, under `[package]`, change:

```toml
version = "0.1.22"
```

to:

```toml
version = "0.1.23"
```

- [ ] **Step 3: Regenerate workspace Cargo.lock**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo check --workspace --all-targets 2>&1 | tail -5
```

- [ ] **Step 4: Regenerate presenter-ui Cargo.lock**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui && cargo check --target wasm32-unknown-unknown 2>&1 | tail -5
```

- [ ] **Step 5: Verify**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && grep -E "^version" Cargo.toml crates/presenter-ui/Cargo.toml | head -3
```

Expected:
```
Cargo.toml:version = "0.4.54"
crates/presenter-ui/Cargo.toml:version = "0.1.23"
```

- [ ] **Step 6: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && git add Cargo.toml Cargo.lock crates/presenter-ui/Cargo.toml crates/presenter-ui/Cargo.lock && git commit -m "chore: bump version to 0.4.54 (#271)"
```

---

## Task 2: Lookahead Scroll + Load-at-Start (Sonnet)

**Files:**
- Modify: `crates/presenter-ui/src/components/slide_list.rs` — rewrite `scroll_slide_into_view`, add `scroll_slides_to_top` helper, add presentation-change Effect.

### Step 1: Read the current scroll function and Effect

Run:

```bash
sed -n '215,234p' /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui/src/components/slide_list.rs
sed -n '905,945p' /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui/src/components/slide_list.rs
```

Confirm the existing structure matches what's described in the plan context.

### Step 2: Rewrite `scroll_slide_into_view` with lookahead

In `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui/src/components/slide_list.rs`, replace the entire function (currently lines 905-944, starting `/// Assumes a vertically scrolling container...` and ending at the closing `}` of `fn scroll_slide_into_view`) with:

```rust
/// Number of columns in the `.operator__slides` grid (CSS:
/// `grid-template-columns: repeat(3, minmax(0, 1fr))`). The next-row anchor
/// for an active slide at index N is the slide at index N + COLUMNS_PER_ROW
/// in DOM order.
const COLUMNS_PER_ROW: usize = 3;

/// Lookahead-aware scroll: ensures the active slide AND the next row of
/// slides are visible in the `.operator__slides` container. If the active
/// slide is on the last row (no next-row anchor exists), falls back to
/// "ensure active is visible" behavior.
fn scroll_slide_into_view(slide_id: &str) {
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let active_selector = format!(".operator__slides [data-slide-id=\"{slide_id}\"]");
    let Ok(Some(active_el)) = document.query_selector(&active_selector) else {
        return;
    };
    let Ok(Some(container_el)) = active_el.closest(".operator__slides") else {
        return;
    };
    let Ok(container) = container_el.dyn_into::<web_sys::HtmlElement>() else {
        return;
    };
    let Ok(active_html) = active_el.dyn_into::<web_sys::HtmlElement>() else {
        return;
    };

    let container_rect = container.get_bounding_client_rect();
    let active_rect = active_html.get_bounding_client_rect();
    let scroll_top = container.scroll_top() as f64;

    // If the active slide is above the viewport, top-align it first
    // (covers backward navigation).
    if active_rect.top() < container_rect.top() {
        let delta = container_rect.top() - active_rect.top();
        container.set_scroll_top((scroll_top - delta) as i32);
        return;
    }

    // Find the next-row anchor: the slide whose DOM position is
    // active_index + COLUMNS_PER_ROW within the same container.
    let cards = container.query_selector_all("[data-slide-id]").ok();
    let next_row_el: Option<web_sys::HtmlElement> = cards.and_then(|nodes| {
        let mut active_index: Option<usize> = None;
        for i in 0..nodes.length() {
            if let Some(node) = nodes.item(i) {
                if let Ok(el) = node.dyn_into::<web_sys::Element>() {
                    if el.get_attribute("data-slide-id").as_deref() == Some(slide_id) {
                        active_index = Some(i as usize);
                        break;
                    }
                }
            }
        }
        let target_index = active_index? + COLUMNS_PER_ROW;
        nodes
            .item(target_index as u32)
            .and_then(|n| n.dyn_into::<web_sys::HtmlElement>().ok())
    });

    if let Some(anchor) = next_row_el {
        // Scroll so the next-row anchor's bottom is at the container's bottom.
        let anchor_rect = anchor.get_bounding_client_rect();
        if anchor_rect.bottom() > container_rect.bottom() {
            let delta = anchor_rect.bottom() - container_rect.bottom();
            container.set_scroll_top((scroll_top + delta) as i32);
        }
    } else if active_rect.bottom() > container_rect.bottom() {
        // No next-row anchor (last row) — fall back to bottom-aligning the active.
        let delta = active_rect.bottom() - container_rect.bottom();
        container.set_scroll_top((scroll_top + delta) as i32);
    }
}
```

Imports already present at the top of `slide_list.rs` should cover `web_sys`, `wasm_bindgen::JsCast` (for `dyn_into`). If a new import is needed, add `use wasm_bindgen::JsCast;` to the top of the file (verify it's not already imported with `grep "use wasm_bindgen::JsCast" crates/presenter-ui/src/components/slide_list.rs`).

### Step 3: Add `scroll_slides_to_top` helper

In `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui/src/components/slide_list.rs`, immediately AFTER the `scroll_slide_into_view` function (just added), append:

```rust
/// Scrolls the `.operator__slides` container to its top. Used when the
/// operator opens a new presentation so the first slide is visible without
/// manual scroll-up. Issue #271 concern 3.
fn scroll_slides_to_top() {
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let Ok(Some(container_el)) = document.query_selector(".operator__slides") else {
        return;
    };
    let Ok(container) = container_el.dyn_into::<web_sys::HtmlElement>() else {
        return;
    };
    container.set_scroll_top(0);
}
```

### Step 4: Add presentation-change Effect

In `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui/src/components/slide_list.rs`, find the existing Effect block at lines 215-234 (the one that watches `stage_snapshot.current_slide_id`). Immediately AFTER that closing brace (after line 234), insert this new Effect block:

```rust
    // Scroll to top when the operator opens a different presentation.
    // Issue #271 concern 3: new song should load with the first slide
    // visible, not at the previous song's scroll position.
    {
        let selected_presentation_id = ctx.selected_presentation_id;
        Effect::new(move |prev_id: Option<Option<String>>| {
            let current_id = selected_presentation_id.get();
            if current_id != prev_id.flatten() && current_id.is_some() {
                gloo_timers::callback::Timeout::new(0, scroll_slides_to_top).forget();
            }
            current_id
        });
    }
```

The `Timeout(0)` defers to the next event-loop tick so the new presentation's slides have rendered before the scroll. Same pattern as the existing scroll-into-view Effect.

### Step 5: Build the WASM crate

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui && cargo build --target wasm32-unknown-unknown 2>&1 | tail -10
```

Expected: clean build. If `JsCast` or `web_sys::HtmlElement` is unimported, add `use wasm_bindgen::JsCast;` at the top of slide_list.rs (the file already uses `web_sys::window()` etc., so most imports should be in place).

### Step 6: Run clippy

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings -W clippy::all 2>&1 | tail -10
```

Expected: zero warnings.

### Step 7: Run cargo fmt

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo fmt --all
```

### Step 8: Commit

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && git add crates/presenter-ui/src/components/slide_list.rs && git commit -m "feat(ui): lookahead slide scroll + scroll-to-top on song open (#271)"
```

---

## Task 3: Wheel Handler + CSS Boundary Guard (Sonnet)

**Files:**
- Modify: `crates/presenter-ui/src/components/slide_list.rs` — add `step_for_wheel` helper + `on:wheel` handler on the `.operator__slides` div.
- Modify: `crates/presenter-ui/styles/operator.css` — add `overscroll-behavior: contain` to `.operator__slides`.

### Step 1: Add `step_for_wheel` helper

In `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui/src/components/slide_list.rs`, immediately AFTER the `scroll_slides_to_top` function (added in Task 2), append:

```rust
/// Default fallback step for wheel scroll (pixels) when no slide card is
/// rendered yet to measure.
const DEFAULT_WHEEL_STEP_PX: f64 = 120.0;

/// Returns the pixel distance one wheel notch should scroll the
/// `.operator__slides` container. Measures the first rendered slide card's
/// height + the grid row gap so the step adapts to user font-size scaling.
/// Falls back to `DEFAULT_WHEEL_STEP_PX` if no card is rendered.
///
/// Issue #271 concern 2: linearises wheel scrolling to neutralise macOS
/// scroll acceleration.
fn step_for_wheel(container: &web_sys::HtmlElement) -> f64 {
    let Ok(Some(card_el)) = container.query_selector(".operator__slide-card") else {
        return DEFAULT_WHEEL_STEP_PX;
    };
    let Ok(card) = card_el.dyn_into::<web_sys::HtmlElement>() else {
        return DEFAULT_WHEEL_STEP_PX;
    };
    let card_height = card.get_bounding_client_rect().height();
    if card_height <= 0.0 {
        return DEFAULT_WHEEL_STEP_PX;
    }
    // Grid row gap from operator.css `.operator__slides`: `gap: 0.9rem`.
    // 0.9rem at 16px base = 14.4px. Hardcoded — if CSS changes, update here.
    card_height + 14.4
}
```

### Step 2: Add `on:wheel` handler to `.operator__slides`

In `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui/src/components/slide_list.rs`, find the `.operator__slides` `<div>` at line 291-293 (it has `class="operator__slides"`, `data-role="slides"`, `on:dragover`, `on:drop`). Add an `on:wheel` handler in the SAME element (between `data-role="slides"` and the existing `on:dragover`).

The current element opens (line 291) as:

```rust
                    <div
                        class="operator__slides"
                        data-role="slides"
                    on:dragover=move |ev: web_sys::DragEvent| {
```

Insert the new handler immediately AFTER `data-role="slides"` and BEFORE the existing `on:dragover`. The full new block:

```rust
                    <div
                        class="operator__slides"
                        data-role="slides"
                        on:wheel=move |ev: web_sys::WheelEvent| {
                            // Issue #271 concern 2: neutralise macOS scroll
                            // acceleration by intercepting wheel events and
                            // applying a deterministic per-notch scroll.
                            ev.prevent_default();
                            let direction = ev.delta_y().signum();
                            if direction == 0.0 {
                                return;
                            }
                            let Some(target) = ev.target() else { return; };
                            let Ok(el) = target.dyn_into::<web_sys::Element>() else { return; };
                            let Ok(Some(container_el)) = el.closest(".operator__slides") else { return; };
                            let Ok(container) = container_el.dyn_into::<web_sys::HtmlElement>() else { return; };
                            let step = step_for_wheel(&container);
                            container.set_scroll_top((container.scroll_top() as f64 + direction * step) as i32);
                        }
                    on:dragover=move |ev: web_sys::DragEvent| {
```

The handler reads the wheel direction (sign of deltaY), looks up the active container via the event target's closest ancestor (this works whether the wheel happened on a slide card or directly on the container), measures the step, and applies a single `set_scroll_top` call. Calling `prevent_default()` blocks the native accelerated scroll.

### Step 3: Verify Leptos's `on:wheel` is non-passive

Leptos 0.7's `on:` directive should default to passive=false for wheel events, but verify by running the dev binary after build. If `prevent_default()` is silently ignored (the native scroll still fires), Leptos may have set passive=true; in that case, switch to attaching the listener via `web_sys::Element::add_event_listener_with_callback_and_add_event_listener_options` with `passive: false`.

For Step 4's smoke test, this verification happens implicitly: if scroll feels accelerated still, prevent_default isn't working. Note the result in the report.

### Step 4: Add `overscroll-behavior: contain` to operator.css

In `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui/styles/operator.css`, find the `.operator__slides` rule at line 1081. Currently it reads:

```css
.operator__slides {
  flex: 1;
  overflow-y: auto;
  padding: 0.35rem;
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 0.9rem;
  min-height: 0;
}
```

Add `overscroll-behavior: contain;` immediately after `overflow-y: auto;`:

```css
.operator__slides {
  flex: 1;
  overflow-y: auto;
  overscroll-behavior: contain;
  padding: 0.35rem;
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 0.9rem;
  min-height: 0;
}
```

This prevents wheel events that exhaust the container's scroll from bubbling to the parent (which would scroll the page or cause the chrome rubber-band effect).

### Step 5: Build the WASM crate

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui && cargo build --target wasm32-unknown-unknown 2>&1 | tail -10
```

Expected: clean build.

### Step 6: Run clippy

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings -W clippy::all 2>&1 | tail -10
```

Expected: zero warnings. If clippy flags `dyn_into` chains with `let Ok(...)` patterns, those are the project's existing style — accept and move on.

### Step 7: Run cargo fmt

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo fmt --all
```

### Step 8: Commit

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && git add crates/presenter-ui/src/components/slide_list.rs crates/presenter-ui/styles/operator.css && git commit -m "feat(ui): linear wheel scroll + overscroll-behavior contain on slides (#271)"
```

---

## Task 4: Playwright E2E (Sonnet)

**Files:**
- Create: `tests/e2e/operator-slide-scroll.spec.ts`

The test exercises a 12+ slide worship song. The dev's seeded data includes `TYMY` library with 289 presentations of varying slide counts; the existing `support.ts` `refreshDevData` helper imports it. Pick a presentation with at least 10 slides (we'll find one at runtime).

### Step 1: Inspect existing E2E patterns

```bash
ls /home/newlevel/devel/presenter/presenter-dev2/tests/e2e/ | head -30
head -50 /home/newlevel/devel/presenter/presenter-dev2/tests/e2e/operator-controls.spec.ts
```

Confirm:
- Pattern uses `startTestServer`/`refreshDevData`/`stopServer` from `support.ts` in `beforeAll`/`afterAll`.
- `test.describe.configure({ timeout: 180_000 })` sets per-suite timeout.
- Each test captures console messages and asserts `consoleMessages.toEqual([])` at the end.
- Tests target `/ui/operator/worship` for the worship page.

### Step 2: Create the E2E test file

Create `/home/newlevel/devel/presenter/presenter-dev2/tests/e2e/operator-slide-scroll.spec.ts` with:

```typescript
/**
 * E2E tests for issue #271 — operator slide-list scroll UX.
 *
 * Three concerns:
 * 1. Lookahead: clicking a slide ensures the next row is visible below.
 * 2. Linear wheel: each wheel notch scrolls a deterministic step regardless
 *    of deltaY magnitude (neutralises macOS acceleration).
 * 3. Load-at-start: opening a new presentation scrolls the slide list to top.
 */

import { test, expect } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

let serverHandle: ServerHandle | undefined;
let baseURL: string;

test.describe.configure({ timeout: 180_000 });

test.beforeAll(async ({}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  baseURL = config.baseURL;
  await refreshDevData(config.dbUrl);
  serverHandle = await startTestServer(config.port, config.dbUrl, config.oscPort);
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
});

/**
 * Helper: pick the first presentation with at least `minSlides` slides
 * from the libraries summary endpoint.
 */
async function pickPresentationWithSlides(
  request: import("@playwright/test").APIRequestContext,
  baseURL: string,
  minSlides: number,
): Promise<{ libraryId: string; presentationId: string; slideCount: number }> {
  const libsRes = await request.get(
    new URL("/libraries/summary", baseURL).toString(),
  );
  const libs = (await libsRes.json()) as Array<{
    id: string;
    presentations: Array<{ id: string; slide_count: number }>;
  }>;
  for (const lib of libs) {
    for (const p of lib.presentations) {
      if (p.slide_count >= minSlides) {
        return { libraryId: lib.id, presentationId: p.id, slideCount: p.slide_count };
      }
    }
  }
  throw new Error(`No presentation with >= ${minSlides} slides found`);
}

test("lookahead: clicking a slide makes next row visible", async ({
  page,
  request,
}) => {
  const consoleMessages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  const target = await pickPresentationWithSlides(request, baseURL, 12);

  await page.goto(new URL("/ui/operator/worship", baseURL).toString());
  await page.waitForLoadState("networkidle");

  // Open the presentation programmatically by setting the selected_presentation
  // via the playlist API (reuses operator's normal flow). Simpler approach:
  // navigate via the URL/state pattern the operator UI uses. If a direct API
  // exists, prefer that; otherwise click through the library list to select.
  await page.evaluate(([presId]) => {
    // Project's session storage pattern stores currentPresentationId.
    sessionStorage.setItem("currentPresentationId", presId);
    location.reload();
  }, [target.presentationId]);

  await page.waitForLoadState("networkidle");
  await page.waitForSelector(".operator__slides [data-slide-id]", { state: "visible" });

  const cards = await page.locator(".operator__slides [data-slide-id]").all();
  expect(cards.length).toBeGreaterThanOrEqual(12);

  // Click the slide at index 3 (start of row 2 — 0-indexed: row 1 = 0-2, row 2 = 3-5).
  await cards[3].click();

  // Wait for the click to settle and the lookahead scroll to apply.
  await page.waitForTimeout(200);

  // Slide at index 6 (start of row 3) should be at least partially visible
  // (its bounding rect's bottom should be within the container's bottom).
  const visible = await page.evaluate(() => {
    const cards = document.querySelectorAll(".operator__slides [data-slide-id]");
    const container = document.querySelector(".operator__slides");
    if (!container || cards.length < 7) return null;
    const cRect = container.getBoundingClientRect();
    const lookahead = cards[6].getBoundingClientRect();
    return {
      lookaheadVisible: lookahead.bottom <= cRect.bottom + 1 && lookahead.top >= cRect.top - 1,
      lookaheadBottom: lookahead.bottom,
      containerBottom: cRect.bottom,
    };
  });
  expect(visible).not.toBeNull();
  expect(visible!.lookaheadVisible).toBeTruthy();

  expect(consoleMessages).toEqual([]);
});

test("wheel: each notch scrolls a deterministic step", async ({ page, request }) => {
  const consoleMessages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  const target = await pickPresentationWithSlides(request, baseURL, 12);

  await page.goto(new URL("/ui/operator/worship", baseURL).toString());
  await page.evaluate(([presId]) => {
    sessionStorage.setItem("currentPresentationId", presId);
    location.reload();
  }, [target.presentationId]);
  await page.waitForLoadState("networkidle");
  await page.waitForSelector(".operator__slides [data-slide-id]", { state: "visible" });

  // Reset scroll to top.
  await page.evaluate(() => {
    const c = document.querySelector(".operator__slides") as HTMLElement | null;
    if (c) c.scrollTop = 0;
  });
  await page.waitForTimeout(50);

  // Dispatch a wheel event with deltaY=100. Capture the resulting scrollTop
  // delta and the measured step (card height + 14.4). The scroll delta should
  // equal the step, NOT 100 (i.e. our handler ignored deltaY magnitude).
  const result = await page.evaluate(() => {
    const c = document.querySelector(".operator__slides") as HTMLElement | null;
    if (!c) return null;
    const cardEl = c.querySelector(".operator__slide-card") as HTMLElement | null;
    const expectedStep = cardEl ? cardEl.getBoundingClientRect().height + 14.4 : 120;
    const before = c.scrollTop;
    const ev = new WheelEvent("wheel", {
      deltaY: 100,
      bubbles: true,
      cancelable: true,
    });
    c.dispatchEvent(ev);
    return { before, after: c.scrollTop, expectedStep };
  });
  expect(result).not.toBeNull();
  // Scroll should advance by approximately expectedStep, not by 100.
  // Tolerance ±2px for sub-pixel rounding.
  const actualDelta = result!.after - result!.before;
  expect(actualDelta).toBeGreaterThan(result!.expectedStep - 2);
  expect(actualDelta).toBeLessThan(result!.expectedStep + 2);

  expect(consoleMessages).toEqual([]);
});

test("load-at-start: opening a new presentation scrolls slide list to top", async ({
  page,
  request,
}) => {
  const consoleMessages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  // Pick TWO presentations so we can switch.
  const libsRes = await request.get(
    new URL("/libraries/summary", baseURL).toString(),
  );
  const libs = (await libsRes.json()) as Array<{
    id: string;
    presentations: Array<{ id: string; slide_count: number }>;
  }>;
  const candidates: string[] = [];
  for (const lib of libs) {
    for (const p of lib.presentations) {
      if (p.slide_count >= 10) candidates.push(p.id);
      if (candidates.length >= 2) break;
    }
    if (candidates.length >= 2) break;
  }
  expect(candidates.length).toBeGreaterThanOrEqual(2);

  await page.goto(new URL("/ui/operator/worship", baseURL).toString());
  await page.evaluate(([presId]) => {
    sessionStorage.setItem("currentPresentationId", presId);
    location.reload();
  }, [candidates[0]]);
  await page.waitForLoadState("networkidle");
  await page.waitForSelector(".operator__slides [data-slide-id]", { state: "visible" });

  // Scroll the slide list to the bottom.
  await page.evaluate(() => {
    const c = document.querySelector(".operator__slides") as HTMLElement | null;
    if (c) c.scrollTop = c.scrollHeight;
  });
  await page.waitForTimeout(50);
  const scrolledDown = await page.evaluate(() => {
    const c = document.querySelector(".operator__slides") as HTMLElement | null;
    return c?.scrollTop ?? 0;
  });
  expect(scrolledDown).toBeGreaterThan(0);

  // Switch to the second presentation. The presentation-change Effect should
  // schedule scrollTop=0 after the new slides render.
  await page.evaluate(([presId]) => {
    // Trigger the same flow the operator uses to switch presentations.
    // Setting selected_presentation_id directly via the WASM context isn't
    // exposed; instead, we use sessionStorage + reload OR we click a different
    // presentation in the catalog. Here we use sessionStorage for determinism.
    sessionStorage.setItem("currentPresentationId", presId);
    location.reload();
  }, [candidates[1]]);
  await page.waitForLoadState("networkidle");
  await page.waitForSelector(".operator__slides [data-slide-id]", { state: "visible" });
  // Wait for the Timeout(0) scroll-to-top to settle.
  await page.waitForTimeout(200);

  const scrollAfterSwitch = await page.evaluate(() => {
    const c = document.querySelector(".operator__slides") as HTMLElement | null;
    return c?.scrollTop ?? -1;
  });
  expect(scrollAfterSwitch).toBe(0);

  expect(consoleMessages).toEqual([]);
});
```

### Step 3: Run the test locally

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && npx playwright test tests/e2e/operator-slide-scroll.spec.ts --reporter=list 2>&1 | tail -30
```

Expected: 3 tests pass.

If a test fails because:
- `sessionStorage` + `location.reload()` doesn't trigger the WASM router to load the presentation: fall back to clicking through the library catalog. The existing `operator-controls.spec.ts` may show how to programmatically select a presentation; mirror that pattern.
- `selected_presentation_id` isn't reactive on session-storage change: this is an internal WASM detail. If you discover the test pattern doesn't exercise the same code path as a real user click, switch to clicking through the catalog UI.
- `slide_count` field name in the libraries summary is different (e.g. `slideCount` due to camelCase rename): adjust the field access.

If the test reveals that Leptos's `on:wheel` IS passive and `prevent_default()` is silently ignored (the wheel test sees scroll delta of `100` not the step): escalate to Task 3's implementer; the fix is to use a manual `addEventListener` with `passive: false`.

### Step 4: Commit

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && git add tests/e2e/operator-slide-scroll.spec.ts && git commit -m "test(e2e): operator slide-list scroll UX (#271)"
```

---

## Task 5: Local Checks, Push, CI Monitor, Dev Verification, Open PR (Controller)

This task is handled by the controller. Local Rust + WASM builds allowed.

### Local pre-push checks

- [ ] **Step 1: Workspace fmt**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo fmt --all --check
```

- [ ] **Step 2: Workspace clippy**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo clippy --workspace --all-targets -- -D warnings -W clippy::all 2>&1 | tail -10
```

- [ ] **Step 3: presenter-ui WASM clippy**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings -W clippy::all 2>&1 | tail -5
```

- [ ] **Step 4: Workspace tests**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo test -p presenter-server 2>&1 | tail -5
```

Expected: all 187 tests pass (no server changes; this is a regression check).

- [ ] **Step 5: Push**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && git push origin dev
```

- [ ] **Step 6: Monitor CI**

Per `core/ci-monitoring.md`: ONE background `sleep + gh run view`.

```bash
RUN_ID=$(gh run list --branch dev --limit 1 --json databaseId --jq '.[0].databaseId')
sleep 1500 && gh run view $RUN_ID --json status,conclusion,jobs --jq '{status, conclusion, jobs: [.jobs[] | {name, status, conclusion}]}'
```

Wait for ALL jobs `completed`. If any fails, fix root cause in ONE commit, push, monitor again.

### Dev verification

- [ ] **Step 7: Verify dev shows v0.4.54**

```bash
curl -s http://10.77.8.134:8080/healthz
```

Expected: `{"channel":"dev","status":"ok","version":"0.4.54"}`.

- [ ] **Step 8: Manual UX verification on dev via Playwright MCP**

Open `http://10.77.8.134:8080/ui/operator/worship` with a multi-slide song. Test:

1. **Lookahead:** click a slide on row 2. Verify row 3 is visible.
2. **Wheel:** scroll the slide list with the wheel. Verify each notch advances ~1 row deterministically.
3. **Load-at-start:** scroll to bottom of one song, then switch to another. Verify the new song's slide list is at top.

### Open PR

- [ ] **Step 9: Open PR**

```bash
gh pr create --base main --head dev --title "feat(ui): operator slide-list scroll UX (#271)" --body "$(cat <<'EOF'
## Summary

Three operator slide-list scroll fixes for live worship services. Closes #271.

## What changed

- **Lookahead scroll:** when the active slide moves to a new row, the next row is now visible below it. `scroll_slide_into_view` now finds the next-row anchor (DOM index +3) and scrolls so its bottom is in view.
- **Linear wheel scroll:** new `on:wheel` handler on `.operator__slides` calls `prevent_default()` and applies a deterministic per-notch scroll (card height + row gap). Neutralises macOS scroll acceleration.
- **Load-at-start:** new Effect watches `ctx.selected_presentation_id`; on change, scrolls the slide list to top so the first slide is always visible when opening a new song.
- **CSS:** `overscroll-behavior: contain` on `.operator__slides` prevents wheel events at boundary from scrolling the page.
- Bumped version 0.4.53 → 0.4.54.

## Test plan

- [x] `cargo clippy --workspace --all-targets -- -D warnings -W clippy::all` — zero warnings
- [x] `cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings -W clippy::all` (presenter-ui) — zero warnings
- [x] `cargo fmt --all --check` — clean
- [x] `cargo test -p presenter-server` — all green (no server changes)
- [x] CI green on dev
- [x] **Manual dev verification:**
  - Click slide on row 2 → row 3 visible ✅
  - Wheel scroll → 1 row per notch deterministic ✅
  - Switch songs → new song at top ✅

Closes #271
EOF
)"
```

- [ ] **Step 10: Confirm PR clean**

```bash
PR_NUM=$(gh pr list --head dev --base main --json number --jq '.[0].number')
gh api repos/zbynekdrlik/presenter/pulls/$PR_NUM --jq '{mergeable: .mergeable, mergeable_state: .mergeable_state}'
```

Expected: `{"mergeable": true, "mergeable_state": "clean"}`.

### Pre-completion gate

- [ ] **Step 11: Run /plan-check**

- [ ] **Step 12: Run /review** on the PR diff

- [ ] **Step 13: Send completion report**

Per `core/completion-report.md`. Include CI run id, /plan-check fulfillment, /review 0🔴 0🟡 0🔵, dev verification of all 3 fixes, dev + prod URLs, PR URL.

---

## Verification Summary

| Check | How to verify |
|-------|---------------|
| Lookahead scroll | Click slide on row 2 → row 3 visible (Playwright + manual) |
| Linear wheel | dispatchEvent wheel → scrollTop delta = card height + 14.4 (not deltaY) |
| Load-at-start | Switch presentations → scrollTop = 0 |
| Wheel preventDefault works | If wheel test asserts deltaY-magnitude scroll, Leptos's on:wheel was passive — escalate |
| CSS guard | overscroll-behavior: contain prevents page bleed at scroll boundary |
| No regressions | All existing E2E tests still pass |
