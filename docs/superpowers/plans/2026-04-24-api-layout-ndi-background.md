# API Layout NDI Background Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Render live NDI video as the background of the `api` stage layout, with white lyric text (from `WorshipSnv`) layered on top with a soft text-shadow for legibility.

**Architecture:** New `ApiStage` component wraps the existing `WorshipSnv` component and adds a sibling `<img src="/ndi/mjpeg">` layer + status overlay. Stage router dispatches `"api"` to `ApiStage` explicitly instead of falling through to `WorshipSnv`. `worship-snv` layout is untouched. All NDI state (active/connecting/disconnected) comes from the existing `StageContext` signals — no new server endpoints.

**Tech Stack:** Rust (Leptos WASM), CSS, TypeScript (Playwright)

**Spec:** `docs/superpowers/specs/2026-04-24-api-layout-ndi-background-design.md`

---

## Context

The `api` stage layout (`PUT /api/stage`) currently falls through the catch-all arm in `crates/presenter-ui/src/pages/stage.rs:167-169` and renders the same `WorshipSnv` component as `worship-snv`. We want a live NDI video behind the six worship boxes when a video source is active, without disturbing the `worship-snv` layout that's in production use.

**Key existing code:**

- `crates/presenter-ui/src/pages/stage.rs` — layout routing (`match code.as_str()`).
- `crates/presenter-ui/src/components/stage/worship_snv.rs` — 6-box rendering (reused as-is).
- `crates/presenter-ui/src/components/stage/ndi_fullscreen.rs` — reference for NDI image + overlay pattern.
- `crates/presenter-ui/src/state/stage.rs` — `StageContext` with `ndi_active: RwSignal<bool>` and `ndi_status: RwSignal<String>`.
- `crates/presenter-ui/styles/stage.css` — stage stylesheet.
- `tests/e2e/ndi-stage-layout.spec.ts` — reference test for NDI-related stage flows.
- `tests/e2e/support.ts` — `startTestServer`, `deriveTestConfig`, `refreshDevData`, `stopServer`.

---

## File Structure

### New files

| File | Purpose |
|------|---------|
| `crates/presenter-ui/src/components/stage/api_stage.rs` | `ApiStage` component: `<div class="stage-api">` wrapper that renders NDI image, status overlay, and nested `<WorshipSnv>`. |
| `tests/e2e/stage-api-ndi.spec.ts` | Playwright E2E verifying api-layout rendering (wrapper, no-source state, text-shadow) and regression guard for `worship-snv`. |

### Modified files

| File | Change |
|------|--------|
| `Cargo.toml` | Bump `[workspace.package].version` from `0.4.32` to `0.4.33`. |
| `crates/presenter-ui/src/components/stage/mod.rs` | Add `pub mod api_stage;`. |
| `crates/presenter-ui/src/pages/stage.rs` | Import `ApiStage`, add explicit `"api"` arm in the layout match. |
| `crates/presenter-ui/styles/stage.css` | Add `.stage-api`, `.stage-api__ndi`, `.stage-api__overlay`, and `.stage-api .stage__slide-text` rules. |

---

## Task 1: Version Bump

**Files:**
- Modify: `Cargo.toml:15`

- [ ] **Step 1: Bump workspace version**

Edit `Cargo.toml` line 15 — change `version = "0.4.32"` to `version = "0.4.33"`.

- [ ] **Step 2: Refresh Cargo.lock**

Run: `cargo check --workspace --quiet`
Expected: Cargo rewrites `Cargo.lock` with the new version; command succeeds.

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "chore: bump version to 0.4.33 for api layout NDI background"
```

---

## Task 2: Failing E2E Test for ApiStage Wrapper

**Files:**
- Create: `tests/e2e/stage-api-ndi.spec.ts`

- [ ] **Step 1: Create the test file with one failing assertion**

Create `tests/e2e/stage-api-ndi.spec.ts` with this exact content:

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

test("api layout renders ApiStage wrapper with no NDI source active", async ({ page }) => {
  const consoleMessages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  // Ensure no video source is active
  await page.request.post(
    new URL("/integrations/video-sources/deactivate", baseURL).toString(),
  );

  // Switch stage to api layout
  await page.request.post(
    new URL("/stage/layout", baseURL).toString(),
    { data: { code: "api" } },
  );

  await page.goto(new URL("/stage", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await page.waitForSelector('body[data-layout-code="api"]', {
    timeout: 10_000,
  });

  // ApiStage wrapper must be in the DOM
  const wrapper = page.locator("div.stage-api");
  await expect(wrapper).toBeAttached();

  // No NDI image when no source is active
  const img = page.locator("img.stage-api__ndi");
  await expect(img).toHaveCount(0);

  // WorshipSnv content is nested inside the wrapper
  const slide = page.locator("div.stage-api .stage__current-slide");
  await expect(slide).toBeAttached();

  expect(consoleMessages).toEqual([]);
});
```

