# Timer Stage Layout: NDI Background Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Render the NDI MJPEG video as a backdrop on the `timer` stage layout, mirroring the existing `api_stage.rs` pattern.

**Architecture:** Add the same `<Show when=ndi_active>` MJPEG `<img>` + connection-status overlay structure to `TimerLayout` that `ApiStage` already uses. Add matching CSS rules. Add a Playwright E2E that verifies the `<img>` is present when an NDI source is active and absent when it isn't, mirroring `tests/e2e/stage-api-ndi.spec.ts`.

**Tech Stack:** Rust + Leptos (WASM), CSS, Playwright (TypeScript).

**Spec:** `docs/superpowers/specs/2026-05-07-timer-ndi-background-design.md` (commit d297b46)

---

## Context

Issue #306: the user wants the same NDI backdrop on `timer` that `api` and `ndi-fullscreen` already have. `StageContext` already exposes `ndi_active` and `ndi_status` signals; `pages/stage.rs` already wires them from live WebSocket events. Only the `timer` layout component, the CSS, and an E2E test need to change.

**Key existing code:**

- `crates/presenter-ui/src/components/stage/api_stage.rs` — canonical pattern (49 lines).
- `crates/presenter-ui/src/components/stage/timer_layout.rs` — current 53-line component, no NDI integration.
- `crates/presenter-ui/styles/stage.css:382-447` — existing `.stage-timer__display` and `.stage-timer__text` rules; section to extend.
- `crates/presenter-ui/styles/stage.css:521-553` — `.stage-api`, `.stage-api__ndi`, `.stage-api__overlay` rules to copy as `.stage-timer__ndi`, `.stage-timer__overlay`.
- `tests/e2e/stage-api-ndi.spec.ts` — closest analogue test; mirror its structure (server bootstrap, layout switch, video-source activate/deactivate, assertions).
- `crates/presenter-ui/src/state/stage.rs` (StageContext) — `ndi_active: RwSignal<bool>`, `ndi_status: RwSignal<String>`.

---

## File Structure

### Modified files

| File | Change |
|------|--------|
| `Cargo.toml` (workspace) | bump `[workspace.package].version` 0.4.72 → 0.4.73 |
| `crates/presenter-ui/src/components/stage/timer_layout.rs` | bind `ndi_active`/`ndi_status`, add MJPEG `<img>` and status overlay as siblings of the existing timer display |
| `crates/presenter-ui/styles/stage.css` | add `.stage-timer__ndi`, `.stage-timer__overlay`; extend `.stage-timer__display` and `.stage-timer__text` with z-index + text-shadow |

### Created files

| File | Responsibility |
|------|----------------|
| `tests/e2e/stage-timer-ndi.spec.ts` | E2E coverage: timer layout shows `img.stage-timer__ndi` when NDI source active; absent otherwise; timer text remains visible; clean console |

---

## Task 1: Version bump 0.4.72 → 0.4.73

**Files:**
- Modify: `Cargo.toml` (workspace `[workspace.package].version`)

- [ ] **Step 1: Edit version**

In `/home/newlevel/devel/presenter/presenter-dev2/Cargo.toml`, change:

```toml
version = "0.4.72"
```

to:

```toml
version = "0.4.73"
```

- [ ] **Step 2: Refresh Cargo.lock**

Run: `cargo update -w`

Expected output: lines for each workspace member updating from `v0.4.72` → `v0.4.73`. No errors.

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "chore: bump version to 0.4.73"
```

---

## Task 2: TimerLayout component change

**Files:**
- Modify: `crates/presenter-ui/src/components/stage/timer_layout.rs` (whole component body)

- [ ] **Step 1: Replace the component**

In `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui/src/components/stage/timer_layout.rs`, replace the entire `pub fn TimerLayout` body. The final file content is:

```rust
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::HtmlElement;

use crate::state::stage::StageContext;
use crate::utils::autofit::autofit_text;
use crate::ws::stage::StageWsState;

const TIMER_MAX_FONT: f64 = 300.0;

