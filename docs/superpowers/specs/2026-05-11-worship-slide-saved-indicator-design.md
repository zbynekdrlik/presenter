# Worship Slide Editor: Drop Save Button + "Saved ✓" Indicator (Design)

> **Status:** Approved
> **Issue:** #313
> **Created:** 2026-05-11

## Problem

The worship slide editor has three textareas (Main / Translation / Stage) plus a Group input. All four already save on blur via `save_all_fields_from_dom(...)`. A redundant Save button sits in the slide controls, leading the operator to believe they MUST click it before edits persist. The user's literal request:

> "when editing worship slides i need to save each one with the save button, IT NEEDS TO SAVE AUTOMATICALLY AS SOON AS I TYPE SOMETHING IN THERE"

After clarification, the chosen behavior is: **keep the current save-on-blur path, drop the misleading Save button, add a transient per-slide "Saved ✓" badge in the slide header so the operator sees that persistence happened.**

## Goal

Remove operator confusion about when edits persist. After tabbing/clicking out of any editor field, the operator must see a visible badge in the slide header confirming the save (or surfacing failure).

## Approach

Three slice changes to the WASM operator UI:

1. Add per-slide save status to `OperatorState`.
2. Wire `save_all_fields_from_dom` to write that status (Saving → Saved → fade, or Failed).
3. Render the badge in the slide header; remove the Save button from slide controls; add CSS for the fade.

No server changes. No schema changes. No new API.

## Components

### `crates/presenter-ui/src/state/operator.rs`

Add:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveStatus {
    Saving,
    Saved,
    Failed,
}
```

Add a field to `OperatorState`:

```rust
pub save_status: RwSignal<std::collections::HashMap<String, SaveStatus>>,
```

Initialise as `RwSignal::new(HashMap::new())` in the existing constructor.

### `crates/presenter-ui/src/components/slide_list.rs`

#### `save_all_fields_from_dom` — wire the status updates

Currently the function:

1. Reads DOM values
2. Skips if unchanged
3. Spawns an async task that PUTs and ignores the result

Updated flow:

1. Read DOM values
2. If unchanged → return (no status change)
3. Set status to `Saving` for this slide_id
4. Spawn:
   - PUT via `api::presentations::update_slide_with_group(...)`
   - If `Ok`: set `Saved`, then `TimeoutFuture::new(2000).await`, then remove the entry from the map (fades the badge)
   - If `Err`: set `Failed` (sticky; replaced on next save attempt)

The map entry is keyed by `slide_id: String`. `HashMap::get(&slide_id)` is cheap; the `RwSignal<HashMap>` re-renders the slide header on change but Leptos only re-flows the affected `<Show>`, so cost is one row.

#### Slide header — render the badge

In the existing slide-header `<header>` block (around line 583-606), add a sibling element next to the slide group:

```rust
<Show when=move || op.save_status.get().contains_key(&slide_id_for_badge)>
    {move || {
        let status = op.save_status.get();
        let s = status.get(&slide_id_for_badge);
        match s {
            Some(SaveStatus::Saving) => view! {
                <span class="operator__slide-save-indicator" data-role="slide-save-indicator" data-status="saving">"Saving…"</span>
            }.into_any(),
            Some(SaveStatus::Saved) => view! {
                <span class="operator__slide-save-indicator" data-role="slide-save-indicator" data-status="saved">"Saved ✓"</span>
            }.into_any(),
            Some(SaveStatus::Failed) => view! {
                <span class="operator__slide-save-indicator" data-role="slide-save-indicator" data-status="failed">"Save failed"</span>
            }.into_any(),
            None => view! { <span></span> }.into_any(),
        }
    }}
</Show>
```

(Exact closure plumbing chosen by the implementer to satisfy Leptos's borrow rules — the spec captures the data-role attributes and copy strings, which are the contract for tests.)

#### Remove the Save button

Lines 615-635 of `slide_list.rs` — the `<button data-action="save">"Save"</button>` block inside `operator__slide-controls`. Remove the whole `<button>` element including its closure. Keep the `Duplicate` and `Delete` siblings.

#### Group field

The Group `<input>` already saves on blur via `update_slide_with_group`. After this change, the group save path must also update `save_status` (same Saving → Saved → fade or Failed flow). Either: (a) inline the status updates at the Group input's `on:blur` site, or (b) extract a small helper `save_with_status(...)` that wraps the PUT + status updates. (b) is preferred for DRY.

### `crates/presenter-ui/styles/operator.css`

Add:

```css
.operator__slide-save-indicator {
    margin-left: 0.5rem;
    font-size: 0.85em;
    opacity: 1;
    transition: opacity 200ms ease-out;
}

