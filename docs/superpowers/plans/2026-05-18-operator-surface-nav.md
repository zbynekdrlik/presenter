# Operator Surface-Nav Strip Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a 4-pill jump-link strip below the operator header that opens Stage, Camera, Tablet, and Timer surfaces in new browser tabs.

**Architecture:** New pure-static Leptos component `<SurfaceNav />` rendering 4 `<a target="_blank" rel="noopener">` links. Inserted once into the operator shell in `pages/operator.rs` between `<Header />` and `<SearchResults />`. CSS appended to `operator.css`. New Playwright spec asserts presence on operator routes, absence on tablet/camera, correct hrefs/target/rel attributes, and zero console errors.

**Tech Stack:** Rust 1.x (workspace), Leptos 0.7 (CSR for `presenter-ui` WASM crate), CSS, Playwright (TypeScript).

**Spec:** `docs/superpowers/specs/2026-05-18-operator-surface-nav-design.md` (commit `c6d4709`).

**Closes:** #326.

---

## File Structure

**Files to create:**

- `crates/presenter-ui/src/components/surface_nav.rs` — new pure-static component (~40 LoC).
- `tests/e2e/operator-surface-nav.spec.ts` — Playwright E2E covering 3 test cases.

**Files to modify:**

- `Cargo.toml` (workspace `[workspace.package].version`) — version bump.
- `Cargo.lock` — lockfile refresh.
- `crates/presenter-ui/Cargo.lock` — WASM lockfile refresh.
- `crates/presenter-ui/src/components/mod.rs` — register the new module.
- `crates/presenter-ui/src/pages/operator.rs` — insert `<SurfaceNav />` in the view! block.
- `crates/presenter-ui/styles/operator.css` — append `.operator__surface-nav` rules.

`crates/presenter-ui/Cargo.toml` is NOT touched — its version is independent of the workspace version (last workspace bump did not touch it; verified in git log).

---

## Task 1: Workspace version bump

**Files:**

- Modify: `Cargo.toml:15`
- Modify: `Cargo.lock` (regenerated)
- Modify: `crates/presenter-ui/Cargo.lock` (regenerated)

