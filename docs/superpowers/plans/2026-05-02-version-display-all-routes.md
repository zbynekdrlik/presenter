# Version Display on Every UI Route Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show the deployed version label on `/ui/tablet` and `/stage` (in addition to the existing `/ui/operator`) and add a Playwright helper that asserts the label format and frontend/backend match across all routes.

**Architecture:** Extract the existing operator-only `VersionFooter` (in `crates/presenter-ui/src/pages/operator.rs:593-608`) into a shared `VersionLabel` component at `crates/presenter-ui/src/components/version_label.rs`. Add `data-testid="version"` so Playwright can target it. Embed the new component in `pages/tablet.rs` (small bottom-right badge) and `components/stage/status_bar.rs` (small line under the connection/latency display). Add a `assertVersionLabel(page, baseURL)` helper to `tests/e2e/support.ts` and call it once per existing route's E2E test.

**Tech Stack:** Rust + Leptos 0.7 (WASM via trunk), plain CSS (no preprocessor), Playwright (TypeScript).

**Spec:** `docs/superpowers/specs/2026-05-02-version-display-all-routes-design.md` (commit `b6b34d5`)

---

## Context

### Spec deviation discovered during plan-writing

The spec's "per-route placement" table lists `/ui/bible` as a separate route. **It is not.** `/ui/bible` is not in the WASM router (`crates/presenter-ui/src/lib.rs:30-60`). The actual routes are:

- `/ui/operator` (and `/ui/operator/<view>` — including `/ui/operator/bible`) → `OperatorPage`
- `/ui/tablet` → `TabletPage`
- `/stage` → `StagePage`

Because `/ui/operator/bible` renders the bible UI INSIDE the operator layout, it is already covered by the operator's existing `VersionFooter`. **The plan therefore only adds new placements for tablet and stage.** All four operator sub-views (`worship`, `bible`, `presentation`, etc.) inherit the version label automatically through the operator page wrapper.

### Project state (verified pre-flight)

- **Existing VersionFooter component:** `crates/presenter-ui/src/pages/operator.rs:593-608` — fetches `/healthz`, displays `v{version} ({channel})` for dev or `v{version}` for release. Wrapped at line 197 by `<footer class="operator__version">` (CSS in `crates/presenter-ui/styles/operator.css:2152` — `position: fixed; bottom: 0.5rem; right: 0.75rem; font-size: 0.7rem; color: rgba(255, 255, 255, 0.35);`).
- **Components module:** `crates/presenter-ui/src/components/mod.rs` — flat list of `pub mod`s. New file added here.
- **Tablet page entry:** `crates/presenter-ui/src/pages/tablet.rs:141-149` — top-level `view! { <TabletTimerBar /> <TabletHeader /> <main class="tablet-layout">...</main> <TabletToast /> }`. Add the version badge after `<TabletToast />`.
- **Stage status bar:** `crates/presenter-ui/src/components/stage/status_bar.rs:90-111` — view structure has `clock`, `song-number`, `live-pill`, `connection`. Add version `<div>` immediately after `connection` per user request "always small under the latency".
- **CSS files (plain CSS, NOT SCSS):** `crates/presenter-ui/styles/{operator,tablet,bible,stage,settings}.css`. Each page has its own CSS file, served via trunk.
- **Playwright support:** `tests/e2e/support.ts` (TypeScript). Helpers exported as named functions.
- **Backend `/healthz`:** Returns `{"channel":"dev","status":"ok","version":"0.4.51"}` (or `"release"`). Source: `crates/presenter-server/src/router.rs` const `BUILD_CHANNEL`.
- **Versions to bump:** Workspace `Cargo.toml` (currently `0.4.51`) → `0.4.52`. presenter-ui `crates/presenter-ui/Cargo.toml` (currently `0.1.20`) → `0.1.21`.

---

## File Structure

### Created files
| File | Responsibility |
|------|---------------|
| `crates/presenter-ui/src/components/version_label.rs` | Shared `VersionLabel` Leptos component. Fetches `/healthz` once on mount, renders `<span data-testid="version">v{version} ({channel})</span>`. Replaces the private `VersionFooter` in operator.rs. |

