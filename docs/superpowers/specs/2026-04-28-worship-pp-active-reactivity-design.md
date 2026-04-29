# Worship-PP Active-Highlight Reactivity Fix — Design

**Date:** 2026-04-28
**Status:** Proposed
**Scope:** Frontend (presenter-ui WASM, single component file) + one E2E test extension

## Goal

Fix the worship-pp stage playlist sidebar so the active-song highlight follows the currently-presenting song. Today the highlight gets stuck on whichever song was active when the sidebar first rendered, and never moves when the user triggers a different song.

## Root cause

In `crates/presenter-ui/src/components/stage/worship_pp.rs:178-193`:

```rust
<For
    each=playlist_entries
    key=|entry| entry.name.clone()
    children=move |entry| {
        let class = if entry.is_active {
            "stage-pp__playlist-entry stage-pp__playlist-entry--active"
        } else {
            "stage-pp__playlist-entry"
        };
        let display_name = clean_song_name(&entry.name);
        view! { <div class=class>{display_name}</div> }
    }
/>
```

The `class` is a string computed *once* — when the row is first inserted into the DOM — from the captured `entry: StagePlaylistEntry` value. When the server later pushes a new `playlist_entries` Vec with `is_active` flipped, Leptos's `<For>` diffs by key (`entry.name`); identical keys mean Leptos reuses the existing DOM node. The captured `entry.is_active` stays at its initial value, so the class never updates.

## Fix

Make the class a reactive closure that reads `is_active` from the `playlist_entries` signal at evaluation time:

```rust
children=move |entry| {
    let entry_name = entry.name.clone();
    let is_active = move || {
        playlist_entries
            .with(|entries| entries.iter().any(|e| e.name == entry_name && e.is_active))
    };
    let class = move || {
        if is_active() {
            "stage-pp__playlist-entry stage-pp__playlist-entry--active"
        } else {
            "stage-pp__playlist-entry"
        }
    };
    let display_name = clean_song_name(&entry.name);
    view! { <div class=class>{display_name}</div> }
}
```

Why this works:
- `class=move || ...` — Leptos tracks signals read inside the closure. The closure reads `playlist_entries` (a signal), so Leptos re-runs the closure whenever `playlist_entries` changes.
- The lookup is by name match (same identity Leptos's `For` already uses for keying). Two distinct entries can't share the same name in a playlist context (the existing `For` key would already collapse them into one row).
- `display_name` stays as a static string — names don't change for a row, only `is_active` does.
- No flicker, no DOM rebuild.

## Why I didn't catch this in the previous PR's E2E

The existing test "active playlist entry has high-contrast background distinct from inactive" seeds ONE playlist entry, triggers ONE presentation, and asserts that the rendered active row has a non-transparent background distinct from inactive rows. It verifies *static* correctness (the active class is applied somewhere) but not *reactivity* (the active class moves to a different row on a new trigger).

The fix here adds a *second* test that explicitly asserts the highlight MOVES between rows on consecutive triggers — the regression guard that should have existed from the start.

## Testing

### New E2E test in `tests/e2e/stage-worship-pp.spec.ts`

```typescript
test("active highlight moves to the new song when the operator triggers a different presentation", async ({ page }) => {
    // 1. Switch stage layout to worship-pp.
    // 2. Seed a playlist with TWO presentations (P1, P2), each with at least one slide.
    // 3. Trigger P1 onto stage with playlist context.
    // 4. Open /stage. Wait for sidebar with two entries to render.
    // 5. Assert: row whose data identifies P1 has `.stage-pp__playlist-entry--active`,
    //    row for P2 does NOT.
    // 6. Trigger P2 onto stage (same playlist).
    // 7. Wait for the sidebar to reflect the change (poll up to 5s).
    // 8. Assert: row for P2 now has `.stage-pp__playlist-entry--active`,
    //    row for P1 does NOT (the highlight moved).
    // 9. Cleanup: delete the playlist.
}
```

The two rows can be distinguished by their text content (presentation name passed through `clean_song_name`) or by an index (first/second `.stage-pp__playlist-entry`).

### Existing test stays as-is

The existing test "active playlist entry has high-contrast background distinct from inactive" remains a valid regression guard for static correctness. The new test guards reactive correctness.

## Risks / unknowns

- If two entries truly share the same name, the lookup `iter().any(|e| e.name == entry_name && e.is_active)` will highlight both rows. The existing `For` key collapses same-named entries to a single rendered row anyway, so this isn't a new failure mode — it's the same behavior as today.
- `playlist_entries` must already be a properly-tracked signal. It is (it's a Memo derived from `stage_snapshot`), so the closure re-runs on every update. Verified by reading worship_pp.rs imports.

## Out of scope

- Refactoring how `playlist_entries` is sourced.
- Active-highlight animation (CSS transitions on the class change). Not requested.
- Worship-snv (no sidebar — not affected).
- Operator UI playlist active-row highlight (different code path, different signal — not in this user's report).

## File-level overview

| File | Change |
|------|--------|
| `crates/presenter-ui/src/components/stage/worship_pp.rs` | Replace the `<For children=...>` closure with the reactive-class version above |
| `tests/e2e/stage-worship-pp.spec.ts` | Add the new "highlight moves on consecutive trigger" test |
| `Cargo.toml` (workspace), `crates/presenter-ui/Cargo.toml` | Version bump to 0.4.40 / next presenter-ui patch |