**Per `version-bumping.md` — bump BEFORE any feature code. Dev is at 0.4.85 (matches main after PR #327 merge), so the next commit MUST bump it.**

- [ ] **Step 1: Bump workspace version**

Edit `Cargo.toml:15` — change `version = "0.4.85"` to `version = "0.4.86"`.

- [ ] **Step 2: Refresh workspace lockfile**

Run: `cargo update --workspace`
Expected: `Cargo.lock` rewrites; no errors.

- [ ] **Step 3: Refresh WASM lockfile**

Run: `cd crates/presenter-ui && cargo update && cd ../..`
Expected: `crates/presenter-ui/Cargo.lock` rewrites; no errors.

- [ ] **Step 4: Verify version pickup**

Run: `cargo metadata --format-version=1 --no-deps | python3 -c "import json,sys; d=json.load(sys.stdin); print(set(p['version'] for p in d['packages']))"`
Expected: output includes `'0.4.86'` (and `'0.1.39'` for presenter-ui, which is independent).

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock crates/presenter-ui/Cargo.lock
git commit -m "chore: bump workspace version to 0.4.86 for #326

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

## Task 2: RED — Playwright E2E asserting the surface-nav

**Files:**

- Create: `tests/e2e/operator-surface-nav.spec.ts`

Per `tdd-workflow.md`: write the failing test FIRST so we have proof it catches the bug class (no surface-nav present) before we add the component.

- [ ] **Step 1: Create the test file**

Create `tests/e2e/operator-surface-nav.spec.ts` with the following content:

```typescript
/**
 * Operator Surface-Nav Strip E2E (#326).
 *
 * Asserts the 4-pill jump-link row appears on operator chrome
 * (including the bible internal view), is absent on tablet and camera,
 * and links open in a new tab (target=_blank rel=noopener).
 *
 * Also asserts zero browser console errors/warnings per
 * ci/browser-console-zero-errors.md.
 */

import { test, expect, type Page } from "@playwright/test";
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
  serverHandle = await startTestServer(config.port, config.dbUrl);
});

test.afterAll(async () => {
  await stopServer(serverHandle);
});

function collectConsole(page: Page): string[] {
  const messages: string[] = [];
  page.on("console", (msg) => {
    const type = msg.type();
    if (type === "error" || type === "warning") {
      messages.push(`[${type}] ${msg.text()}`);
    }
  });
  page.on("pageerror", (err) => {
    messages.push(`[pageerror] ${err.message}`);
  });
  return messages;
}

const EXPECTED_TARGETS: ReadonlyArray<{ name: string; href: string }> = [
  { name: "Stage", href: "/stage" },
  { name: "Camera", href: "/ui/camera" },
  { name: "Tablet", href: "/ui/tablet" },
  { name: "Timer", href: "/overlays/timer" },
];

async function waitForOperatorReady(page: Page): Promise<void> {
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
}

test("surface-nav strip is visible on /ui/operator with 4 correct anchors", async ({ page }) => {
  const consoleMessages = collectConsole(page);

  await page.goto(`${baseURL}/ui/operator`);
  await waitForOperatorReady(page);

  const nav = page.locator('[data-role="surface-nav"]');
  await expect(nav).toBeVisible();

  for (const target of EXPECTED_TARGETS) {
    const link = nav.locator(`[data-role="surface-nav-link"][data-target="${target.name}"]`);
    await expect(link, `link for ${target.name} should exist`).toHaveCount(1);
    await expect(link).toHaveAttribute("href", target.href);
    await expect(link).toHaveAttribute("target", "_blank");
    const rel = await link.getAttribute("rel");
    expect(rel ?? "", `rel for ${target.name} should contain noopener`).toContain("noopener");
  }

  expect(consoleMessages, "browser console must be clean").toEqual([]);
});

test("surface-nav strip is visible on the bible internal view", async ({ page }) => {
  const consoleMessages = collectConsole(page);

  await page.goto(`${baseURL}/ui/operator/bible`);
  await waitForOperatorReady(page);

  const nav = page.locator('[data-role="surface-nav"]');
  await expect(nav).toBeVisible();

  for (const target of EXPECTED_TARGETS) {
    await expect(
      nav.locator(`[data-role="surface-nav-link"][data-target="${target.name}"]`),
    ).toHaveCount(1);
  }

  expect(consoleMessages, "browser console must be clean").toEqual([]);
});

test("surface-nav strip is absent on /ui/tablet and /ui/camera", async ({ page }) => {
  const consoleMessages = collectConsole(page);

  await page.goto(`${baseURL}/ui/tablet`);
  await waitForOperatorReady(page);
  await expect(page.locator('[data-role="surface-nav"]')).toHaveCount(0);

  await page.goto(`${baseURL}/ui/camera`);
  await waitForOperatorReady(page);
  await expect(page.locator('[data-role="surface-nav"]')).toHaveCount(0);

  expect(consoleMessages, "browser console must be clean").toEqual([]);
});
```

- [ ] **Step 2: Run test to verify it fails (RED)**

Run: `npm run test:playwright -- operator-surface-nav`
Expected: All 3 tests FAIL. The first two should fail with `[data-role="surface-nav"]` not visible / not found. The third should pass (the strip is genuinely absent everywhere right now).

Record the failure output; the test commit SHA becomes the RED reference for the PR description.

- [ ] **Step 3: Commit the failing test**

```bash
git add tests/e2e/operator-surface-nav.spec.ts
git commit -m "test(operator): add E2E for surface-nav strip [red] (#326)

Asserts the 4-pill jump-link row appears on /ui/operator and
/ui/operator/bible with correct hrefs, target=_blank, rel=noopener,
and is absent on /ui/tablet and /ui/camera. Console must be clean.

Test fails before the component lands. Locks the regression once it
passes after the next commit.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

## Task 3: Create the SurfaceNav component

**Files:**

- Create: `crates/presenter-ui/src/components/surface_nav.rs`
- Modify: `crates/presenter-ui/src/components/mod.rs:2`

- [ ] **Step 1: Create the component file**

Create `crates/presenter-ui/src/components/surface_nav.rs` with the following content:

```rust
use leptos::prelude::*;

/// Surface-nav strip: 4 pill links that open external surfaces in a new
/// browser tab. Lives on the operator chrome only (rendered in
/// `pages/operator.rs`). See spec
/// `docs/superpowers/specs/2026-05-18-operator-surface-nav-design.md`.
#[component]
pub fn SurfaceNav() -> impl IntoView {
    let targets = [
        ("Stage", "/stage"),
        ("Camera", "/ui/camera"),
        ("Tablet", "/ui/tablet"),
        ("Timer", "/overlays/timer"),
    ];

    view! {
        <nav
            class="operator__surface-nav"
            data-role="surface-nav"
            aria-label="Open other surfaces in a new tab"
        >
            <span class="operator__surface-nav-label">"Open in new tab:"</span>
            {targets.into_iter().map(|(label, href)| view! {
                <a
                    class="operator__surface-nav-link"
                    data-role="surface-nav-link"
                    data-target=label
                    href=href
                    target="_blank"
                    rel="noopener"
                >
                    {label}
                    <span class="operator__surface-nav-icon" aria-hidden="true">"\u{2197}"</span>
                </a>
            }).collect_view()}
        </nav>
    }
}
```

- [ ] **Step 2: Register the module**

Modify `crates/presenter-ui/src/components/mod.rs`. The current line 2 reads:

```rust
pub mod info_popover;
```

Add a new line immediately after it:

```rust
pub mod info_popover;
pub mod surface_nav;
```

Keep the rest of the file unchanged. Alphabetical-ish order is fine — `info_popover` before `surface_nav` is OK.

- [ ] **Step 3: Verify WASM clippy passes**

Run: `cd crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings -W clippy::all && cd ../..`
Expected: success with zero warnings.

- [ ] **Step 4: Commit**

```bash
git add crates/presenter-ui/src/components/surface_nav.rs crates/presenter-ui/src/components/mod.rs
git commit -m "feat(operator): add SurfaceNav component (#326)

Pure-static Leptos component rendering 4 pill anchors to /stage,
/ui/camera, /ui/tablet, /overlays/timer. target=_blank rel=noopener
on every link. No state, no signals, no API calls.

Not yet mounted — next commit wires it into the operator shell.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

## Task 4: Wire SurfaceNav into the operator shell

**Files:**

- Modify: `crates/presenter-ui/src/pages/operator.rs:6` (import)
- Modify: `crates/presenter-ui/src/pages/operator.rs:162-164` (view! insertion)

- [ ] **Step 1: Add the import**

Find line 6 in `crates/presenter-ui/src/pages/operator.rs`:

```rust
use crate::components::header::Header;
```

Add a new line immediately after it:

```rust
use crate::components::header::Header;
use crate::components::surface_nav::SurfaceNav;
```

- [ ] **Step 2: Insert SurfaceNav in the view!**

Find the view! block starting at line 162. The current opening looks like:

```rust
    view! {
        <Header />
        <SearchResults />
```

Change it to:

```rust
    view! {
        <Header />
        <SurfaceNav />
        <SearchResults />
```

Keep everything else in the view! block unchanged.

- [ ] **Step 3: Verify WASM clippy passes**

Run: `cd crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings -W clippy::all && cd ../..`
Expected: success with zero warnings.

- [ ] **Step 4: Commit**

```bash
git add crates/presenter-ui/src/pages/operator.rs
git commit -m "feat(operator): mount SurfaceNav in operator shell (#326)

Strip is shell-level (one mount, not per-view-panel), so it appears
on every /ui/operator/* route including /ui/operator/bible.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

## Task 5: Surface-nav CSS

**Files:**

- Modify: `crates/presenter-ui/styles/operator.css` (append at end)

- [ ] **Step 1: Append the CSS rules**

Open `crates/presenter-ui/styles/operator.css`. At the end of the file (after the last existing rule, after any closing `}`), append a blank line, then the following block:

```css
/* --- Surface-nav strip (#326) ------------------------------------------- */

