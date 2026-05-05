# Operator view isolation: keep worship-only UI off bible/timer/ai/settings

**Issue:** #295

**Date:** 2026-05-04

**Branch:** dev (workspace 0.4.62)

## Problem

The operator UI shows worship-specific elements at the top of every view, not just the worship view. Two leaks:

1. The "stage preview" panel in the operator header — song name, Ableton ON / Follow OFF toggles, and the current/next slide text — currently displays on `worship`, `timers`, `ai`, and `settings`. It is correctly hidden only on `bible`.
2. The worship section that contains the **libraries / playlists / presentations** panels (with their `+` buttons) is rendered on every view because its CSS lacks the default `display: none` from `.operator__panel`. The bible / timers / ai / settings panels each have a `.operator__panel` base class that hides them by default; the worship section does not.

Result: a user navigating to bible / timers / ai / settings still sees the worship song name plus Ableton/Follow buttons in the header AND the libraries/playlists/presentations panels with their `+` buttons.

## Goal

Worship-specific elements appear only when the operator's view is `worship`. The diagnostic / utility elements in the operator header (`stage-monitor` health counter, `clear-slide` 🧹 button, `bible-preview`) remain visible on the views they currently target.

## Architecture

Two trivial CSS / Rust changes. No logic changes. No new components.

### Fix 1 — `crates/presenter-ui/src/components/stage_preview.rs:170`

Flip the inline-style condition on the `[data-role="worship-preview"]` div.

Current:

```rust
style=move || {
    if ctx.view.get() == "bible" { "display:none" } else { "" }
}
```

New:

```rust
style=move || {
    if ctx.view.get() == "worship" { "" } else { "display:none" }
}
```

### Fix 2 — `crates/presenter-ui/styles/operator.css`

After the existing rule at line 1410:

```css
body.operator[data-view="worship"] [data-view-panel="worship"] {
  display: flex;
}
```

Add:

```css
body.operator:not([data-view="worship"]) [data-view-panel="worship"] {
  display: none;
}
```

The existing show-on-worship rule continues to apply when the view IS worship. The new hide rule applies on every other view. Specificity is symmetric across the two rules; declaration order keeps the show-on-worship rule winning when both could match (which never happens because they're mutually exclusive).

## Surfaces affected

| Element | Currently shown on | After fix |
|---|---|---|
| StagePreview `worship-preview-wrap` (song name, Ableton/Follow buttons, current/next text) | worship, timers, ai, settings | worship only |
| StagePreview `bible-preview` | bible | bible (unchanged) |
| StagePreview `stage-monitor` (health counter) | all views | all views (unchanged) |
| StagePreview `clear-slide` (🧹) | all views | all views (unchanged) |
| operator__worship section (libraries / playlists / presentations + their `+` buttons) | all views | worship only |
| operator__panel--bible / timers / ai / settings | their respective view | unchanged |

## Testing

### Playwright E2E

One new test file `tests/e2e/operator-view-isolation.spec.ts` (or extend an existing operator spec if the project prefers grouping). Asserts the visibility matrix above.

For each non-worship view (bible, timers, ai, settings):

1. Navigate to `/ui/operator/{view}`
2. Wait for WASM ready (`body[data-wasm-ready="true"]`)
3. Assert `[data-role="worship-preview"]` is not visible
4. Assert `[data-view-panel="worship"]` is not visible (the libraries / playlists / presentations section)
5. Assert `[data-role="stage-monitor"]` IS visible
6. Assert `[data-role="clear-slide"]` IS visible
7. Assert the matching panel `[data-view-panel="{view}"]` IS visible (sanity)

For the worship view:

1. Navigate to `/ui/operator`
2. Assert `[data-role="worship-preview"]` IS visible
3. Assert `[data-view-panel="worship"]` IS visible
4. Assert the libraries panel renders (`[data-role="library-list"]` or equivalent visible)

Per project rule (`ci/browser-console-zero-errors.md`), the test must collect `console.error` and `console.warning` messages and assert the array is empty at the end of each test.

### Manual verification on dev

After deploy, open `http://10.77.8.134:8080/ui/operator` (worship) and visually verify the song name and `+` buttons are present. Then click into Bible, Timers, AI, and Settings; verify both elements disappear from the header and main area.

## Files

| File | Change |
|---|---|
| `crates/presenter-ui/src/components/stage_preview.rs:170` | Flip inline-style condition (`bible` hide → `worship` show) |
| `crates/presenter-ui/styles/operator.css` (after line 1413) | Add 3-line `:not([data-view="worship"])` rule |
| `tests/e2e/operator-view-isolation.spec.ts` | New E2E test |

## Acceptance

- Playwright E2E test passes locally and on CI.
- Workspace tests still pass (`cargo test --workspace`, WASM clippy).
- Browser console stays clean throughout the E2E test.
- Manual verification on dev confirms the fix on every view.

## Out of scope

- Refactoring `worship-preview-wrap` into its own gated component.
- Auditing other potentially-leaky UI elements outside StagePreview (none identified during code reading).
- Server-side or persistence changes (none required).