- [ ] **Step 2: Run the test — confirm it FAILS**

Run: `npm run test:playwright -- stage-api-ndi`
Expected: FAIL. The locator `div.stage-api` has zero matches because `"api"` currently routes to `WorshipSnv`, which renders `<div class="stage-container" data-layout="worship-snv">`.

- [ ] **Step 3: Commit**

```bash
git add tests/e2e/stage-api-ndi.spec.ts
git commit -m "test(stage): add failing E2E for api layout NDI wrapper"
```

---

## Task 3: Create ApiStage Component and Route

**Files:**
- Create: `crates/presenter-ui/src/components/stage/api_stage.rs`
- Modify: `crates/presenter-ui/src/components/stage/mod.rs:1-8`
- Modify: `crates/presenter-ui/src/pages/stage.rs:6-9, 148-172`

- [ ] **Step 1: Create the ApiStage component**

Create `crates/presenter-ui/src/components/stage/api_stage.rs` with this exact content:

```rust
use leptos::prelude::*;

use crate::components::stage::worship_snv::WorshipSnv;
use crate::state::stage::StageContext;
use crate::ws::stage::StageWsState;

/// Stage layout for API-driven slides with an optional live NDI video background.
///
/// Wraps `WorshipSnv` and adds a sibling `<img src="/ndi/mjpeg">` layer that
/// renders only when a video source is active (driven by
/// `StageContext::ndi_active`). Also surfaces the NDI connection status
/// overlay for the "connecting" / "disconnected" states.
#[component]
pub fn ApiStage(
    ws_state: ReadSignal<StageWsState>,
    latency_ms: ReadSignal<Option<f64>>,
) -> impl IntoView {
    let ctx = use_context::<StageContext>().expect("StageContext not provided");
    let ndi_active = ctx.ndi_active;
    let ndi_status = ctx.ndi_status;

    view! {
        <div class="stage-api">
            <Show when=move || ndi_active.get()>
                <img src="/ndi/mjpeg" class="stage-api__ndi" />
            </Show>

            <Show when=move || {
                let status = ndi_status.get();
                status == "disconnected" || status == "connecting"
            }>
                <div class="stage-api__overlay">
                    {move || {
                        let status = ndi_status.get();
                        if status == "disconnected" {
                            "Signal Lost — Reconnecting..."
                        } else if status == "connecting" {
                            "Connecting..."
                        } else {
                            ""
                        }
                    }}
                </div>
            </Show>

            <WorshipSnv ws_state=ws_state latency_ms=latency_ms />
        </div>
    }
}
```

- [ ] **Step 2: Register the module**

Edit `crates/presenter-ui/src/components/stage/mod.rs`. Replace the whole file with:

```rust
pub mod api_stage;
pub mod bible_layout;
pub mod bible_overlay;
pub mod ndi_fullscreen;
pub mod preach_layout;
pub mod status_bar;
pub mod timer_layout;
pub mod worship_pp;
pub mod worship_snv;
```

- [ ] **Step 3: Add ApiStage to the import list in stage.rs**

In `crates/presenter-ui/src/pages/stage.rs`, replace lines 6-9:

```rust
use crate::components::stage::{
    bible_layout::BibleLayout, ndi_fullscreen::NdiFullscreen, preach_layout::PreachLayout,
    timer_layout::TimerLayout, worship_pp::WorshipPp, worship_snv::WorshipSnv,
};
```

with:

```rust
use crate::components::stage::{
    api_stage::ApiStage, bible_layout::BibleLayout, ndi_fullscreen::NdiFullscreen,
    preach_layout::PreachLayout, timer_layout::TimerLayout, worship_pp::WorshipPp,
    worship_snv::WorshipSnv,
};
```

- [ ] **Step 4: Add the "api" arm in the layout match**