### Modified files
| File | Change |
|------|--------|
| `Cargo.toml` | Workspace version 0.4.51 → 0.4.52 |
| `crates/presenter-ui/Cargo.toml` | presenter-ui version 0.1.20 → 0.1.21 |
| `crates/presenter-ui/src/components/mod.rs` | Add `pub mod version_label;` and `pub use version_label::VersionLabel;` |
| `crates/presenter-ui/src/pages/operator.rs` | Replace inline `<VersionFooter />` with `<VersionLabel />`. Delete the local `VersionFooter` component (lines 592-608). |
| `crates/presenter-ui/src/pages/tablet.rs` | Add `<span class="tablet__version-badge"><VersionLabel /></span>` after `<TabletToast />` in `TabletPage` view. |
| `crates/presenter-ui/src/components/stage/status_bar.rs` | Add `<div class="stage__version">...<VersionLabel /></div>` after the connection `<div>`. |
| `crates/presenter-ui/styles/tablet.css` | Append `.tablet__version-badge` rule (fixed bottom-right, small, low-opacity). |
| `crates/presenter-ui/styles/stage.css` | Append `.stage__version` rule (small font, opacity 0.6). |
| `tests/e2e/support.ts` | Add `assertVersionLabel(page, baseURL)` helper. |
| `tests/e2e/operator-*.spec.ts` (one chosen) | Call `assertVersionLabel` after page load. |
| `tests/e2e/tablet*.spec.ts` (one chosen) | Same. |
| `tests/e2e/stage*.spec.ts` (or smoke spec, see Task 5) | Same. |

### Lock files
- `Cargo.lock` (workspace) and `crates/presenter-ui/Cargo.lock` (separate) — auto-updated by `cargo check`/`cargo build`.

---

## Task 1: Bump Version (Haiku)

**Files:**
- Modify: `Cargo.toml`
- Modify: `crates/presenter-ui/Cargo.toml`
- Modify: `Cargo.lock` and `crates/presenter-ui/Cargo.lock` (regenerated)

- [ ] **Step 1: Bump workspace version**

In `/home/newlevel/devel/presenter/presenter-dev2/Cargo.toml`, under `[workspace.package]`, change:

```toml
version = "0.4.51"
```

to:

```toml
version = "0.4.52"
```

- [ ] **Step 2: Bump presenter-ui crate version**

In `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui/Cargo.toml`, under `[package]`, change:

```toml
version = "0.1.20"
```

to:

```toml
version = "0.1.21"
```

- [ ] **Step 3: Regenerate workspace Cargo.lock**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo check --workspace --all-targets 2>&1 | tail -10
```

Expected: clean check, `Cargo.lock` updated.

- [ ] **Step 4: Regenerate presenter-ui Cargo.lock**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui && cargo check --target wasm32-unknown-unknown 2>&1 | tail -10
```

Expected: clean check (or only existing warnings — no new errors), `crates/presenter-ui/Cargo.lock` updated to reflect the new presenter-ui version.

If the WASM target is not installed, run `rustup target add wasm32-unknown-unknown` first.

- [ ] **Step 5: Verify**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && grep -E "^version" Cargo.toml crates/presenter-ui/Cargo.toml | head -3
```

Expected output:

```
Cargo.toml:version = "0.4.52"
crates/presenter-ui/Cargo.toml:version = "0.1.21"
```

- [ ] **Step 6: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && git add Cargo.toml Cargo.lock crates/presenter-ui/Cargo.toml crates/presenter-ui/Cargo.lock && git commit -m "chore: bump version to 0.4.52 (#287)"
```

---

## Task 2: Extract VersionLabel Component (Sonnet)

**Files:**
- Create: `crates/presenter-ui/src/components/version_label.rs`
- Modify: `crates/presenter-ui/src/components/mod.rs`
- Modify: `crates/presenter-ui/src/pages/operator.rs` (replace inline VersionFooter)

- [ ] **Step 1: Create the shared component**

Create `crates/presenter-ui/src/components/version_label.rs` with:

```rust
use leptos::prelude::*;

/// Shared version label component. Fetches `/healthz` once on mount and
/// displays `v{version} ({channel})` for dev builds, or `v{version}` for
/// release builds.
///
/// Tagged with `data-testid="version"` so Playwright can target it
/// consistently across all UI routes.
#[component]
pub fn VersionLabel() -> impl IntoView {
    let version_text = RwSignal::new(String::new());
    leptos::task::spawn_local(async move {
        if let Ok(health) = crate::api::get_json::<crate::api::HealthzResponse>("/healthz").await {
            let text = if health.channel.is_empty() || health.channel == "release" {
                format!("v{}", health.version)
            } else {
                format!("v{} ({})", health.version, health.channel)
            };
            version_text.set(text);
        }
    });
    view! {
        <span data-testid="version">{move || version_text.get()}</span>
    }
}
```

- [ ] **Step 2: Re-export from components/mod.rs**

In `crates/presenter-ui/src/components/mod.rs`, add at the appropriate alphabetical position (after `toast`):

```rust
pub mod version_label;
```

(There is no need for `pub use` re-export — call sites use `crate::components::version_label::VersionLabel`. If the module convention is `pub use`, follow the existing pattern; verify by reading the existing mod.rs first.)

- [ ] **Step 3: Replace the inline VersionFooter in operator.rs**

In `crates/presenter-ui/src/pages/operator.rs`:

a) Find the existing usage at line 197-198:

```rust
        <footer class="operator__version">
            <VersionFooter />
        </footer>
```

Replace with:

```rust
        <footer class="operator__version">
            <crate::components::version_label::VersionLabel />
        </footer>
```

b) Delete the local `VersionFooter` component definition. Find the function (around line 592-608):

```rust
#[component]
fn VersionFooter() -> impl IntoView {
    let version_text = RwSignal::new(String::new());
    leptos::task::spawn_local(async move {
        if let Ok(health) = crate::api::get_json::<crate::api::HealthzResponse>("/healthz").await {
            let text = if health.channel.is_empty() || health.channel == "release" {
                format!("v{}", health.version)
            } else {
                format!("v{} ({})", health.version, health.channel)
            };
            version_text.set(text);
        }
    });
    view! {
        <span>{move || version_text.get()}</span>
    }
}
```

Delete the entire function (including the `#[component]` attribute and any blank lines immediately before/after). Verify with `grep -n "VersionFooter" crates/presenter-ui/src/pages/operator.rs` — expected output: NO matches.

- [ ] **Step 4: Build the WASM crate**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui && cargo build --target wasm32-unknown-unknown 2>&1 | tail -15
```

Expected: clean build. If build fails because `crate::api::HealthzResponse` or `crate::api::get_json` is not visible from the new `components/version_label.rs` location, check the existing imports in operator.rs and replicate them in the new file.

- [ ] **Step 5: Run clippy on the WASM crate**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings -W clippy::all 2>&1 | tail -10
```

Expected: zero warnings.

- [ ] **Step 6: Run cargo fmt**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo fmt --all
```

- [ ] **Step 7: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && git add crates/presenter-ui/src/components/version_label.rs crates/presenter-ui/src/components/mod.rs crates/presenter-ui/src/pages/operator.rs && git commit -m "refactor(ui): extract VersionLabel into shared component (#287)"
```

---

## Task 3: Add VersionLabel to Tablet (Sonnet)

**Files:**
- Modify: `crates/presenter-ui/src/pages/tablet.rs:141-149`
- Modify: `crates/presenter-ui/styles/tablet.css` (append rule)

- [ ] **Step 1: Add VersionLabel to TabletPage view**

In `crates/presenter-ui/src/pages/tablet.rs`, at line 141-149 the current view is:

```rust
    view! {
        <TabletTimerBar />
        <TabletHeader />
        <main class="tablet-layout">
            <TabletSidebar />
            <TabletMain />
        </main>
        <TabletToast />
    }
```

Replace with:

```rust
    view! {
        <TabletTimerBar />
        <TabletHeader />
        <main class="tablet-layout">
            <TabletSidebar />
            <TabletMain />
        </main>
        <TabletToast />
        <span class="tablet__version-badge">
            <crate::components::version_label::VersionLabel />
        </span>
    }
```