#[component]
pub fn TimerLayout(
    ws_state: ReadSignal<StageWsState>,
    latency_ms: ReadSignal<Option<f64>>,
) -> impl IntoView {
    let ctx = use_context::<StageContext>().expect("StageContext not provided");
    let ndi_active = ctx.ndi_active;
    let ndi_status = ctx.ndi_status;

    let timer_ref = NodeRef::<leptos::html::Div>::new();

    let timer_text = move || {
        ctx.snapshot
            .get()
            .map(|s| presenter_core::format_countdown(s.timers.countdown_to_start.seconds_remaining))
            .unwrap_or_else(|| "00:00".to_string())
    };

    {
        let r = timer_ref;
        Effect::new(move |_| {
            let _t = timer_text();
            if let Some(el) = r.get() {
                let html_el: &HtmlElement = &el;
                let el_clone = html_el.clone();
                let cb = wasm_bindgen::closure::Closure::once_into_js(move || {
                    autofit_text(&el_clone, TIMER_MAX_FONT);
                });
                let _ = web_sys::window()
                    .expect("window")
                    .request_animation_frame(cb.as_ref().unchecked_ref());
            }
        });
    }

    view! {
        <div class="stage-container" data-layout="timer">
            <Show when=move || ndi_active.get()>
                <img src="/ndi/mjpeg" class="stage-timer__ndi" />
            </Show>

            <Show when=move || {
                let status = ndi_status.get();
                status == "disconnected" || status == "connecting"
            }>
                <div class="stage-timer__overlay">
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

            <div class="stage-timer__display">
                <span class="stage__debug-label">"timer-display"</span>
                <div node_ref=timer_ref class="stage-timer__text">
                    {timer_text}
                </div>
            </div>
            <super::status_bar::StatusBar ws_state=ws_state latency_ms=latency_ms />
        </div>
    }
}
```

The diff vs. current:
- Two new `let` bindings after `let ctx = ...` for `ndi_active` and `ndi_status`.
- Two new `<Show>` blocks inserted as siblings BEFORE the existing `.stage-timer__display` div.
- Existing `.stage-timer__display`, `.stage-timer__text`, and `<StatusBar>` markup unchanged.

- [ ] **Step 2: Native + WASM build**

Run: `cd crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings -W clippy::all`

Expected: `Finished` with zero warnings. If `unused_imports` or similar fires, fix and re-run.

- [ ] **Step 3: Workspace clippy**

Run from repo root: `cargo clippy --workspace --all-targets -- -D warnings -W clippy::all`

Expected: `Finished` with zero warnings.

- [ ] **Step 4: Commit**

```bash
git add crates/presenter-ui/src/components/stage/timer_layout.rs
git commit -m "feat(stage): NDI background on timer layout (#306)"
```

---

## Task 3: CSS additions for `.stage-timer__ndi` and overlay

**Files:**
- Modify: `crates/presenter-ui/styles/stage.css` (add new rules; extend existing `.stage-timer__display` and `.stage-timer__text`)

- [ ] **Step 1: Add new rules**

In `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui/styles/stage.css`, find the timer section. The existing rules for `.stage-timer__display` and `.stage-timer__text` start near line 382. Add the new rules immediately after the existing timer rules (before the next `/* ===== ... ===== */` comment block).

Append:

```css
/* ===== NDI background for timer layout (#306) ===== */
.stage-timer__ndi {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
}