.operator__surface-nav {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.4rem 1rem;
    background: var(--bg-secondary, #1a1a1a);
    border-bottom: 1px solid var(--border-subtle, #2a2a2a);
    font-size: 0.85rem;
}

.operator__surface-nav-label {
    color: var(--text-muted, #888);
    margin-right: 0.25rem;
}

.operator__surface-nav-link {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    padding: 0.25rem 0.6rem;
    border: 1px solid var(--border-subtle, #2a2a2a);
    border-radius: 999px;
    color: var(--text-primary, #e0e0e0);
    text-decoration: none;
    transition: background 0.15s ease, border-color 0.15s ease;
}

.operator__surface-nav-link:hover,
.operator__surface-nav-link:focus-visible {
    background: var(--bg-hover, #2a2a2a);
    border-color: var(--border-strong, #444);
    outline: none;
}

.operator__surface-nav-icon {
    opacity: 0.7;
    font-size: 0.9em;
}

@media (max-width: 800px) {
    .operator__surface-nav {
        flex-wrap: wrap;
        padding: 0.3rem 0.5rem;
        font-size: 0.8rem;
    }
}
```

- [ ] **Step 2: Commit**

```bash
git add crates/presenter-ui/styles/operator.css
git commit -m "feat(operator): style surface-nav strip (#326)

Pill row below operator header. Mobile breakpoint at 800px wraps pills.
Mirrors existing operator__view-nav visual language via CSS variables.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

## Task 6: GREEN — final local gate

This task runs the full local test/lint matrix and confirms the RED Playwright test from Task 2 is now GREEN.

- [ ] **Step 1: Workspace formatting**

Run: `cargo fmt --all --check`
Expected: no output, exit 0. If it fails, run `cargo fmt --all` and amend… wait, no — never amend. Instead: run `cargo fmt --all`, then `git add -u` the formatting fixups and add them to a new commit `style: cargo fmt`.

- [ ] **Step 2: Workspace clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings -W clippy::all`
Expected: success with zero warnings.

- [ ] **Step 3: WASM clippy**

Run: `cd crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings -W clippy::all && cd ../..`
Expected: success with zero warnings.

- [ ] **Step 4: Workspace tests**

Run: `cargo test --workspace`
Expected: all tests pass. The new feature touches no Rust unit-test territory; this is a regression check that the WASM/component additions didn't break anything else.

- [ ] **Step 5: Playwright E2E — GREEN**

Run: `npm run test:playwright -- operator-surface-nav`
Expected: all 3 tests PASS. Record the commit SHA here as the GREEN reference for the PR body.

If any test fails:

- Inspect the failure with `npx playwright show-report`.
- Most likely failure mode: trunk/WASM bundle stale. Run `cd crates/presenter-ui && trunk build --release && cd ../..` (the dev server rebuilds on its own, but `startTestServer` may use a cached bundle). Then rerun.
- DO NOT change the test assertions to make it pass. The assertions reflect the spec.

- [ ] **Step 6: If formatting fixups were needed, commit them**

If Step 1 surfaced formatting changes:

```bash
git add -u
git commit -m "style: cargo fmt fixups (#326)

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

Skip this step if Step 1 was already clean.

---

## Task 7: Controller-handled — push, CI, deploy-verify, PR

This task is NOT executed by the implementer subagent. The controller (parent agent) runs it after Tasks 1-6 are committed locally.

- [ ] **Step 1: Final pre-push review**

Run: `git log --oneline origin/dev..HEAD` — confirm the commit order:
1. `chore: bump workspace version to 0.4.86 for #326`
2. `test(operator): add E2E for surface-nav strip [red] (#326)`
3. `feat(operator): add SurfaceNav component (#326)`
4. `feat(operator): mount SurfaceNav in operator shell (#326)`
5. `feat(operator): style surface-nav strip (#326)`
6. (optional) `style: cargo fmt fixups (#326)`

- [ ] **Step 2: Push**

Run: `git push origin dev`
Expected: clean push, no force, no hooks-skipped.

- [ ] **Step 3: Monitor CI**

Run in background: `sleep 300 && gh run view <run-id> --json status,conclusion,jobs`
Per `ci-monitoring.md`: single sleep + `gh run view` call, NOT `gh run watch`, NOT `/loop`, NOT a custom bash script. Wait for ALL jobs (Branch Sync, Format, Clippy, Test, Quality, Coverage, Mutation Testing, Build, Playwright E2E 1-3/3, Deploy to Dev) to reach terminal state.

If any job fails: `gh run view <run-id> --log-failed`, investigate root cause, fix locally, push fix as ONE commit. Do NOT blindly rerun.

- [ ] **Step 4: Verify on dev**

Once Deploy to Dev is green, verify deployment per `post-deploy-verification.md` and `autonomous-verification.md`:

1. `curl -s http://10.77.8.134:8080/healthz` — confirm `version: 0.4.86`.
2. Open `http://10.77.8.134:8080/ui/operator` in Playwright (NOT localhost):
   - Read `[data-testid="version"]` — confirm `v0.4.86 (dev)`.
   - Read `[data-role="surface-nav"]` — confirm visible, 4 anchors with correct hrefs/target/rel.
   - Check browser console — must be clean.
3. Navigate to `http://10.77.8.134:8080/ui/operator/bible` — confirm strip still visible.
4. Navigate to `http://10.77.8.134:8080/ui/tablet` — confirm strip absent.
5. Navigate to `http://10.77.8.134:8080/ui/camera` — confirm strip absent.

Record the version from the live DOM for the completion-report `✅ Deploy:` line.

- [ ] **Step 5: Open the PR**

```bash
gh pr create --base main --head dev \
  --title "feat(operator): surface-nav strip — jump to Stage/Camera/Tablet/Timer (#326)" \
  --body "$(cat <<'EOF'
## Summary

Adds a 4-pill jump-link strip below the operator header. Each pill opens
the corresponding surface in a new browser tab.

Closes #326.

## Surfaces and targets

- Source: `/ui/operator` and all internal views (`/ui/operator/bible`, timers, AI, settings)
- Targets (new tab): Stage `/stage`, Camera `/ui/camera`, Tablet `/ui/tablet`, Timer `/overlays/timer`
- NOT on: `/ui/tablet`, `/ui/camera`, `/stage` (clean surfaces)
- NOT a target: Worship (current view), Bible (internal operator tab)

## Tests

- New Playwright spec `tests/e2e/operator-surface-nav.spec.ts` with 3 cases:
  - Strip visible on `/ui/operator` with correct hrefs/target/rel
  - Strip visible on `/ui/operator/bible`
  - Strip absent on `/ui/tablet` and `/ui/camera`
- Every case also asserts zero console errors/warnings
- RED before component, GREEN after — verified locally

## Test plan

- [x] Workspace fmt, clippy, test
- [x] WASM clippy
- [x] Playwright E2E green locally
- [x] CI green
- [x] Dev deploy verified at http://10.77.8.134:8080/ui/operator (v0.4.86)

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 6: Verify PR is mergeable + clean**

Run: `gh pr view <number> --json mergeable,mergeStateStatus`
Expected: `mergeable: true`, `mergeStateStatus: CLEAN`. If `UNSTABLE`/`BLOCKED`/`BEHIND`/`DIRTY` — investigate and fix per `autonomous-quality-discipline.md`. Do NOT report ready until clean.

- [ ] **Step 7: Send completion report**

Per `completion-report.md` template. MUST include the audit block (all 3 audit lines green) and both 🌐 Dev + 🌐 Prod URL lines once main deploys post-merge. Then WAIT for explicit "merge it" — never merge a PR without user instruction per `pr-merge-policy.md`.

---

## Spec coverage check

| Spec section | Plan task |
|--------------|-----------|
| Mental model — operator-only source | Tasks 3, 4 (mount in operator shell only) |
| 4 targets (Stage/Camera/Tablet/Timer) | Task 3 (`targets` array) |
| Strip on all `/ui/operator/*` views | Task 4 (shell-level mount), Task 2 (test for `/ui/operator/bible`) |
| Strip absent on tablet/camera | Task 2 (test #3) |
| `target=_blank` + `rel=noopener` | Task 3 (component), Task 2 (assertions) |
| Pure-static (no state/API) | Task 3 (no signals, no `use_ctx!`) |
| New row below header, label "Open in new tab:" | Task 3 (component) |
| Mobile wrap at 800px | Task 5 (CSS media query) |
| Console-clean assertion in E2E | Task 2 (`collectConsole` + `expect(...).toEqual([])`) |
| Workspace version bump first | Task 1 (BEFORE Task 2's failing test) |