In `crates/presenter-ui/src/pages/stage.rs`, replace the match block at lines 151-170 with:

```rust
            match code.as_str() {
                "worship-pp" => {
                    view! { <WorshipPp ws_state=ws_state latency_ms=latency_ms /> }.into_any()
                }
                "timer" => {
                    view! { <TimerLayout ws_state=ws_state latency_ms=latency_ms /> }.into_any()
                }
                "preach" => {
                    view! { <PreachLayout ws_state=ws_state latency_ms=latency_ms /> }.into_any()
                }
                "ndi-fullscreen" => {
                    view! { <NdiFullscreen ws_state=ws_state latency_ms=latency_ms /> }.into_any()
                }
                "bible" => {
                    view! { <BibleLayout ws_state=ws_state latency_ms=latency_ms /> }.into_any()
                }
                "api" => {
                    view! { <ApiStage ws_state=ws_state latency_ms=latency_ms /> }.into_any()
                }
                _ => {
                    view! { <WorshipSnv ws_state=ws_state latency_ms=latency_ms /> }.into_any()
                }
            }
```

- [ ] **Step 5: Build the WASM bundle**

Run: `cd crates/presenter-ui && trunk build --release && cd ../..`
Expected: build succeeds; `crates/presenter-ui/dist/` updated.

- [ ] **Step 6: Run the E2E test — confirm PASS**

Run: `npm run test:playwright -- stage-api-ndi`
Expected: PASS. `div.stage-api` present, `img.stage-api__ndi` absent (no active source), `.stage-api .stage__current-slide` present, zero console errors.

- [ ] **Step 7: Commit**

```bash
cargo fmt --all
git add crates/presenter-ui/src/components/stage/api_stage.rs \
        crates/presenter-ui/src/components/stage/mod.rs \
        crates/presenter-ui/src/pages/stage.rs
git commit -m "feat(stage): add ApiStage component with NDI background layer"
```

---

## Task 4: CSS for .stage-api and Text-Shadow

**Files:**
- Modify: `crates/presenter-ui/styles/stage.css` (append at end of file)
- Modify: `tests/e2e/stage-api-ndi.spec.ts` (extend existing test with text-shadow assertion)

- [ ] **Step 1: Extend the E2E test with text-shadow and wrapper-CSS assertions**

In `tests/e2e/stage-api-ndi.spec.ts`, add these two assertions **before** the final `expect(consoleMessages).toEqual([]);` line in the existing `api layout renders ApiStage wrapper...` test:

```typescript
  // Wrapper should be absolutely sized to viewport
  const wrapperStyle = await wrapper.evaluate((el) => {
    const cs = window.getComputedStyle(el);
    return {
      position: cs.position,
      width: cs.width,
      height: cs.height,
    };
  });
  expect(wrapperStyle.position).toBe("relative");

  // Slide text inside .stage-api must have a non-empty text-shadow
  const slideShadow = await page
    .locator("div.stage-api .stage__current-slide .stage__slide-text")
    .evaluate((el) => window.getComputedStyle(el).textShadow);
  expect(slideShadow).not.toBe("none");
  expect(slideShadow).not.toBe("");
```

- [ ] **Step 2: Run the test — confirm it FAILS on the new assertions**

Run: `npm run test:playwright -- stage-api-ndi`
Expected: FAIL. The `textShadow` computed style is `"none"` because no CSS rule matches `.stage-api .stage__slide-text` yet.

- [ ] **Step 3: Append the CSS rules**

Open `crates/presenter-ui/styles/stage.css` and append to the end of the file:

```css

/* ===== API stage layout (NDI background) ===== */
.stage-api {
    position: relative;
    width: 100vw;
    height: 100vh;
    background: #000;
    overflow: hidden;
}

.stage-api__ndi {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
}

.stage-api__overlay {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    background: rgba(0, 0, 0, 0.7);
    color: #ef4444;
    font-size: 1.2rem;
    z-index: 1;
}

.stage-api .stage__slide-text {
    text-shadow:
        0 0 8px rgba(0, 0, 0, 0.9),
        0 2px 4px rgba(0, 0, 0, 0.7);
}
```

- [ ] **Step 4: Rebuild the WASM bundle (so the CSS is bundled into dist)**

Run: `cd crates/presenter-ui && trunk build --release && cd ../..`
Expected: build succeeds.

- [ ] **Step 5: Run the test — confirm PASS**