.stage-timer__overlay {
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
```

- [ ] **Step 2: Extend `.stage-timer__display`**

In the same file, find the existing `.stage-timer__display { ... }` rule (near line 382). Add `position: relative;` and `z-index: 2;` to its declarations so the timer text sits above the NDI img. The block becomes:

```css
.stage-timer__display {
    /* keep all existing properties */
    position: relative;
    z-index: 2;
}
```

If `position: relative` is already present, leave it; just add `z-index: 2`. Do not remove or change any existing property.

- [ ] **Step 3: Extend `.stage-timer__text`**

In the same file, find `.stage-timer__text { ... }` (near line 394). Add a `text-shadow` so digits stay readable over varied video. Append (do not replace existing properties):

```css
.stage-timer__text {
    /* keep all existing properties */
    text-shadow:
        0 0 8px rgba(0, 0, 0, 0.9),
        0 2px 4px rgba(0, 0, 0, 0.7);
}
```

- [ ] **Step 4: Commit**

```bash
git add crates/presenter-ui/styles/stage.css
git commit -m "style(stage): NDI backdrop CSS for timer layout (#306)"
```

---

## Task 4: Playwright E2E test

**Files:**
- Create: `tests/e2e/stage-timer-ndi.spec.ts`

- [ ] **Step 1: Create the test file**

Create `/home/newlevel/devel/presenter/presenter-dev2/tests/e2e/stage-timer-ndi.spec.ts` with:

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

test("timer layout renders without NDI image when no source is active", async ({ page }) => {
  const consoleMessages = collectConsoleErrors(page);

  await page.request.post(
    new URL("/integrations/video-sources/deactivate", baseURL).toString(),
  );

  await page.request.post(
    new URL("/stage/layout", baseURL).toString(),
    { data: { code: "timer" } },
  );

  await page.goto(new URL("/stage", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await page.waitForSelector('body[data-layout-code="timer"]', {
    timeout: 10_000,
  });

  // Timer wrapper present
  const wrapper = page.locator('div.stage-container[data-layout="timer"]');
  await expect(wrapper).toBeAttached();

  // Timer display still rendered
  await expect(wrapper.locator(".stage-timer__display")).toBeAttached();
  await expect(wrapper.locator(".stage-timer__text")).toBeVisible();

  // No NDI image when no source is active
  await expect(wrapper.locator("img.stage-timer__ndi")).toHaveCount(0);

  // Timer text has the legibility shadow we added
  const textShadow = await wrapper
    .locator(".stage-timer__text")
    .evaluate((el) => window.getComputedStyle(el).textShadow);
  expect(textShadow).not.toBe("none");
  expect(textShadow).not.toBe("");

  expect(consoleMessages).toEqual([]);
});

test("timer layout renders NDI image when an NDI source is active", async ({ page }) => {
  // The /ndi/mjpeg endpoint returns 503 for a bogus source name; that 503 is
  // expected noise for this scenario only. See stage-api-ndi.spec.ts for the
  // same allowance.
  const consoleMessages = collectConsoleErrors(page, [
    /Failed to load resource.*503/i,
  ]);

  await page.request.post(
    new URL("/integrations/video-sources/deactivate", baseURL).toString(),
  );

  await page.request.post(
    new URL("/stage/layout", baseURL).toString(),
    { data: { code: "timer" } },
  );

  await page.goto(new URL("/stage", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await page.waitForSelector('body[data-layout-code="timer"]', {
    timeout: 10_000,
  });

  // Create + activate a bogus NDI source so the WS event flips ndi_active.
  const createResp = await page.request.post(
    new URL("/integrations/video-sources", baseURL).toString(),
    { data: { name: "test-ndi-source", ndi_name: "BOGUS-FOR-TIMER-TEST" } },
  );
  expect(createResp.ok()).toBeTruthy();
  const source = await createResp.json();

  try {
    const activateResp = await page.request.post(
      new URL(
        `/integrations/video-sources/${source.id}/activate`,
        baseURL,
      ).toString(),
    );
    expect(activateResp.ok()).toBeTruthy();

    const wrapper = page.locator('div.stage-container[data-layout="timer"]');
    await expect(wrapper.locator("img.stage-timer__ndi")).toBeVisible({
      timeout: 10_000,
    });

    // Timer text remains visible and is on top of the video
    await expect(wrapper.locator(".stage-timer__text")).toBeVisible();

    const zIndex = await wrapper
      .locator(".stage-timer__display")
      .evaluate((el) => window.getComputedStyle(el).zIndex);
    expect(Number(zIndex)).toBeGreaterThanOrEqual(2);
  } finally {
    await page.request.post(
      new URL("/integrations/video-sources/deactivate", baseURL).toString(),
    );
    await page.request.delete(
      new URL(
        `/integrations/video-sources/${source.id}`,
        baseURL,
      ).toString(),
    );
  }

  expect(consoleMessages).toEqual([]);
});
```

- [ ] **Step 2: Run the new test locally on dev**

The project uses a Playwright config that bootstraps its own server. Run:

```bash
npm run test:playwright -- stage-timer-ndi
```

Expected: 2 tests pass. If `npm run test:playwright` is not the right command, check `package.json` `scripts` for `playwright`/`test:e2e`/`pw` and use that. If it requires the dev server to be running, start it per `docs/ops/runbook.md` first.

If a video-source create/delete step fails because of a different API shape on this branch, inspect `tests/e2e/stage-api-ndi.spec.ts:171-208` — it uses the same endpoints; copy the exact request shape from there.

- [ ] **Step 3: Commit**

```bash
git add tests/e2e/stage-timer-ndi.spec.ts
git commit -m "test(e2e): timer layout NDI background coverage (#306)"
```

---

## Task 5: Push, monitor CI, manual verify on dev, open PR (controller-handled)

This task is performed by the orchestrator, not a subagent.

- [ ] **Step 1: Final local checks**

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all
cargo test --workspace -- --nocapture
cd crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings -W clippy::all && cd ../..
```

All must pass.

- [ ] **Step 2: Push to dev**

```bash
git push origin dev
```

If the push reports `Branch Sync Check` will fail, sync first:

```bash
git fetch origin
git merge origin/main --no-edit
git push origin dev
```

- [ ] **Step 3: Capture the run id and monitor**

```bash
gh run list --branch dev --limit 1 --json databaseId
# Then in a single background bash:
sleep 1500 && gh run view <run-id> --json status,conclusion,jobs
```

Wait until ALL jobs (including Mutation Testing) report `success`. If any fail, run `gh run view <run-id> --log-failed`, fix in ONE commit, push once.

- [ ] **Step 4: Verify on dev**

After Deploy to Dev is green:

```bash
curl -s 'http://10.77.8.134:8080/healthz'
# Expect: {"channel":"dev","status":"ok","version":"0.4.73"}
```

Then in Playwright (or by opening a real browser pointed at the dev IP):

1. POST `http://10.77.8.134:8080/stage/layout` with `{"code":"timer"}`.
2. Open `http://10.77.8.134:8080/stage`.
3. If a real NDI source is configured on dev, activate it; assert visually that countdown digits sit over the live video. If no real NDI source is reachable, the unit/E2E coverage from Task 4 is the canonical proof.

- [ ] **Step 5: Open PR**

```bash
gh pr create --base main --head dev --title "feat(stage): NDI background on timer layout (#306)" --body "$(cat <<'EOF'
## Summary

Closes #306: the `timer` stage layout now renders the live NDI MJPEG video as a backdrop, the same way `api` and `ndi-fullscreen` already do.

## What changed

- `TimerLayout` (Leptos) — added `<Show when=ndi_active>` MJPEG `<img>` and a connection-status overlay as siblings of the existing timer display. Mirrors `ApiStage` exactly.
- `stage.css` — new `.stage-timer__ndi` and `.stage-timer__overlay` rules; extended `.stage-timer__display` with `z-index: 2` and `.stage-timer__text` with a text-shadow for legibility over varied video.
- New Playwright E2E `stage-timer-ndi.spec.ts` — covers active-NDI image render, no-NDI image absence, timer text visibility, browser-console-zero-errors.
- Version bump 0.4.72 → 0.4.73.

## Verification

- All local checks green (`cargo test`, native + WASM clippy, fmt).
- CI green incl. Mutation Testing.
- Dev `/healthz` reports v0.4.73.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

Wait for `mergeStateStatus: CLEAN` and `mergeable: MERGEABLE` before reporting. Do NOT merge until the user says "merge it".

---

## Verification summary

| Check | How to verify |
|-------|---------------|
| TimerLayout renders NDI img when active | `npm run test:playwright -- stage-timer-ndi` |
| TimerLayout omits NDI img when inactive | Same test |
| Timer text remains visible and elevated | Same test (z-index assertion) |
| Browser console clean | Same test (`expect(consoleMessages).toEqual([])`) |
| Workspace builds | `cargo build --workspace` green |
| Native clippy clean | `cargo clippy --workspace --all-targets -- -D warnings -W clippy::all` |
| WASM clippy clean | from `crates/presenter-ui`: `cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings -W clippy::all` |
| Version bump | `Cargo.toml` workspace version is `0.4.73` |
| Dev deploy | `/healthz` shows v0.4.73 |
