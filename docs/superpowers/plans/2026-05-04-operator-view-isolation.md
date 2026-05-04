# Operator View Isolation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Hide worship-specific UI elements (song name, Ableton/Follow buttons, libraries / playlists / presentations panels with their `+` buttons) on the bible / timers / ai / settings views so the operator only sees them on the worship view.

**Architecture:** Two surgical changes — a Leptos inline-style condition flip in `stage_preview.rs` and one new CSS rule in `operator.css`. No new components, no logic changes. Add a Playwright E2E spec asserting the visibility matrix per view.

**Tech Stack:** Rust/Leptos (WASM), CSS, Playwright TypeScript.

**Spec:** `docs/superpowers/specs/2026-05-04-operator-view-isolation-design.md` (commit 3965546).

---

## Context

Issue #295 (title-only): "worship song name and plus sign it is bleeding to all pages bible, timer setting it is on top, and it should be only in worship".

Two CSS-level leaks:

1. `crates/presenter-ui/src/components/stage_preview.rs:166-172` — the `worship-preview-wrap` div (song name, Ableton/Follow buttons, current/next slide preview) is rendered with `style="display:none"` only when view is `bible`. It stays visible on `timers`, `ai`, `settings` (BUG).
2. `crates/presenter-ui/src/pages/operator.rs:166` — `<section class="operator__worship" data-view-panel="worship">` lacks the `.operator__panel { display: none; ... }` default that the bible / timers / ai / settings sections inherit. The CSS rule at `operator.css:1410` only sets `display: flex` when view IS worship; never hides on other views.

User confirmed (brainstorming Q1) that the diagnostic `[data-role="stage-monitor"]` and the `[data-role="clear-slide"]` (🧹) button stay visible on every view.

**Key existing code:**

- `crates/presenter-ui/src/components/stage_preview.rs:166-228` — the `<div data-role="worship-preview" class="operator__worship-preview-wrap">` block.
- `crates/presenter-ui/styles/operator.css:1402-1422` — current panel-display rules.
- `crates/presenter-ui/src/components/header.rs:187` — view names: `worship`, `bible`, `timers`, `ai`, `settings`.
- `crates/presenter-ui/src/pages/operator.rs:49, 277` — both call `body.set_attribute("data-view", ...)`.
- `tests/e2e/api-stage.spec.ts:1-50` — establishes the project's E2E pattern: `startTestServer` + `deriveTestConfig` + `refreshDevData` from `./support`, console filtering for the chrome `crbug.com/981419` warning, `expect(consoleMessages).toEqual([])` at the end of each test.

---

## File Structure

### Modified Files

| File | Change |
|------|--------|
| `Cargo.toml` | Workspace version 0.4.62 → 0.4.63 |
| `crates/presenter-ui/Cargo.toml` | Version 0.1.31 → 0.1.32 |
| `crates/presenter-ui/src/components/stage_preview.rs:170` | Flip the inline-style condition: show `worship-preview-wrap` only on `worship` view |
| `crates/presenter-ui/styles/operator.css` | Add a new `body.operator:not([data-view="worship"]) [data-view-panel="worship"] { display: none; }` rule after the existing show-on-worship rule |

### Created Files

| File | Responsibility |
|------|----------------|
| `tests/e2e/operator-view-isolation.spec.ts` | Playwright E2E: visibility matrix for all 5 operator views. Asserts worship UI hidden on non-worship views; stage-monitor + clear-slide stay visible. Browser console clean. |

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
version = "0.4.63"
```

- [ ] **Step 2: Bump presenter-ui version**

In `crates/presenter-ui/Cargo.toml`, change line 3:

```toml
version = "0.1.32"
```

- [ ] **Step 3: Update lockfiles**

```bash
cargo update --workspace
cargo update --workspace --manifest-path crates/presenter-ui/Cargo.toml
```

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock crates/presenter-ui/Cargo.toml crates/presenter-ui/Cargo.lock
git commit -m "chore: bump version to 0.4.63"
```

