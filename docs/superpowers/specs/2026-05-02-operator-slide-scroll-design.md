# Operator Slide-List Scroll UX — Design

**Date:** 2026-05-02
**Status:** Proposed
**Scope:** Frontend (`presenter-ui`) — `crates/presenter-ui/src/components/slide_list.rs` + minor CSS
**Issue:** [#271](https://github.com/zbynekdrlik/presenter/issues/271) — slide-list scroll behavior during live worship

## Goal

Three operator-UX fixes to the `/ui/operator` slide-list during live worship services:

1. **Lookahead scroll:** when the active slide moves to a new row, ensure exactly one row of upcoming slides is visible BELOW the active row (proactive, not reactive).
2. **Linear wheel scroll:** intercept mouse wheel events and apply a fixed-step `scrollBy`, neutralising macOS scroll acceleration so each notch advances ~1 row deterministically.
3. **Load-at-start on song open:** when the operator selects a new presentation, scroll to the top so the first slide is visible — not the bottom of the previous song's scroll position.

## Why

The user reports during live services that:

1. Clicking the next slide only causes the slide-list to scroll once the active slide is already off-screen — there's no lookahead. The operator can't see what's coming up.
2. Wheel scrolling feels "exponential, first slow then fast" on the iMac+mouse setup. macOS applies scroll acceleration before browser delivery; for the operator's deliberate "scroll to next slide" gesture, this is jumpy and distracting.
3. Opening a new worship song lands the slide-list scrolled near the bottom (whatever the previous scroll state was), forcing the operator to scroll back to slide 1 every time they switch songs.

These three concerns share the same code surface (`slide_list.rs` + the `.operator__slides` container in `operator.css`) and the same mental model — "the operator should always be able to see what's next without manual scrolling".

## Approach

All changes inside `crates/presenter-ui/src/components/slide_list.rs`. No new files, no new public APIs. CSS unchanged unless the wheel handler needs `overscroll-behavior: contain` to prevent page scroll bleed-through.

### Concern 1 — Lookahead scroll

Replace the reactive logic in `scroll_slide_into_view` (currently `slide_list.rs:909`) with proactive lookahead. After locating the active slide element:

- Find the slide whose `data-slide-id` matches the slide at the active index + 3 (next row, since the grid is `grid-template-columns: repeat(3, ...)` — confirmed in `operator.css:1081`).
- If that "next-row anchor" element exists and its bottom edge sits below the container's bottom, scroll the container so the anchor's bottom is flush with the container's bottom.
- If the active slide itself is above the container's top, top-align it (existing behavior preserved as a fallback when navigating backward).
- If no next-row anchor exists (last row), fall back to the current "ensure active is visible" logic (bottom-align if below).

The arithmetic uses the slide's *position in the rendered list*, not Cartesian coordinates — Leptos renders slides in DOM order, and the next row is always 3 DOM siblings later in the grid. This adapts automatically if the responsive breakpoint changes column count, because the next-row anchor jump is `+columns_per_row`. For now `columns_per_row = 3` is hardcoded; if a future redesign uses a different count, the constant is one place.

### Concern 2 — Linear wheel scroll

Add an `on:wheel` handler attached to the `.operator__slides` container (or its closest scrollable ancestor inside the slide list view). The handler:

```rust
on:wheel=move |ev: WheelEvent| {
    ev.prevent_default();
    let direction = ev.delta_y().signum();        // -1, 0, or +1
    if direction == 0.0 { return; }
    let step_px = step_for_wheel(&container);     // ~card height + gap
    let _ = container.scroll_by_with_x_and_y(0.0, direction * step_px);
}
```

`step_for_wheel(container)` measures the first `.operator__slide-card` child via `getBoundingClientRect` and adds the row gap (CSS computes `0.9rem` ≈ 14.4px at default font-size). If no card is rendered yet, fall back to a constant `120` (covers typical card height).

Calling `prevent_default()` blocks the native accelerated scroll entirely. The single `scroll_by` per wheel event linearises everything: each notch = 1 row, no acceleration possible.

The handler must be attached as a non-passive listener so `preventDefault` works. Leptos's `on:wheel` directive defaults to passive=false in most cases, but the project's existing event-listener patterns should be checked. If passive is required, fall back to `addEventListener("wheel", ..., { passive: false })` via `web_sys`.

### Concern 3 — Load-at-start on song open

Add a separate Effect that watches `ctx.selected_presentation_id` (or whatever signal reflects the currently-loaded song). When it changes:

```rust
Effect::new(move |prev: Option<Option<PresentationId>>| {
    let current = ctx.selected_presentation_id.get();
    if current != prev.flatten() && current.is_some() {
        gloo_timers::callback::Timeout::new(0, || {
            scroll_slides_to_top();
        })
        .forget();
    }
    current
});
```

The 0ms timeout defers to the next event-loop tick so the new presentation's slides have actually rendered before we set `scroll_top = 0`. Helper `scroll_slides_to_top()` queries `.operator__slides` and sets its `scroll_top` to 0.

This effect runs in addition to the existing `scroll_slide_into_view` effect at line 215. Order: load-at-start runs first (instant top), then if a current_slide_id is also set on the new presentation (e.g. operator restored a saved playlist position), the lookahead effect scrolls down to that slide. The two effects don't fight because they trigger on different signals (presentation change vs. slide change).

## Components touched

- `crates/presenter-ui/src/components/slide_list.rs`:
  - `scroll_slide_into_view` (line 909) — rewritten for lookahead
  - New helper `scroll_slides_to_top` (private fn)
  - New helper `step_for_wheel` (private fn)
  - New Effect for presentation-change scroll-to-top
  - New `on:wheel` handler attached to the slides container element
- `crates/presenter-ui/styles/operator.css`:
  - Possibly add `overscroll-behavior: contain` to `.operator__slides` so wheel events that exhaust the container's scroll don't bubble up to the page (prevents accidental page scroll when at the boundary)

## Behavior after this change

| Scenario | Before | After |
|---|---|---|
| Click slide 1 (row 1) | Slide 1 visible (no scroll change) | Slide 1 visible + slide 4 (row 2) visible below it |
| Click slide 4 (row 2) | If slide 4 is off-screen, scroll until it's at bottom | Slide 4 visible + slide 7 (row 3) visible below it |
| Click slide that is last row | Scroll to show it | Scroll to show it (lookahead skipped — no next row exists) |
| Wheel-scroll one notch | Variable scroll distance with macOS acceleration | Deterministic 1-row scroll per notch |
| Wheel-scroll fast | Multiple compounded notches accelerate | Each notch = 1 row, fast scroll is linear |
| Open a new worship song | Slide list at previous scroll position | Slide list scrolled to top (slide 1 visible) |
| Open new song with current_slide_id set | First-row scroll then jump to current slide | Same — load-at-start runs first, then lookahead scrolls to current |

## Testing

### Playwright E2E

New file `tests/e2e/operator-slide-scroll.spec.ts`:

1. **Lookahead test:** load a 12+ slide worship song. Click slide 4. Assert `slide 7` (next row) is visible (`getBoundingClientRect().bottom <= container.getBoundingClientRect().bottom`).
2. **Wheel test:** dispatch a `WheelEvent` with `deltaY: 100` on the slides container. Assert `container.scrollTop` increases by exactly the measured step (1 row), regardless of `deltaY` magnitude.
3. **Load-at-start test:** load a 20-slide song, scroll to bottom. Switch to a different song. Assert `container.scrollTop === 0` after the new song's slides render.
4. Console must be empty (per `ci/browser-console-zero-errors.md`).

### Manual verification on dev

1. Open `http://10.77.8.134:8080/ui/operator/worship` with a 20+ slide song. Click slide on row 3. Verify row 4 visible below.
2. Wheel-scroll the slide list. Each notch should advance approximately 1 row, with no acceleration after multiple notches.
3. Switch to a different song. First slide should be visible at top (no manual scroll-up needed).

## Out of scope

- Configurable STEP via settings (YAGNI; runtime measurement adapts to font scale automatically)
- Smooth-scroll animation between rows (deliberately linear/instant; user wants determinism, not motion)
- Trackpad-specific tuning (user uses iMac+mouse; if trackpad input becomes a concern later, separate PR)
- Lookahead behavior on responsive breakpoints with different column counts (currently always 3 columns; if breakpoints reduce columns, the constant `3` is one place to adjust)
- Horizontal scroll (the slides container is `overflow-y: auto` only)
- The "wheel feels exponential" feedback could also be partially due to macOS's "Tracking" speed setting in System Settings; this PR fixes the in-app behavior to be deterministic, but if the user reports it still feels off after this lands, the next investigation step is the OS-level mouse settings — out of scope for code

## Risks / unknowns

- **`prevent_default()` on `on:wheel`:** Leptos's wheel event handler may default to passive in some Leptos versions. If `prevent_default()` is silently ignored, the linearisation won't take effect. Mitigation: verify in Step 1 of implementation that the handler actually intercepts and `console.log` confirms `preventDefault` was called. If passive, switch to `web_sys::EventTarget::add_event_listener_with_callback_and_add_event_listener_options` with `passive: false`.
- **`+3` for next-row anchor:** assumes `grid-template-columns: repeat(3, ...)`. Confirmed by reading `operator.css:1081`. If responsive breakpoints reduce columns at some viewport width (currently they don't — verify with `grep "operator__slides" crates/presenter-ui/styles/operator.css`), the arithmetic needs to read the actual column count via `getComputedStyle(container).gridTemplateColumns`.
- **Effect signal name (`selected_presentation_id`):** the spec refers to `ctx.selected_presentation_id`. The actual context field name should be verified in `crates/presenter-ui/src/state/operator.rs` before implementation. If the signal is named differently (e.g. `current_presentation_id`, `active_presentation_id`), the implementer adapts.

## Closes

- Issue #271 — operator slide-list scroll behavior.
