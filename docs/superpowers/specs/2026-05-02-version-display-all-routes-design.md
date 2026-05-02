# Version Display on Every UI Route — Design

**Date:** 2026-05-02
**Status:** Proposed
**Scope:** Frontend (presenter-ui WASM) + E2E tests
**Issue:** [#287](https://github.com/zbynekdrlik/presenter/issues/287) — Extend version display to all UI routes + add Playwright assertion (foundation)

## Goal

Display the deployed version on every user-facing UI route (`/ui/operator`, `/ui/tablet`, `/ui/bible`, `/stage`) so post-deploy verification can confirm the new build is live regardless of which route the user is on. Add a Playwright assertion so format and frontend/backend match are continuously validated.

## Why

Per `~/devel/airuleset/modules/quality/version-on-dashboard.md`, every web dashboard MUST show the deployed version label visibly on every route. Today only `/ui/operator` does. Tablet, Bible, and Stage UIs are blind spots — a tablet operator reporting an issue can't tell which version they're on, frontend/backend drift can ship invisibly, and the completion-report `Deploy:` line can only verify the operator route.

## Approach

Reusable Leptos component approach. Move the existing operator-only `VersionFooter` (in `crates/presenter-ui/src/pages/operator.rs:593`) to a shared component at `crates/presenter-ui/src/components/version_label.rs` named `VersionLabel`. Embed it in tablet, bible, and the stage status bar. Add a `data-testid="version"` attribute so Playwright can target it. Add a per-route Playwright helper that asserts format and frontend/backend match.

## Components

### 1. Shared `VersionLabel` component

New file: `crates/presenter-ui/src/components/version_label.rs`

Contract:

```rust
#[component]
pub fn VersionLabel(
    /// Optional CSS class for per-page styling
    #[prop(optional, into)] class: Option<String>,
) -> impl IntoView { ... }
```

Renders `<span data-testid="version" class={class}>v{version} ({channel})</span>` if channel is `dev`, or `<span data-testid="version" class={class}>v{version}</span>` if channel is `release` or empty. Fetches `/healthz` once on mount via `crate::api::get_json::<HealthzResponse>("/healthz")`. Same logic as today's `VersionFooter` — no behavioral change.

Re-export from `crates/presenter-ui/src/components/mod.rs`.

### 2. Operator integration

Replace the existing `VersionFooter` private component in `pages/operator.rs` with a call to `VersionLabel`. Existing visual layout preserved:

```rust
<footer class="operator__version">
    <VersionLabel class="operator__version-text" />
</footer>
```

The `VersionFooter` private component is deleted (its logic moved into `VersionLabel`).

### 3. Tablet integration

Add to `crates/presenter-ui/src/pages/tablet.rs` — small bottom-right corner badge, low-opacity, must not steal touch targets:

```rust
<span class="tablet__version-badge"><VersionLabel /></span>
```

CSS in `crates/presenter-ui/style/tablet.scss` (or wherever tablet styles live):

```scss
.tablet__version-badge {
    position: fixed;
    right: 0.5rem;
    bottom: 0.5rem;
    font-size: 0.65rem;
    opacity: 0.4;
    pointer-events: none;
    z-index: 1;
}
```

### 4. Bible integration

Same pattern as tablet — `crates/presenter-ui/src/pages/bible.rs`:

```rust
<span class="bible__version-badge"><VersionLabel /></span>
```

CSS class follows the same minimalist pattern (small, low-opacity, fixed bottom-right, non-interactive).

### 5. Stage integration

Embed inside the existing `StatusBar` component at `crates/presenter-ui/src/components/stage/status_bar.rs`. Add a NEW `<div>` immediately AFTER the connection block (which shows `CONNECTED · 23 ms`):

```rust
<div node_ref=connection_ref class=connection_class>
    <span class="stage__debug-label">"connection"</span>
    {connection_text}
</div>
<div class="stage__version">
    <span class="stage__debug-label">"version"</span>
    <VersionLabel />
</div>
```

CSS:

```scss
.stage__version {
    font-size: 0.5em; // smaller than other status-bar items per user request: "always small under the latency"
    opacity: 0.6;
}
```

The status bar appears in every stage layout (`worship_snv`, `worship_pp`, `preach_layout`, `timer_layout`, `bible_layout`, etc.) via the `StatusBar` component, so this single change covers all stage layouts.

The `ndi_fullscreen` layout currently passes `hide_live=true` — confirm whether it shows the StatusBar at all. If `StatusBar` is not rendered in `ndi_fullscreen`, the version label won't appear there either; that's acceptable (NDI fullscreen is a pure video output and projecting any text on it would be a worse violation of the "no clutter on the projector" principle than the version label not appearing).

### 6. Playwright assertion

New helper in `tests/e2e/support.ts`:

```typescript
export async function assertVersionLabel(page: Page, baseURL: string): Promise<void> {
    const versionEl = page.locator('[data-testid="version"]').first();
    await expect(versionEl).toBeVisible({ timeout: 10_000 });
    const text = (await versionEl.textContent())?.trim() ?? '';
    expect(text).toMatch(/^v\d+\.\d+\.\d+(-dev\.\d+)?(\s\(\w+\))?$/);

    const healthRes = await fetch(new URL('/healthz', baseURL).toString());
    const health = await healthRes.json() as { version: string; channel: string };
    const expected = (health.channel === 'release' || health.channel === '')
        ? `v${health.version}`
        : `v${health.version} (${health.channel})`;
    expect(text).toBe(expected);
}
```

Existing E2E tests for each route get one line added after page load:

- `tests/e2e/operator-*.spec.ts`: `await assertVersionLabel(page, baseURL)` (operator already covered today by visual check, this just adds the assertion)
- `tests/e2e/tablet*.spec.ts`: same one line
- A dedicated bible-route test if none exists; otherwise add to existing bible test
- `tests/e2e/stage*.spec.ts` (or tablet pages that load stage): same one line

Per-route assertion (rather than one combined test) is intentional — every existing route test guards its own version label, so a regression on any single route fails fast in the responsible test file.

## Behavior after this change

| Route | Before | After |
|---|---|---|
| `/ui/operator` | Bottom-right footer shows `v0.4.51 (dev)` | Same (component refactored, no visual change) |
| `/ui/tablet` | No version display | Tiny bottom-right corner badge, opacity 0.4 |
| `/ui/bible` | No version display | Tiny bottom-right corner badge, opacity 0.4 |
| `/stage` | No version display | Small line in status bar, immediately under connection/latency |
| Playwright | Operator visual only | Per-route `assertVersionLabel` enforcing format + backend match |

## Testing

### Unit / component tests (Rust)

Not strictly required — the component is a thin wrapper around `/healthz` fetch and string formatting. Existing operator-route Playwright coverage already exercises this path. If `cargo test -p presenter-ui` has a pattern for component tests, add a unit test asserting the format strings; otherwise skip.

### Playwright E2E

For each of operator, tablet, bible, stage — assert via the new `assertVersionLabel` helper:

1. Label exists at `[data-testid="version"]`
2. Label is visible
3. Format matches `^v\d+\.\d+\.\d+(-dev\.\d+)?(\s\(\w+\))?$`
4. Label text equals the value derived from `/healthz` (`v{version}` or `v{version} ({channel})`)

### Manual verification on dev

After CI deploys to dev:

1. Open `http://10.77.8.134:8080/ui/operator` — version label visible bottom-right
2. Open `http://10.77.8.134:8080/ui/tablet` — version label visible bottom-right (small)
3. Open `http://10.77.8.134:8080/ui/bible` — version label visible bottom-right (small)
4. Open `http://10.77.8.134:8080/stage` — version label visible under connection/latency in status bar
5. All four show the same version (e.g. `v0.4.52 (dev)`)

### CI

The new helper runs in the existing `Playwright E2E` job; no workflow changes.

## Closes

- Issue #287 — version display foundation gate fulfilled.

## Risks / unknowns

- **Tablet/Bible CSS file location.** The plan says `crates/presenter-ui/style/tablet.scss` but the actual SCSS structure should be verified. If styles live elsewhere (e.g. inline `<style>`, single `style.scss` aggregator, or one-file-per-component), follow the existing pattern.
- **NDI fullscreen layout.** `hide_live=true` is passed to NdiFullscreen — verify whether `StatusBar` is still rendered in that layout. If it is not (likely the case for a pure NDI passthrough), the version label intentionally does not appear there.
- **`VersionLabel` first paint.** The component fetches `/healthz` async and renders empty until response arrives (~50ms typically). Existing `VersionFooter` has the same flash-of-empty behavior; no regression. Tablet/Bible badges are low-opacity and small, so the flash is unlikely to be noticed.
- **Playwright `assertVersionLabel` timing.** The 10-second visibility timeout matches the existing pattern in the project's E2E helpers and accommodates slow `/healthz` resolution on cold-started server processes.

## Out of scope

- Build-time SHA injection or git-describe-style labels (`v0.4.51` from `/healthz` is sufficient; SHA suffix is nice-to-have, not in #287).
- A dedicated `/version` API endpoint — `/healthz` already returns the same fields.
- Auto-fade or query-param gating on `/stage` — user explicitly wants always-on under the latency display.
- Visual redesign of the operator footer — refactor only; pixel-identical output.
- E2E test for the `ndi_fullscreen` layout — not currently covered by Playwright; if added later, the absence of `StatusBar` there means no assertion needed for that specific layout.