---

## Task 2: Apply the two CSS-level fixes

**Files:**
- Modify: `crates/presenter-ui/src/components/stage_preview.rs:170` (one-line condition flip)
- Modify: `crates/presenter-ui/styles/operator.css` (after line 1412, add 3-line rule)

- [ ] **Step 1: Read the current StagePreview condition**

```bash
sed -n '166,172p' crates/presenter-ui/src/components/stage_preview.rs
```

You should see:

```rust
            <div
                data-role="worship-preview"
                class="operator__worship-preview-wrap"
                style=move || {
                    if ctx.view.get() == "bible" { "display:none" } else { "" }
                }
            >
```

- [ ] **Step 2: Flip the condition**

Replace the closure body. Find:

```rust
                style=move || {
                    if ctx.view.get() == "bible" { "display:none" } else { "" }
                }
```

Replace with:

```rust
                style=move || {
                    if ctx.view.get() == "worship" { "" } else { "display:none" }
                }
```

The result: the div is `display:none` on every view except `worship`.

- [ ] **Step 3: Read the current CSS panel rules**

```bash
sed -n '1410,1422p' crates/presenter-ui/styles/operator.css
```

You should see the existing rule:

```css
body.operator[data-view="worship"] [data-view-panel="worship"] {
  display: flex;
}

body.operator[data-view="bible"] [data-view-panel="bible"],
body.operator[data-view="timers"] [data-view-panel="timers"],
body.operator[data-view="ai"] [data-view-panel="ai"] {
  display: block;
}

body.operator[data-view="settings"] [data-view-panel="settings"] {
  display: block;
}
```

- [ ] **Step 4: Add the new hide-on-non-worship rule**

In `crates/presenter-ui/styles/operator.css`, immediately after the existing show-on-worship rule (between line 1412 and 1414, or wherever the closing `}` of the show-on-worship rule is), add:

```css
body.operator:not([data-view="worship"]) [data-view-panel="worship"] {
  display: none;
}
```

The result: the worship section (libraries / playlists / presentations + their `+` buttons) is hidden on every view except worship. The existing show-on-worship rule continues to display it as flex when the view is worship.

- [ ] **Step 5: Build the WASM crate**

```bash
cd crates/presenter-ui && cargo build --target wasm32-unknown-unknown && cd ../..
```

Expected: build passes.

- [ ] **Step 6: Run workspace clippy + fmt**

```bash
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all
cd crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings -W clippy::all && cd ../..
cargo fmt --all --check
```

Expected: clean. Zero warnings.

- [ ] **Step 7: Run workspace tests**

```bash
cargo test --workspace -- --nocapture
```

Expected: all tests pass. No tests should rely on the buggy old condition.

- [ ] **Step 8: Commit**

```bash
cargo fmt --all
git add crates/presenter-ui/src/components/stage_preview.rs crates/presenter-ui/styles/operator.css
git commit -m "fix(ui): worship-only UI elements isolated from other views (#295)

Two CSS-level fixes:
- Inline-style on the StagePreview worship-preview-wrap now shows
  on view='worship' and hides on every other view (was: hide on
  'bible' only, leaking onto timers/ai/settings).
- New CSS rule hides the worship section ([data-view-panel='worship']
  containing libraries/playlists/presentations + their + buttons)
  when the view is not 'worship'.

The existing show-on-worship rule continues to apply. Diagnostic
elements (stage-monitor counter, clear-slide button) stay visible
on every view, as confirmed in the spec."
```

---

## Task 3: Playwright E2E for the visibility matrix

**Files:**
- Create: `tests/e2e/operator-view-isolation.spec.ts`

- [ ] **Step 1: Inspect existing E2E patterns**

Read `tests/e2e/api-stage.spec.ts` lines 1-30 and 55-80 to see how the project sets up `beforeAll`/`afterAll` with `startTestServer`, navigates with `page.goto(new URL("/...", baseURL).toString())`, waits for `body[data-wasm-ready="true"]`, and how it collects/asserts console messages.

