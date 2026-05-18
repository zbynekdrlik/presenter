# Operator Surface-Nav Strip — Design

> **Issue:** #326 — Operator page jump links to other surfaces (camera, tablet, bible, stage)
> **Status:** Accepted (2026-05-18)

## Problem

Once a user enters the operator UI at `/ui/operator`, there is no quick way to open the sibling surfaces (`/ui/camera`, `/ui/tablet`, `/stage`, `/overlays/timer`) for verification. They have to back out to the landing page `/` and click through, or type the URL.

## Mental model

The user's framing:

- **Operator app** = worship + bible + timers + AI + settings. One Leptos shell, multiple internal views switched via the top tab strip (`.operator__view-nav`). The route `/ui/bible` already redirects to `/ui/operator/bible`.
- **Tablet** = preacher's view. Never needs to navigate to other surfaces.
- **Camera, Stage** = clean output surfaces. Anything overlaid is undesirable (visible on the wall, or distracts the director).

So the switcher appears only on operator chrome and only opens external surfaces (not internal tabs).

## Scope

**In scope:**

- New row of 4 jump pills on operator chrome, opening in new browser tab.
- Targets: Stage (`/stage`), Camera (`/ui/camera`), Tablet (`/ui/tablet`), Timer Overlay (`/overlays/timer`).
- Covers every `/ui/operator/*` route (worship, bible, timers, AI, settings) — one component, single insertion point.
- Playwright E2E asserting presence + correct hrefs + `target=_blank`, and absence on tablet/camera surfaces.

**Out of scope:**

- Switcher on `/ui/tablet`, `/ui/camera`, `/stage`.
- Internal targets (Worship, Bible) — Worship is the current view; Bible is reachable via the existing `operator__view-nav` button.
- Conditional / role-based visibility.
- Keyboard shortcuts to jump between surfaces.
- Active-state highlighting (operator is always the source; there is no "current external surface").

## Visual layout

```
┌──────────────────────────────────────────────────────────────────────────────┐
│ Presenter v0.4.85  [search]   [Worship][Bible][Timers][AI][Settings]   [Stage Output ▾] [Live][Edit] ☰ │
├──────────────────────────────────────────────────────────────────────────────┤
│  Open in new tab:   [ Stage ↗ ]  [ Camera ↗ ]  [ Tablet ↗ ]  [ Timer ↗ ]    │
├──────────────────────────────────────────────────────────────────────────────┤
│  ... slide grid / controls ...                                               │
```

The strip is its own row directly below the existing `<header class="operator__header">`, before `<SearchResults />`. Left-aligned label "Open in new tab:" followed by 4 pill links. Each pill includes the surface name and a small ↗ icon (Unicode `\u{2197}`) to signal external-tab behavior.

## Component design

**New component:** `crates/presenter-ui/src/components/surface_nav.rs` (~40 LoC).

```rust
use leptos::prelude::*;

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

Pure static. No state, no signals, no API calls. Read-only links.

## Integration

**File:** `crates/presenter-ui/src/pages/operator.rs:162` — view! block.

```rust
view! {
    <Header />
    <SurfaceNav />          // NEW
    <SearchResults />
    <main class="operator__main">
        ...
    </main>
    ...
}
```

**File:** `crates/presenter-ui/src/components/mod.rs` — add `pub mod surface_nav;`.

The component renders unconditionally on operator chrome. Internal tab switching (`ctx.view`) does not affect it — the strip is always visible.

## CSS

**File:** `crates/presenter-ui/styles/operator.css` — append rules mirroring `.operator__view-nav` visual language.

```css
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

Mobile: pills wrap onto a second line at ≤800px viewport. No hamburger collapse; the strip is short enough to fit when wrapped.

## Test plan

### Playwright E2E

**New file:** `tests/e2e/operator-surface-nav.spec.ts` (~50 LoC).

Three test cases, all started via `startTestServer()` helper consistent with neighbouring specs:

1. **Strip visible on operator with 4 correct anchors:**
   - Navigate to `/ui/operator`. Wait for `body[data-wasm-ready="true"]`.
   - Assert `[data-role="surface-nav"]` is visible.
   - For each of `Stage`, `Camera`, `Tablet`, `Timer`:
     - Assert `[data-role="surface-nav-link"][data-target="<Name>"]` exists.
     - Assert its `href` equals the expected URL.
     - Assert its `target` attribute equals `_blank`.
     - Assert its `rel` attribute contains `noopener`.

2. **Strip visible on the bible internal view (operator chrome):**
   - Navigate to `/ui/operator/bible`. Wait for WASM ready.
   - Assert `[data-role="surface-nav"]` is visible.
   - Assert all 4 anchors still present (the strip is shell-level, not per-view).

3. **Strip absent on tablet and camera surfaces:**
   - Navigate to `/ui/tablet`. Wait for WASM ready. Assert `[data-role="surface-nav"]` count is 0.
   - Navigate to `/ui/camera`. Wait for WASM ready. Assert `[data-role="surface-nav"]` count is 0.

Every test must also assert zero console errors / warnings (project convention from `browser-console-zero-errors.md`).

### Manual verification (post-deploy on dev)

- Open `http://10.77.8.134:8080/ui/operator` in Chromium.
- Confirm strip visible with 4 pills under the header.
- Click each pill — confirm new browser tab opens at the expected URL.
- Switch operator tabs (Worship → Bible → Timers → AI → Settings) — confirm strip remains visible and unchanged.
- Open `http://10.77.8.134:8080/ui/tablet` and `http://10.77.8.134:8080/ui/camera` — confirm strip absent.

## Constraints

- **No backend changes.** Pure frontend addition. No new HTTP endpoints, no schema, no migrations.
- **No new dependencies.** Standard Leptos `view!` macro + static `<a>` tags.
- **No state leakage.** SurfaceNav holds no signals, makes no network calls, does not depend on `AppContext`.
- **Single PR per `autonomous-batch-issue-development.md`.** Component + integration + CSS + E2E ship together.
- **Version bump first** per `version-bumping.md`. Workspace 0.4.85 → 0.4.86 before any other code change.

## Risks and mitigations

| Risk | Mitigation |
|------|------------|
| Strip crowds the operator at narrow widths | Mobile breakpoint at 800px wraps pills; no fixed minimum width. |
| User confuses the new strip with the internal tab strip (Worship/Bible/...) | Visual distinction: `<a>` tags with rounded pill shape and ↗ icon, label "Open in new tab:" explicitly signals external-tab behavior. |
| Future surface (e.g. new dashboard) requires updating multiple places | All 4 targets live in a single `targets` array at the top of `surface_nav.rs`. Adding one means one line. |

## Non-goals

- Active-state highlighting on the current source surface — the strip only appears on operator, and operator has no entry for itself.
- Single-tab same-window navigation — explicit user requirement: open in new chrome tab.
- Surface-switcher on tablet/camera/stage — explicit user requirement: tablet is preacher-only, camera/stage are clean output surfaces.