- [ ] **Step 2: Append CSS rule to styles/tablet.css**

In `crates/presenter-ui/styles/tablet.css`, append (at end of file):

```css

/* Version badge — bottom-right corner, small, low-opacity, non-interactive */
.tablet__version-badge {
  position: fixed;
  right: 0.5rem;
  bottom: 0.5rem;
  font-size: 0.65rem;
  color: rgba(255, 255, 255, 0.45);
  pointer-events: none;
  z-index: 1;
}
```

- [ ] **Step 3: Verify the WASM crate still builds**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui && cargo build --target wasm32-unknown-unknown 2>&1 | tail -10
```

Expected: clean build.

- [ ] **Step 4: Run clippy**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings -W clippy::all 2>&1 | tail -10
```

Expected: zero warnings.

- [ ] **Step 5: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && git add crates/presenter-ui/src/pages/tablet.rs crates/presenter-ui/styles/tablet.css && git commit -m "feat(ui): add version label to tablet page (#287)"
```

---

## Task 4: Add VersionLabel to Stage StatusBar (Sonnet)

**Files:**
- Modify: `crates/presenter-ui/src/components/stage/status_bar.rs:90-111`
- Modify: `crates/presenter-ui/styles/stage.css` (append rule)

- [ ] **Step 1: Add VersionLabel to StatusBar view**

In `crates/presenter-ui/src/components/stage/status_bar.rs`, the current view (lines 90-111) is:

```rust
    view! {
        <div node_ref=clock_ref class="stage__clock">
            <span class="stage__debug-label">"clock"</span>
            {clock_text}
        </div>
        {move || has_song_number().then(|| view! {
            <div node_ref=song_number_ref class="stage__song-number" data-role="song-number">
                <span class="stage__debug-label">"song-number"</span>
                {song_number}
            </div>
        })}
        {(!hide_live).then(|| view! {
            <div node_ref=live_ref class=live_class>
                <span class="stage__debug-label">"live"</span>
                {live_text}
            </div>
        })}
        <div node_ref=connection_ref class=connection_class>
            <span class="stage__debug-label">"connection"</span>
            {connection_text}
        </div>
    }
```

Replace with (added new `<div class="stage__version">` AFTER the connection block — per user request "always small under the latency"):

```rust
    view! {
        <div node_ref=clock_ref class="stage__clock">
            <span class="stage__debug-label">"clock"</span>
            {clock_text}
        </div>
        {move || has_song_number().then(|| view! {
            <div node_ref=song_number_ref class="stage__song-number" data-role="song-number">
                <span class="stage__debug-label">"song-number"</span>
                {song_number}
            </div>
        })}
        {(!hide_live).then(|| view! {
            <div node_ref=live_ref class=live_class>
                <span class="stage__debug-label">"live"</span>
                {live_text}
            </div>
        })}
        <div node_ref=connection_ref class=connection_class>
            <span class="stage__debug-label">"connection"</span>
            {connection_text}
        </div>
        <div class="stage__version">
            <span class="stage__debug-label">"version"</span>
            <crate::components::version_label::VersionLabel />
        </div>
    }
```

- [ ] **Step 2: Append CSS rule to styles/stage.css**

In `crates/presenter-ui/styles/stage.css`, append (at end of file):

```css

/* Version label inside StatusBar — small, under the connection/latency display */
.stage__version {
  font-size: 0.5em;
  opacity: 0.6;
}
```

- [ ] **Step 3: Verify the WASM crate still builds**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui && cargo build --target wasm32-unknown-unknown 2>&1 | tail -10
```

Expected: clean build.

- [ ] **Step 4: Run clippy**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings -W clippy::all 2>&1 | tail -10
```

Expected: zero warnings.

- [ ] **Step 5: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && git add crates/presenter-ui/src/components/stage/status_bar.rs crates/presenter-ui/styles/stage.css && git commit -m "feat(ui): add version label to stage status bar (#287)"
```

---

## Task 5: Add Playwright Helper + Per-Route Assertions (Sonnet)