```bash
sed -n '1,30p' tests/e2e/api-stage.spec.ts
sed -n '55,90p' tests/e2e/api-stage.spec.ts
sed -n '210,225p' tests/e2e/api-stage.spec.ts
```

The project pattern uses helpers from `./support`:
- `deriveTestConfig(testInfo)` returns `{ baseURL, dbUrl, port, oscPort }`
- `startTestServer(port, dbUrl, oscPort)` returns a `ServerHandle`
- `stopServer(server)` shuts it down
- Console filter: skip messages containing `crbug.com/981419`

- [ ] **Step 2: Create the new test file**

Create `tests/e2e/operator-view-isolation.spec.ts` with this content:

```typescript
import { test, expect, type Page } from "@playwright/test";
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

function collectConsoleMessages(page: Page): string[] {
  const messages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() !== "error" && msg.type() !== "warning") return;
    // Filter the known chromium "Untrusted Types" warning
    if (msg.text().includes("crbug.com/981419")) return;
    messages.push(`[${msg.type()}] ${msg.text()}`);
  });
  return messages;
}

async function openOperator(page: Page, viewPath: string): Promise<void> {
  await page.goto(new URL(`/ui/operator${viewPath}`, baseURL).toString(), {
    waitUntil: "domcontentloaded",
  });
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
}

test("worship view shows worship UI", async ({ page }) => {
  const consoleMessages = collectConsoleMessages(page);
  await openOperator(page, "");

  await expect(page.locator('[data-role="worship-preview"]')).toBeVisible();
  await expect(page.locator('[data-view-panel="worship"]')).toBeVisible();
  await expect(page.locator('[data-role="stage-monitor"]')).toBeVisible();
  await expect(page.locator('[data-role="clear-slide"]')).toBeVisible();

  expect(consoleMessages).toEqual([]);
});

test("bible view hides worship UI, shows bible panel", async ({ page }) => {
  const consoleMessages = collectConsoleMessages(page);
  await openOperator(page, "/bible");

  await expect(page.locator('[data-role="worship-preview"]')).not.toBeVisible();
  await expect(page.locator('[data-view-panel="worship"]')).not.toBeVisible();
  await expect(page.locator('[data-view-panel="bible"]')).toBeVisible();
  await expect(page.locator('[data-role="stage-monitor"]')).toBeVisible();
  await expect(page.locator('[data-role="clear-slide"]')).toBeVisible();

  expect(consoleMessages).toEqual([]);
});

test("timers view hides worship UI, shows timers panel", async ({ page }) => {
  const consoleMessages = collectConsoleMessages(page);
  await openOperator(page, "/timers");

  await expect(page.locator('[data-role="worship-preview"]')).not.toBeVisible();
  await expect(page.locator('[data-view-panel="worship"]')).not.toBeVisible();
  await expect(page.locator('[data-view-panel="timers"]')).toBeVisible();
  await expect(page.locator('[data-role="stage-monitor"]')).toBeVisible();
  await expect(page.locator('[data-role="clear-slide"]')).toBeVisible();

  expect(consoleMessages).toEqual([]);
});

test("ai view hides worship UI, shows ai panel", async ({ page }) => {
  const consoleMessages = collectConsoleMessages(page);
  await openOperator(page, "/ai");

  await expect(page.locator('[data-role="worship-preview"]')).not.toBeVisible();
  await expect(page.locator('[data-view-panel="worship"]')).not.toBeVisible();
  await expect(page.locator('[data-view-panel="ai"]')).toBeVisible();
  await expect(page.locator('[data-role="stage-monitor"]')).toBeVisible();
  await expect(page.locator('[data-role="clear-slide"]')).toBeVisible();

  expect(consoleMessages).toEqual([]);
});

test("settings view hides worship UI, shows settings panel", async ({
  page,
}) => {
  const consoleMessages = collectConsoleMessages(page);
  await openOperator(page, "/settings");

  await expect(page.locator('[data-role="worship-preview"]')).not.toBeVisible();
  await expect(page.locator('[data-view-panel="worship"]')).not.toBeVisible();
  await expect(page.locator('[data-view-panel="settings"]')).toBeVisible();
  await expect(page.locator('[data-role="stage-monitor"]')).toBeVisible();
  await expect(page.locator('[data-role="clear-slide"]')).toBeVisible();

  expect(consoleMessages).toEqual([]);
});
```