.operator__slide-save-indicator[data-status="saved"] {
    color: #16a34a; /* green */
}

.operator__slide-save-indicator[data-status="saving"] {
    color: #6b7280; /* gray */
}

.operator__slide-save-indicator[data-status="failed"] {
    color: #dc2626; /* red */
    font-weight: 600;
}
```

(The fade-out at 2s is driven by REMOVING the entry from the HashMap in `save_all_fields_from_dom`, not by a CSS animation. The 200ms opacity transition above is for the moment the `<Show>` removes the element from the DOM — Leptos can apply leaving classes if we want a smoother fade, but the simple version is fine.)

## Tests

### Playwright E2E

New file `tests/e2e/operator-slide-save-indicator.spec.ts`. Three scenarios:

1. **Edit + blur shows "Saved ✓"**. Open operator, select a presentation, click into a slide's Main textarea, type a character, blur the textarea (`page.locator('body').click()`). Within 3 seconds, `[data-role="slide-save-indicator"][data-status="saved"]` is visible with text "Saved ✓". Within an additional 3 seconds, it's gone.

2. **No Save button in slide controls**. With a slide in edit mode, assert `button[data-action="save"]` has count 0 in the slide controls.

3. **Failure shows "Save failed"**. Intercept the PUT to return 500. Trigger the same edit+blur. Assert `[data-role="slide-save-indicator"][data-status="failed"]` is visible with text "Save failed" and does NOT auto-fade within 5 seconds.

All three end with `expect(consoleMessages).toEqual([])`.

### Regression test (for the bug-fix line in the completion report)

The bug is "operator must click Save". The regression test that proves the fix is the **no-button** assertion in test scenario 2. Cite this in the completion report:

```
✅ Regression test: tests/e2e/operator-slide-save-indicator.spec.ts:<line of scenario 2> — RED on <test_sha>, GREEN on <fix_sha>
```

## Out of scope

- Bible slide editor (`crates/presenter-ui/src/pages/bible_slides.rs`). Different page, similar pattern. File a follow-up issue if the same friction shows up there.
- Save-while-typing autosave. User explicitly chose blur-only behavior.
- A persistent "Last saved HH:MM:SS" timestamp. Indicator is transient by design.
- Localization. The strings are English-only in the existing UI; matching that for now.

## Risks

| Risk | Mitigation |
|---|---|
| HashMap re-render thrash when many slides save in quick succession | The map is keyed by slide_id; Leptos diffing on a HashMap signal re-runs each subscriber but the body uses `.get()` lookup, which is O(1). At 50 slides × 1 save each that's <1ms WASM. Acceptable. |
| Existing E2E tests targeting the Save button break | Removed in this PR. Adjust or remove those tests as part of this work. |
| Fade timer races with rapid blur/refocus | Each save attempt overwrites the map entry before spawning its own fade timer. The earlier fade timer eventually fires and removes the (now-newer) entry — which is the wrong moment. **Need:** include a per-save token in the entry; the fade only clears if the token still matches. (Implementation detail; the spec calls this out as a known correctness concern to address in the plan.) |
| Operator misses the badge | The badge is 0.85em next to the slide group label, in the operator's gaze when they finish typing. If it proves invisible during real services, we can grow it. |

## Verification checklist

| Check | Method |
|---|---|
| Save button removed | Playwright scenario 2 |
| "Saved ✓" appears within 3s of blur | Playwright scenario 1 |
| "Saved ✓" fades within ~2-5s of appearing | Playwright scenario 1 |
| "Save failed" sticks on PUT failure | Playwright scenario 3 |
| Browser console clean across all three scenarios | `expect(consoleMessages).toEqual([])` |
| Version bump 0.4.73 → 0.4.74 | `Cargo.toml` workspace version |
| WASM clippy clean | `cargo clippy --target wasm32-unknown-unknown -p presenter-ui` |
| Workspace clippy clean | `cargo clippy --workspace --all-targets -- -D warnings -W clippy::all` |
