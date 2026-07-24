---
name: presenter-ui
description: >
  Leptos/WASM frontend (presenter-ui) code-authoring gotchas: view! macro
  pitfalls, keyed <For> identity, reactive vs captured values. Use when editing
  crates/presenter-ui components. For BUILD/clippy/test commands see the deploy skill.
triggers:
  - leptos
  - view! macro
  - "<For>"
  - presenter-ui component
  - wasm frontend
  - sidebar / operator / stage component
---

# Presenter UI (Leptos/WASM) Skill

Build / wasm-clippy / host-test commands live in `.claude/skills/deploy`
(`presenter-ui` is OUTSIDE the workspace — own `Cargo.lock`, wasm32 target). This
skill is the CODE-authoring gotchas.

## `view!` macro: `each` (and attribute values) need a NAMED closure

The `view!` macro does NOT parse an inline `move || …` as an attribute/`each`
value — it errors with `expected identifier, found keyword 'move'` and a cascade
of bogus tag-mismatch errors. Bind the closure to a `let` first, pass it by name:

```rust
// WRONG — compile error inside view!
<For each=move || items().into_iter().enumerate().collect::<Vec<_>>() … />

// RIGHT
let indexed = move || items().into_iter().enumerate().collect::<Vec<_>>();
view! { <For each=indexed key=|(i, _)| *i children=move |(i, _)| { … } /> }
```

A plain `move || single_signal.get()` bound to a `let` is fine; the issue is only
inlining it *inside* the macro. Closures over only `Copy` signals (and the
`StageContext`/`AppContext`, which are `Copy`) can be copied into several `move`
closures, so re-using `items` in multiple derived closures is fine.

## Keyed `<For>`: key by a UNIQUE id, read changing values REACTIVELY (#496)

`key=|e| e.name.clone()` collides when two rows share a name (e.g. a worship set
that repeats a song — same name AND `presentation_id`). Leptos then reuses/mis-
reconciles row DOM and any value captured ONCE in `children` (e.g.
`let name = clean(&e.name)`) sticks at its first-render value.

- **Key by something unique.** When no per-item unique id reaches the client,
  enumerate and key by the **index** (`key=|(idx, _)| *idx`).
- **Read anything that can change REACTIVELY inside `children`** — the active
  class AND the display text — from the signal by index, not from the captured
  item. Under index keys a captured `display_name`/`is_active` would otherwise go
  stale when the list is edited live:

```rust
children=move |(idx, _entry)| {
    let snap = ctx.snapshot;
    let class = move || if snap.with(|o| /* active row == idx */) { "…--active" } else { "…" };
    let name  = move || snap.with(|o| /* entries[idx].name, cleaned */ );
    view! { <div class=class>{name}</div> }
}
```

- Disambiguating WHICH occurrence is active needs server help: the stage snapshot
  carries `active_entry_index` (per-occurrence), threaded from the trigger
  (`StageStateRequest.entry_index`). The sidebar resolves it via
  `worship_pp_helpers::active_sidebar_index(entries, snapshot_active_index)`
  (explicit index, fallback to first `is_active`). Don't re-derive by name/id.

## Per-row live state in a `<For>`: key on identity, read the state through a `Memo`

Keying a row on its live state (`format!("{id}-{state}")`) makes every state flap destroy and rebuild
the whole row — buttons included. Key on identity (`id`, plus anything that changes the row's
CONTROLS, e.g. `is_active`) and let the badge/hint read the state reactively.

A plain closure capturing a `String` id is **not `Copy`**, so it cannot be moved into the four places
that need it (class, `data-*`, text, hint). A `Memo` **is** `Copy`:

```rust
let row_id = source.id.clone();
let status = Memo::new(move |_| statuses.with(|l| l.iter().find(|s| s.id == row_id).cloned()));
let state_str = move || status.get().map(|s| s.state).unwrap_or_default();   // Copy closure ✓
```
The DTO must then derive `PartialEq` (Memo requires it). And `data-state=state_str` — NOT
`data-state=move || state_str()`, which clippy rejects as a redundant closure.

**A missing entry is not an error state.** `unwrap_or_default()` yields `""` on first paint and for a
row added since the last poll — render that as a neutral "Checking…", never as the failure copy
("NDI unavailable"), or a healthy server accuses itself for one frame (or forever, if the poll errors).