- [ ] **Step 3: Run the new spec locally**

```bash
npx playwright test tests/e2e/operator-view-isolation.spec.ts
```

Expected: all 5 tests pass.

If a test fails because `[data-role="stage-monitor"]` or `[data-role="clear-slide"]` is hidden on a particular view (e.g. bible), that means the spec assumption "they stay on every view" is wrong. Inspect with `npx playwright test --debug`. The likely fix is to remove those assertions on the affected view rather than change the implementation, since the goal is only to hide WORSHIP-specific elements.

If a test fails because `[data-view-panel="bible"]` (or another non-worship panel) is not visible on its own view, that means the existing CSS rule at `operator.css:1414-1418` isn't matching. Verify the body has `data-view="<view>"` set:

```bash
# In the failing test, add a temporary diagnostic:
console.log(await page.evaluate(() => document.body.getAttribute("data-view")));
```

If the body doesn't have the attribute set, that's a separate bug — escalate.

- [ ] **Step 4: Verify console messages stay clean**

The 5 tests collect console errors and warnings (filtered for the known chromium `crbug.com/981419` warning per project convention). If any test fails on the `expect(consoleMessages).toEqual([])` assertion, look at the failure output to see what message leaked. Common causes:

- A new uncaught exception introduced by the CSS / Rust change → fix it.
- An existing console warning that wasn't there before → check whether the WASM build is generating it; if it's pre-existing and not a regression, add it to the filter list.

- [ ] **Step 5: Commit**

```bash
git add tests/e2e/operator-view-isolation.spec.ts
git commit -m "test(e2e): operator view isolation matrix (#295)

Five new Playwright tests asserting the visibility matrix per view:
worship + bible + timers + ai + settings. Worship-specific elements
(worship-preview-wrap, libraries/playlists/presentations panel) hidden
on non-worship views; stage-monitor and clear-slide stay visible
everywhere; the matching panel for each view shows. Browser console
must stay clean (per ci/browser-console-zero-errors)."
```

---

## Task 4: Local checks, push, monitor CI, deploy verify, PR, completion report

**Controller-handled task.** Each step is what the controller does after Tasks 1-3 are committed.

- [ ] **Step 1: Run all local checks**

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all
cd crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings -W clippy::all && cd ../..
cargo test --workspace -- --nocapture
npx playwright test tests/e2e/operator-view-isolation.spec.ts
```

If any fail, fix in ONE commit and re-run.

- [ ] **Step 2: Push to dev**

```bash
git push origin dev
```

- [ ] **Step 3: Monitor CI to terminal state**

```bash
gh run list --branch dev --limit 1 --json databaseId --jq '.[0].databaseId'
# Capture run id, then:
sleep 1500 && gh run view <run-id> --json status,conclusion,jobs --jq '{status, conclusion, failed: [.jobs[] | select(.conclusion == "failure") | .name]}'
```

If any job fails, `gh run view <run-id> --log-failed`, fix in ONE commit, push again, re-monitor.

- [ ] **Step 4: Verify dev deployment is live**

```bash
curl -s http://10.77.8.134:8080/healthz
```

Expected: `{"channel":"dev","status":"ok","version":"0.4.63"}`.

- [ ] **Step 5: Manual verification on dev**

Use Playwright MCP (`browser_navigate` + `browser_evaluate`) to navigate through all 5 operator views on `http://10.77.8.134:8080/ui/operator/<view>`. For each view, assert visibility:

| URL | worship-preview | worship panel | matching panel |
|---|---|---|---|
| `/ui/operator` | visible | visible | (worship is the active view) |
| `/ui/operator/bible` | hidden | hidden | bible panel visible |
| `/ui/operator/timers` | hidden | hidden | timers panel visible |
| `/ui/operator/ai` | hidden | hidden | ai panel visible |
| `/ui/operator/settings` | hidden | hidden | settings panel visible |

For each view, also check that `[data-role="stage-monitor"]` and `[data-role="clear-slide"]` are visible.

Capture a screenshot or DOM dump of one of the non-worship views (e.g. `/ui/operator/bible`) for the PR body, showing the worship UI is no longer present at the top.

- [ ] **Step 6: Open PR**

```bash
gh pr create --title "fix(ui): isolate worship UI from other operator views (#295)" --body "$(cat <<'EOF'
## Summary

Fixes #295: worship-specific UI elements (song name, Ableton/Follow buttons, libraries/playlists/presentations panels with their `+` buttons) no longer appear on bible, timers, ai, or settings views. They render only when the active view is worship. Diagnostic elements (stage-monitor health counter, clear-slide button) stay visible on every view.

## What changed

Two CSS-level fixes, no new components, no logic changes:

1. **stage_preview.rs:170** — flipped the inline-style condition on `[data-role="worship-preview"]` from `if view == "bible" { hide } else { show }` to `if view == "worship" { show } else { hide }`.
2. **operator.css** — added `body.operator:not([data-view="worship"]) [data-view-panel="worship"] { display: none; }` to hide the worship section (which lacks the default `.operator__panel { display: none; }` base).

## Test plan

- [x] All workspace tests pass
- [x] 5 new Playwright E2E tests in `tests/e2e/operator-view-isolation.spec.ts` (worship + bible + timers + ai + settings)
- [x] Each test asserts: worship UI hidden on non-worship views; stage-monitor + clear-slide visible everywhere; the matching panel for each view shows; browser console clean.
- [x] Dev `/healthz` reports v0.4.63
- [x] Manual: navigated all 5 views on dev via Playwright; visibility matrix confirmed.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 7: Verify PR is mergeable**

```bash
gh pr view <pr-number> --json mergeable,mergeStateStatus
```

Expected: `mergeable: MERGEABLE`, `mergeStateStatus: CLEAN`. If `UNSTABLE` due to mutation testing or PR Automation still pending, wait. If anything else, investigate.

- [ ] **Step 8: Run pre-completion gates**

Invoke `/plan-check` skill — must come back N/N fulfilled. Invoke `/review` skill on this PR — must come back `0 🔴 0 🟡 0 🔵`. Fix any findings inside the diff before sending the completion report.

- [ ] **Step 9: Send completion report**

Per `core/completion-report.md`. Include CI run ID, plan-check N/N, review clean, deploy verification (dev shows v0.4.63 with all 5 views verified), URLs, PR title + URL.

---

## Verification Summary

| Check | How to verify |
|-------|---------------|
| Worship-preview-wrap hidden on non-worship | E2E tests for bible/timers/ai/settings assert `:not(:visible)` |
| Worship section hidden on non-worship | Same E2E tests assert `[data-view-panel="worship"]:not(:visible)` |
| Worship-preview-wrap visible on worship | Worship E2E test asserts `:visible` |
| Worship section visible on worship | Same E2E test asserts `:visible` |
| Stage-monitor + clear-slide stay visible everywhere | All 5 E2E tests assert both visible |
| Matching panel visible per view | Each non-worship E2E test asserts the specific panel visible |
| No regressions | Workspace tests still pass; WASM clippy clean |
| Clean console | All 5 E2E tests assert `consoleMessages.length === 0` |
| Live behavior on dev | Manual Playwright tour through all 5 views |