Run: `npm run test:playwright -- stage-api-ndi`
Expected: PASS. Both the wrapper position and the text-shadow assertions now pass.

- [ ] **Step 6: Commit**

```bash
git add crates/presenter-ui/styles/stage.css tests/e2e/stage-api-ndi.spec.ts
git commit -m "style(stage): add .stage-api wrapper and text-shadow for api layout"
```

---

## Task 5: Worship-snv Regression Guard

**Files:**
- Modify: `tests/e2e/stage-api-ndi.spec.ts`

- [ ] **Step 1: Add a regression test ensuring worship-snv is untouched**

In `tests/e2e/stage-api-ndi.spec.ts`, append a new test after the existing one (after the closing `});` of the first test):

```typescript
test("worship-snv layout is not affected by api stage changes", async ({ page }) => {
  const consoleMessages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  // Switch back to worship-snv
  await page.request.post(
    new URL("/stage/layout", baseURL).toString(),
    { data: { code: "worship-snv" } },
  );

  await page.goto(new URL("/stage", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await page.waitForSelector('body[data-layout-code="worship-snv"]', {
    timeout: 10_000,
  });

  // No api wrapper
  await expect(page.locator("div.stage-api")).toHaveCount(0);
  await expect(page.locator("img.stage-api__ndi")).toHaveCount(0);

  // Worship-snv slide text must NOT have a text-shadow (only api layout gets it)
  const slideShadow = await page
    .locator('div.stage-container[data-layout="worship-snv"] .stage__current-slide .stage__slide-text')
    .evaluate((el) => window.getComputedStyle(el).textShadow);
  expect(slideShadow).toBe("none");

  expect(consoleMessages).toEqual([]);
});
```

- [ ] **Step 2: Run the test — confirm PASS**

Run: `npm run test:playwright -- stage-api-ndi`
Expected: Both tests PASS. The regression test confirms `worship-snv` has no `.stage-api` wrapper and no text-shadow applied.

- [ ] **Step 3: Commit**

```bash
git add tests/e2e/stage-api-ndi.spec.ts
git commit -m "test(stage): add worship-snv regression guard for api layout changes"
```

---

## Task 6: Local Verification (fmt, clippy, tests, full build)

**Files:** none modified in this task unless checks fail.

- [ ] **Step 1: Format check**

Run: `cargo fmt --all --check`
Expected: no diff. If there's drift, run `cargo fmt --all` and re-commit as a `style: cargo fmt` commit.

- [ ] **Step 2: Clippy on workspace**

Run: `cargo clippy --workspace --all-targets -- -D warnings -W clippy::all`
Expected: no warnings, no errors. Note: the workspace excludes `crates/presenter-ui`, so this does NOT cover ApiStage.

- [ ] **Step 3: Clippy on presenter-ui (outside workspace)**

Run: `cd crates/presenter-ui && cargo clippy --all-targets -- -D warnings -W clippy::all && cd ../..`
Expected: no warnings, no errors. This is the check that validates `api_stage.rs`.

- [ ] **Step 4: Rust test**

Run: `cargo test --workspace`
Expected: all pass.

- [ ] **Step 5: presenter-ui lib test (outside workspace)**

Run: `cd crates/presenter-ui && cargo test --lib && cd ../..`
Expected: all pass.

- [ ] **Step 6: Full WASM release build**

Run: `cd crates/presenter-ui && trunk build --release && cd ../..`
Expected: build succeeds; `dist/` populated.

- [ ] **Step 7: Release server build**

Run: `cargo build --release -p presenter-server`
Expected: build succeeds.

- [ ] **Step 8: Final Playwright run for all stage-api tests**

Run: `npm run test:playwright -- stage-api-ndi`
Expected: both tests PASS, zero console errors.

- [ ] **Step 9: If any local check produced fixable drift (e.g. cargo fmt), commit it in one batch**

```bash
# Only if changes are needed
cargo fmt --all
git add -u
git commit -m "style: cargo fmt"
```

---

## Task 7: Push, Monitor CI, Open PR, Deploy Verify

**Files:** none (this task is entirely about push/PR/verify).

- [ ] **Step 1: Fetch and merge main (avoid Branch Sync Check failures)**

Run:

```bash
git fetch origin
git merge origin/main --no-edit
```

Expected: fast-forward or clean merge. Resolve any conflicts if present.

- [ ] **Step 2: Push to dev**