**Files:**
- Modify: `tests/e2e/support.ts` (add helper)
- Modify: one test per route — find the simplest existing E2E test for each of operator, tablet, stage, and add a single helper call.

- [ ] **Step 1: Add the assertVersionLabel helper to tests/e2e/support.ts**

In `tests/e2e/support.ts`, add this helper at the end of the file (or in alphabetical/logical position with other exports):

```typescript
import { type Page } from "@playwright/test";

/**
 * Assert that the version label on the current page exists, has the expected
 * format, and matches the backend `/healthz` response.
 *
 * Format: `v<major>.<minor>.<patch>(-dev.<n>)?( (<channel>))?`
 * Examples: `v0.4.52`, `v0.4.52 (dev)`, `v0.4.52-dev.3 (dev)`
 *
 * Frontend version MUST equal `/healthz` `version` field — single source of
 * truth. Channel suffix appears only for non-release builds.
 */
export async function assertVersionLabel(
  page: import("@playwright/test").Page,
  baseURL: string,
): Promise<void> {
  const versionEl = page.locator('[data-testid="version"]').first();
  await expect(versionEl).toBeVisible({ timeout: 10_000 });

  const text = (await versionEl.textContent())?.trim() ?? "";
  expect(text).toMatch(/^v\d+\.\d+\.\d+(-dev\.\d+)?(\s\(\w+\))?$/);

  const healthRes = await fetch(new URL("/healthz", baseURL).toString());
  const health = (await healthRes.json()) as {
    version: string;
    channel: string;
  };
  const expected =
    health.channel === "release" || health.channel === ""
      ? `v${health.version}`
      : `v${health.version} (${health.channel})`;
  expect(text).toBe(expected);
}
```

If `import { type Page }` is already imported elsewhere in the file, do not duplicate it — adapt the helper signature to use the existing import.

If `expect` and `Locator` are already imported (they are, per earlier grep — line 7 of support.ts), reuse them; do not re-import.

The helper uses `globalThis.fetch` (Node 18+ has it built-in; verify by running an existing Playwright test once to confirm). If `fetch` is not available, replace with `await page.request.get('/healthz')` and parse the JSON via the Playwright APIRequestContext pattern.

- [ ] **Step 2: Find existing E2E tests per route**

Run from `/home/newlevel/devel/presenter/presenter-dev2`:

```bash
ls tests/e2e/ | head -30
```

Identify the simplest, fastest test file for each of:
- **operator** — likely `tests/e2e/operator-*.spec.ts` or `tests/e2e/settings.spec.ts` (settings is reached via operator). Pick one that loads `/ui/operator` and waits for ready state.
- **tablet** — likely `tests/e2e/tablet.spec.ts` or `tests/e2e/tablet-pwa.spec.ts`. Pick the one that just loads the tablet page.
- **stage** — likely `tests/e2e/stage*.spec.ts` or a stage-related spec. Find a test that opens `/stage`.

If a route has multiple tests, pick the one that reaches a stable post-load state quickest (e.g., the test that just verifies the page loads and the WS connects).

- [ ] **Step 3: Add assertVersionLabel call to one operator test**

In the chosen operator E2E test, immediately after the page reaches its stable load state (typically after `await page.waitForLoadState('networkidle')` or after a specific selector becomes visible), add:

```typescript
import { assertVersionLabel } from "./support";
// ... inside the test ...
await assertVersionLabel(page, baseURL);
```

If `assertVersionLabel` is exported from a different relative path, adjust the import. The test file's existing `import` block likely already pulls from `./support`; just add `assertVersionLabel` to that import list.

`baseURL` is conventionally available from the test fixture (`{ baseURL }` in the test signature). If it isn't, capture it from `process.env.PRESENTER_BASE_URL` or the existing pattern used in that test file.

- [ ] **Step 4: Add assertVersionLabel call to one tablet test**

Same pattern as Step 3, in the chosen tablet test. After the tablet page reaches its stable load state (e.g., after `await expect(page.locator('[data-role="tablet-ready"]')).toBeVisible()` — adapt to whatever readiness signal that test uses), add:

```typescript
await assertVersionLabel(page, baseURL);
```