**`on_cleanup` does not work for a `gloo_timers` Interval** in this crate: the Interval is not `Send`,
and leptos' `on_cleanup` requires `Send` for the crate's **host** (non-wasm) test build, which is how
`cargo test --lib` runs here. Use `interval.forget()`, like every other poller on the settings page —
the timer dies with the page navigation (there is no client-side router).

Reminder: this crate is **excluded from the workspace** — `cargo test -p presenter-ui` fails.
Run `cd crates/presenter-ui && cargo test --lib`.

## The toast `<div>` is ALWAYS mounted — assert visibility via `data-visible`, not DOM presence (#558 W1)

`components/toast.rs` renders `<div data-role="toast" data-visible=… class:operator__toast--visible=…>`
UNCONDITIONALLY — the element exists in the DOM at all times, with an EMPTY text node when no
toast is active. `operator.css` fades it purely via `opacity` (no `display`/`visibility` change).
Two E2E assertions that look correct both fail to detect "no toast shown":

- `toHaveCount(0)` — the element always exists (count is always 1).
- `.not.toBeVisible()` — Playwright's visibility check ignores `opacity`; an `opacity: 0` element
  still has a non-empty bounding box and no `visibility: hidden`, so it reports VISIBLE regardless.

Assert the component's own state instead: `expect(locator).toHaveAttribute("data-visible", "false")`.

## A `console.warn` from a WASM poller can red-line an unrelated E2E

`leptos::logging::warn!` reaches the browser console, and several specs (e.g.
`operator-slide-scroll.spec.ts`) assert **zero console errors AND warnings**. The settings card is
embedded in the operator page, so its 5 s poll runs there too — and the in-flight fetch a page
teardown aborts fails with `TypeError: Failed to fetch`, warning once per closed page and failing
those specs. Do not answer that by swallowing the error (stale data the operator trusts is worse
than none): tolerate ONE failure silently, and on the second in a row fall back to a "Checking…"
state and log once (`STALE_AFTER_FAILURES` in `video_sources.rs`).

## E2E test library names: NEVER a standalone single-char differentiator token (#558 X1)

Every 409/race E2E test creates two test libraries and opens one via
`openPresentationInEditMode(page, libName)`, which searches by LIBRARY NAME to
surface the presentation. `search_presenter` (`repository/search.rs`) splits the
query into tokens (`query_tokens` — splits on non-alphanumeric) and
`search_libraries`'s DB-level match is `Condition::any()` **OR-ed across every
token**. A lone single-character differentiator like `` `${lib} A` `` /
`` `${lib} B` `` makes `"a"`/`"b"` its OWN token — and `SearchName.contains("a")`
then matches a huge slice of the REAL seeded dev libraries (e.g. `GRACE
PEREMOT`, `OKSANA`, `TARZUS`, `ROMANI ARCHA` all contain a bare `a`). Those
unrelated matches flood `matched_library_ids`, and `search_presentations`'
result-page `LIMIT` gets exhausted by their (real, large) presentation counts
before the test's own presentation is ever reached — the search dropdown then
shows the test's LIBRARY as a match but never surfaces its PRESENTATION, and
`openPresentationInEditMode` times out waiting for
`[data-role="search-result-item"][data-kind="presentation"]`.

**Fix: glue the differentiator directly onto the preceding word** — `RaceA`,
`Conflict409A`, `Conflict409RecoverA` — never a space-separated lone letter.
Every existing 409/race test in `wasm-slide-multiselect.spec.ts` already follows
this; keep new ones consistent.

## E2E: the library SIDEBAR lists only FAVORITES — select fresh libraries via the modal (#570)

`[data-role="library-item"]` in the operator sidebar renders only favorite
libraries (plus the count button). A library an E2E test just created via the
API is NOT in the sidebar — a `hasText` click on it times out with zero
matches. Select it through the "Show all libraries" modal instead:

```ts
await page.locator('[data-role="library-more"]').click();
await page
  .locator(`[data-role="library-row"][data-library-id="${lib.id}"] .operator__list-button`)
  .click();
await page.waitForSelector('[data-role="presentation-list"]');
```

Related JSON-shape gotcha: `SlideText` serializes as an object — assert
`slide.content.main.value`, never `slide.content.main` directly.

## Operator header design rule (#573, user directive 2026-07-24)

Connection/status indicators (Resolume chips etc.) live in the TOP brand/surface-nav row
(`.operator__brand-nav`, next to the Stage/Camera/Tablet/Timer links), never next to the
Stage Output controls in `operator__header-right`.