Run: `git push origin dev`
Expected: push succeeds, triggering the CI pipeline.

- [ ] **Step 3: Monitor CI to terminal state**

Run: `gh run list --branch dev --limit 3`
Identify the latest run ID.

Run (in background): `sleep 300 && gh run view <run-id> --json status,conclusion,jobs`
Wait for the notification, then check all jobs.

Expected: ALL jobs succeed (`checks → test → coverage → build → e2e → deploy-dev`). If any fail, run `gh run view <run-id> --log-failed`, fix ALL issues in ONE commit, push again, re-monitor.

- [ ] **Step 4: Verify the dev deploy works**

Run: `curl -s http://10.77.8.134:8080/healthz`
Expected: JSON response with `"version":"0.4.33"` and `"channel":"dev"`.

- [ ] **Step 5: Verify api layout on dev via Playwright MCP**

Set the dev stage layout to `api`:

```bash
curl -X POST http://10.77.8.134:8080/stage/layout \
  -H "Content-Type: application/json" \
  -d '{"code":"api"}'
```

Use the Playwright MCP (`mcp__plugin_playwright_playwright__browser_navigate`) to open `http://10.77.8.134:8080/stage`, then evaluate in the page:

```javascript
({
  layout: document.body.dataset.layoutCode,
  hasApiWrapper: !!document.querySelector('div.stage-api'),
  hasNdiImg: !!document.querySelector('img.stage-api__ndi'),
  slideShadow: getComputedStyle(document.querySelector('.stage-api .stage__slide-text') || document.body).textShadow,
})
```

Expected: `layout === "api"`, `hasApiWrapper === true`, `slideShadow` non-empty. `hasNdiImg` will be `true` only if an NDI source is currently active on the dev server. Confirm zero browser console errors via `mcp__plugin_playwright_playwright__browser_console_messages`.

- [ ] **Step 6: Open a PR from dev to main**

Run:

```bash
gh pr create --base main --head dev --title "feat(stage): NDI background for api layout" --body "$(cat <<'EOF'
## Summary
- New `ApiStage` component wraps `WorshipSnv` and adds a live NDI video backdrop for the `api` stage layout
- White lyric text gets a soft dark text-shadow for legibility over video
- `worship-snv` layout is unchanged (regression-guarded by E2E)

## Spec
docs/superpowers/specs/2026-04-24-api-layout-ndi-background-design.md

## Test plan
- [x] Playwright: api layout renders `.stage-api` wrapper with no source active, no `img.stage-api__ndi`, text-shadow applied
- [x] Playwright: worship-snv regression guard (no `.stage-api`, no text-shadow)
- [ ] Manual post-deploy: activate a real NDI source, confirm video visible under lyrics on `/stage?layout=api`

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 7: Verify the PR is mergeable**

Run: `gh pr view --json mergeable,mergeStateStatus`
Expected: `mergeable: true`, `mergeStateStatus: "CLEAN"` (or `"BLOCKED"` only for required-review gating, not for failed checks).

- [ ] **Step 8: Report the PR URL and wait for explicit merge approval**

Do NOT merge. Report the green PR URL to the user and wait for explicit "merge it" before proceeding to production deploy.

---

## Verification Summary

| Requirement (from spec) | Covered by |
|-------------------------|------------|
| New `ApiStage` component | Task 3 Step 1 |
| `"api"` routes to `ApiStage` (not `WorshipSnv` catch-all) | Task 3 Step 4 |
| `WorshipSnv` nested inside `ApiStage` unchanged | Task 3 Step 1 (imports & uses as child) |
| `<img src="/ndi/mjpeg">` rendered only when `ndi_active` | Task 3 Step 1 (`<Show>`) + Task 2 Step 1 (assert absent) |
| Status overlay for connecting/disconnected | Task 3 Step 1 (second `<Show>`) |
| CSS `.stage-api`, `.stage-api__ndi`, `.stage-api__overlay` | Task 4 Step 3 |
| Text-shadow on api-layout slide text only | Task 4 Step 3 + E2E assertion Task 4 Step 1 |
| `worship-snv` untouched | Task 5 (regression guard) |
| No new server endpoints, reuses `/ndi/mjpeg` | Task 3 (component-only change) |
| Browser console zero errors | Every E2E asserts this |
| Version bumped | Task 1 |
| CI green end-to-end | Task 7 |