- [ ] **Step 5: Add assertVersionLabel call to one stage test**

Same pattern as Step 3, in the chosen stage test. The stage page exposes `__presenterStageConnectionState` global; test should wait until that becomes `"connected"` before asserting the version label (the WS state going to connected confirms the page is fully loaded).

```typescript
await assertVersionLabel(page, baseURL);
```

- [ ] **Step 6: Run the modified tests locally**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && npm run test:playwright -- --grep "<your test name>" 2>&1 | tail -30
```

Substitute `<your test name>` with the test names you modified. If the project uses tags or `--project` filters, follow the existing pattern (check `package.json` `scripts` and `playwright.config.*`).

Expected: tests pass with the new assertion. If a test fails because the version label is missing or has the wrong format, fix the implementation in earlier tasks.

If running individual tests is awkward in this project, run the broader smoke suite:

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && npm run test:playwright -- --grep "@smoke" 2>&1 | tail -30
```

Or fall back to running the entire e2e suite — slower but exhaustive.

- [ ] **Step 7: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && git add tests/e2e/ && git commit -m "test(e2e): add per-route version label assertion (#287)"
```

If you only modified specific files, add them by name rather than `tests/e2e/`. Avoid `git add -A`.

---

## Task 6: Local Checks, Push, CI Monitor, Dev Verification, Open PR (Controller)

This task is handled by the controller (the agent driving the plan), not an implementer subagent. Local Rust + WASM builds are allowed on this machine.

### Local pre-push checks

- [ ] **Step 1: Workspace fmt**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo fmt --all --check
```

Expected: zero output.

- [ ] **Step 2: Workspace clippy**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo clippy --workspace --all-targets -- -D warnings -W clippy::all 2>&1 | tail -10
```

Expected: zero warnings.

- [ ] **Step 3: presenter-ui WASM clippy**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings -W clippy::all 2>&1 | tail -10
```

Expected: zero warnings.

- [ ] **Step 4: presenter-server tests**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo test -p presenter-server 2>&1 | tail -10
```

Expected: all tests pass (this PR doesn't touch server code; this confirms no regression).

- [ ] **Step 5: Push**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && git push origin dev
```

- [ ] **Step 6: Monitor CI to terminal state**

Per `core/ci-monitoring.md`: ONE background `sleep + gh run view` per cycle. Never `gh run watch`. Never custom monitor scripts.

```bash
RUN_ID=$(gh run list --branch dev --limit 1 --json databaseId --jq '.[0].databaseId')
sleep 900 && gh run view $RUN_ID --json status,conclusion,jobs --jq '{status, conclusion, jobs: [.jobs[] | {name, status, conclusion}]}'
```

Wait for ALL jobs to reach `"status": "completed"`. If any fails, `gh run view <run-id> --log-failed`, fix root cause in ONE commit, push, monitor again.

### Dev verification (after CI deploys)

- [ ] **Step 7: Verify all four routes show v0.4.52**

```bash
echo "=== /healthz ===" && curl -s http://10.77.8.134:8080/healthz
echo
```

Expected: `{"channel":"dev","status":"ok","version":"0.4.52"}`.

Open in Playwright (or a browser if running interactively) each of:

1. `http://10.77.8.134:8080/ui/operator` — version visible bottom-right
2. `http://10.77.8.134:8080/ui/operator/bible` — version visible bottom-right (inherits from operator)
3. `http://10.77.8.134:8080/ui/tablet` — version visible bottom-right (small badge)
4. `http://10.77.8.134:8080/stage` — version visible in status bar, under connection/latency

For automated verification, use the Playwright MCP:

```
mcp__plugin_playwright_playwright__browser_navigate(url: "http://10.77.8.134:8080/ui/tablet")
mcp__plugin_playwright_playwright__browser_evaluate(function: "() => document.querySelector('[data-testid=\"version\"]')?.textContent")
```

The returned text MUST start with `v0.4.52`. Repeat for all four routes.

### Open PR

- [ ] **Step 8: Open PR**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && gh pr create --base main --head dev --title "feat(ui): version display on every route (#287)" --body "$(cat <<'EOF'
## Summary

Extends the deployed version label to every UI route so post-deploy verification works regardless of which route the user is on. Closes #287.

Per `~/devel/airuleset/modules/quality/version-on-dashboard.md`, every web dashboard MUST show the deployed version label visibly on every route. Today only `/ui/operator` does — this PR fixes that for `/ui/tablet` and `/stage`. (`/ui/operator/bible` already inherits the operator footer.)

## What changed

- New shared component `crates/presenter-ui/src/components/version_label.rs` (`VersionLabel`).
- Refactored operator's inline `VersionFooter` to use `VersionLabel`. Pixel-identical output.
- Added `<VersionLabel />` to `pages/tablet.rs` (bottom-right corner badge).
- Added `<VersionLabel />` to `components/stage/status_bar.rs` (small line under connection/latency).
- New Playwright helper `assertVersionLabel(page, baseURL)` in `tests/e2e/support.ts`.
- Per-route assertions added to one operator, one tablet, one stage E2E test.
- Bumped version 0.4.51 → 0.4.52.

## Test plan

- [x] `cargo clippy --workspace --all-targets -- -D warnings -W clippy::all` — zero warnings
- [x] `cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings -W clippy::all` — zero warnings (presenter-ui)
- [x] `cargo fmt --all --check` — clean
- [x] `cargo test -p presenter-server` — all green (no server changes)
- [x] CI green on dev
- [x] Manual dev verification on all four routes — version label visible, correct format, matches `/healthz`

Closes #287
EOF
)"
```

- [ ] **Step 9: Confirm PR is mergeable + clean**

```bash
PR_NUM=$(gh pr list --head dev --base main --json number --jq '.[0].number')
gh api repos/zbynekdrlik/presenter/pulls/$PR_NUM --jq '{mergeable: .mergeable, mergeable_state: .mergeable_state}'
```

Expected: `{"mergeable": true, "mergeable_state": "clean"}`.

If `mergeable_state` is `"unstable"`, wait — Mutation Testing or another long-running job may still be in progress. If `"behind"`, sync dev with main and push. If `"dirty"` or `"blocked"`, investigate the root cause; never bypass branch protection.

### Pre-completion gate

- [ ] **Step 10: Run `/plan-check`**

Audit every requirement in this plan and the spec. Every item must be `[x]`.

- [ ] **Step 11: Run `/review`** on the PR diff

Address every 🔴, 🟡, AND 🔵 finding inside the diff. Re-run until both audits return `0 🔴 0 🟡 0 🔵`.

- [ ] **Step 12: Send completion report**

Per `core/completion-report.md`. Include:

- `✅ CI: green` (with run id)
- `✅ /plan-check: N/N fulfilled`
- `✅ /review: clean — 0 🔴 0 🟡 0 🔵`
- `✅ Deploy: dev shows v0.4.52 on all four routes (operator, operator/bible, tablet, stage)`
- `🌐 Dev:  http://10.77.8.134:8080/ui/operator`
- `🌐 Prod: http://10.77.9.205/ui/operator`
- `[presenter] PR #N: <full title>` + URL
- Wait for explicit "merge it" before merging.

---

## Verification Summary

| Check | How to verify |
|-------|---------------|
| `VersionLabel` exists and is shared | `grep -n "pub fn VersionLabel" crates/presenter-ui/src/components/version_label.rs` |
| Operator footer still works | Open `/ui/operator` on dev — version visible bottom-right (no visual change) |
| Operator/bible inherits | Open `/ui/operator/bible` — same footer visible |
| Tablet shows version | Open `/ui/tablet` — small bottom-right badge with version |
| Stage shows version under latency | Open `/stage` — version label visible in status bar, immediately after `CONNECTED · NN ms` |
| Format regex valid | Playwright helper asserts `/^v\d+\.\d+\.\d+(-dev\.\d+)?(\s\(\w+\))?$/` |
| Frontend = backend | Playwright helper asserts equal to `/healthz` value |
| CI green | All Pipeline + Deploy + Mutation Testing jobs `success` |
| No regressions | `cargo test -p presenter-server` 184 tests still pass |
