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

## `(pointer: coarse)` CSS/JS scoping needs `hasTouch: true` in Playwright to test (#569)

A touch-only UI behavior (a CSS `@media (pointer: coarse)` rule, or a JS
`matchMedia("(pointer: coarse)")` gate — e.g. "only force-rotate / only auto-fullscreen on a real
phone/tablet, never on a desktop browser window") needs the `(pointer: coarse)` media feature to
actually MATCH in a test. Playwright/Chromium only reports a coarse pointer when the **browser
context** declares touch support — a plain viewport resize (`setViewportSize`) does NOT do this;
by default every Playwright context reports a FINE (mouse/trackpad) pointer regardless of window
shape. Use `test.use({ hasTouch: true })` (works inside a nested `test.describe` block, doesn't
disturb the file's top-level `beforeAll`/`afterAll`) to emulate a real touch device, and keep a
SEPARATE test with the default (no `hasTouch`) context to prove the desktop/fine-pointer case is
correctly left untouched — one test per side of the gate, not one test assuming both.

## `env!("CARGO_PKG_VERSION")` is USELESS here for server-version comparison (#574)

`presenter-ui` has its OWN unrelated Cargo.toml `version` (e.g. `0.1.43` — bumped
independently of the workspace's `0.4.x`, since the crate is workspace-`exclude`d, see
the deploy skill). `env!("CARGO_PKG_VERSION")` inside `presenter-ui` compiles to THAT
number, never the server's real version — `info_popover.rs` already does this (a
pre-existing, out-of-scope display quirk, left alone in #574). Do NOT reach for
`env!("CARGO_PKG_VERSION")` to detect "did the server get redeployed under this open
tab" — it would mismatch on every single load. Instead capture the baseline from the
tab's OWN first `/healthz` response (`header.rs`'s `known_version` signal) and compare
LATER polls against that captured value, never against a build-time constant.

## E2E: NEVER mock a non-2xx response in a zero-console spec (#598)

Chrome itself logs `Failed to load resource: the server responded with a status of <N>`
for EVERY non-2xx fetch — `page.route` mocks included. A spec that fulfills a route with
`status: 500` (or 401/404) and also asserts `expect(consoleMessages).toEqual([])` can
NEVER pass; it fails identically on retry (not flake). To exercise a client fetch-failure
branch with a clean console, fulfill with a MALFORMED 200 instead:
`route.fulfill({ status: 200, contentType: "application/json", body: "not-json" })` —
`get_json()` in `crates/presenter-ui/src/api/mod.rs` funnels non-2xx (`ApiError::Status`)
and parse failure (`ApiError::Deserialize`) into the same `Err` path, and gloo-net's
`json()` parses in pure Rust (zero browser console noise).

## `#571`'s re-entry (double-submit) guard is on CREATE handlers only, never on `on_delete` (#641)

`op.submitting` guards `on_save` (library/playlist — covers BOTH create AND rename, same
handler) and the 3 presentation create paths (`on_create_blank`/`on_paste_confirm`/
`on_import_confirm`). **`on_delete` in `library_modal.rs`/`playlist_modal.rs` has NO
guard and needs none** — delete is idempotent, so a double-click's second call is a
harmless no-op/404, not a duplicate-creation bug. Don't assume "guarded path" ⇒ every
mutating handler in the same file; check which ones are actually non-idempotent.

## create-from-PASTE names the presentation from the pasted text's `Title:` line, not the name input (#641)

`on_paste_confirm` (`presentation_modal.rs`) calls `song_parser::extract_title(&text)`
on the PASTED text, falling back to `"Untitled"` — it never reads `create_name`/
`presentation-create-name` (which is hidden via `style:display=none` on the paste
sub-step anyway). To control the resulting presentation's name in a test, put
`Title: <name>` as the pasted text's first line — `pad_title_number` only rewrites a
purely-numeric prefix followed by whitespace (`"6 Foo"` → `"006 Foo"`), so a name with
no leading digit+space passes through unchanged. `is_metadata_line` strips the `Title:`
line itself out of the resulting slides.

## The `.pro` import E2E fixture's name is FIXED and collides with real seeded dev data (#641)

`tests/e2e/fixtures/test-import.pro` (checked in since the import modal was built,
`ea0b3dcd`, but never actually used by any test before `#641`) decodes to a presentation
named **"088 Alive with you"** — baked into the file's protobuf bytes, not something a
test can override via the upload form (only the file itself is posted, no name field).
The seeded dev library **"NEW LEVEL"** already contains a real presentation with that
exact name (`data/libraries/NEW LEVEL/088 Alive with you.pro`). Any test that imports
this fixture into `selectLibrary()`'s "first sidebar favorite" risks a same-name
collision that makes an "exactly 1" count assertion meaningless depending on which
library happens to be first. Create a **fresh, never-favorited library via the API**
(`request.post` `/libraries`) and select it through the `#570` "Show all libraries"
modal instead — guaranteed empty, no dependency on seed-data contents.

## A self-consuming `dispose(self)` + `impl Drop` pairing ALWAYS double-fires — guard with a flag (#637)

The `Watchdog`-style pattern (`ndi_watchdog.rs`'s `stop(&self)` + `Drop`) is safe because `stop`
takes `&self` and the caller keeps the value alive afterward — `Drop` only ever fires later, once,
at actual scope-end. Copying that pattern onto a handle whose primary teardown method takes `self`
by VALUE (`pub(crate) fn dispose(self) { self.remove_listeners(); }`) is NOT equivalent: the moment
`dispose()` returns, `self` still runs through Rust's normal drop glue, so `Drop::drop` fires
**immediately afterward on every ordinary call**, not just in the "caller forgot to dispose"
fallback case the doc comment describes. If the shared cleanup method (`remove_listeners`) isn't
provably idempotent from the CALLER's observable side (here: an E2E test counting
`removeEventListener` invocations, not just DOM correctness — `tests/e2e/stage-ndi-playback-guard.spec.ts`),
this reads as a leak-looking net-count failure even though the real DOM never errors. Fix: add a
`disposed: Cell<bool>` (or similar) flag inside the shared cleanup fn, `if self.disposed.replace(true) { return; }`
before doing the actual work — makes the second call (always `Drop`, right after `dispose()`) a true
no-op, restoring the "Drop = safety net only" intent instead of the pair always double-executing.
See `ndi_playback_guard.rs`'s `PlaybackGuardHandle`.

## Reuse `attachConsoleErrorCollector` + `REPO_ROOT` from `support.ts` — don't hand-roll them (#641)

`tests/e2e/support.ts` already exports `attachConsoleErrorCollector(page, errors)` (the
`browser-console-zero-errors.md` collector, used by 6+ existing spec files) and
`REPO_ROOT` (`= process.cwd()`, used for repo-relative fixture/data paths elsewhere,
e.g. `stage-empty-db-console.spec.ts`). New tests should import and reuse both instead
of re-inlining the same `page.on("console", ...)` block or a fresh `process.cwd()` call
— a deep-review pass on `#641` flagged both as avoidable duplication.

## A Chrome `VERBOSE` console entry is invisible to the zero-console gate — pin the fix with a DOM assertion instead (#677)

`attachConsoleErrorCollector` (and this repo's zero-console-errors rule generally) only
collects `console.error`/`console.warn`. Chrome's own DOM/accessibility hints — e.g.
`[DOM] Password field is not contained in a form` for a `type="password"` input with no
`<form>` ancestor — log at `VERBOSE`, a level neither `error` nor `warning`. A spec that
only asserts `expect(consoleMessages).toEqual([])` will happily pass on a REGRESSION of
this class of bug, because the collector never sees VERBOSE lines at all. To pin a fix
for this class of smell, assert the actual DOM relationship the browser is complaining
about, not the console:

```ts
const hasFormAncestor = await apiKey.evaluate((el) => el.closest("form") !== null);
expect(hasFormAncestor).toBe(true);
```

When wrapping a field in a `<form>` to satisfy this, remember the button that already
saves via `on:click` MUST stay `type="button"` (never `type="submit"`), and give the new
`<form>` its own `on:submit=move |ev| ev.prevent_default()` — otherwise pressing Enter in
any field newly implicitly-submits the form, which with no `action` GET-reloads the
current URL and blows away all WASM app state. See `crates/presenter-ui/src/pages/ai.rs`'s
`ai-settings-form` and `tests/e2e/wasm-ai-chat.spec.ts`'s Enter-key regression test.

## `screen.orientation` tracks PHYSICAL orientation, not the DISPLAYED viewport — never counter-rotate from an instantaneous read (#694)

`tablet_orientation.rs`'s `install_orientation_flip_watcher` counter-rotates the tablet UI
180° when the device is landscape-secondary (`body[data-tablet-flip]` → CSS `rotate(180deg)`).
Two spec facts (W3C Screen Orientation, verified — never guess this API) make an
instantaneous read of `screen.orientation` a BUG source on a rotation-locked phone:

1. **`screen.orientation.type`/`.angle` reflect the device's PHYSICAL orientation and
   `change` fires on physical tilt** — an OS rotation lock keeps only the DISPLAYED
   viewport fixed. So lifting / laying a locked phone flat makes the sensor transiently
   report `landscape-secondary` while the viewport never rotates. A watcher that sets the
   flip from that read MANUFACTURES the very 180° flip it was built to suppress (the #694
   live-event report: "keď ho dvihnem a položím sa vyhodnotí že som dal telefón hore
   nohami"). The distinguisher between this false trigger and a genuine turn is
   **STABILITY**: a real turn settles at secondary and stays; a lift/put-down flap reverts.
   Fix pattern: apply the flip only after the reading stays stable past a short settle
   window (debounce SET ~300ms); CLEAR immediately (a stuck upside-down UI is worse).
2. **The angle→type mapping is NATURAL-ORIENTATION-DEPENDENT.** Natural-portrait phone:
   `0°=portrait-primary, 90°=landscape-primary, 180°=portrait-secondary, 270°=landscape-secondary`.
   Natural-landscape tablet: `0°=landscape-primary, 180°=landscape-secondary`. So a
   `.angle === 180` check means portrait-secondary on a PHONE, landscape-secondary on a
   TABLET — a `.angle` fallback mis-fires across device classes. Trust ONLY
   `screen.orientation.type` (broadly supported: Chrome/Firefox/Edge, Safari 16.4+); an
   engine without `.type` simply never flips, which is the safe default.

Also: the 90° portrait fallback (`@media (orientation: portrait)` in `tablet.css`) is PURE
CSS and re-evaluates itself — the JS watcher needs NO `resize` listener for it, and a
`resize`-triggered flip re-read is a false-trigger source (mobile browser-chrome show/hide
on lift fires `resize`; a 180° landscape↔landscape turn never resizes).

**E2E note:** the static `mockScreenOrientationType` fake can't exercise SEQUENCES. Use the
mutable `installDynamicOrientation` + `window.__setOrientation(type, angle, fireChange)`
helper (`tablet-orientation-lock.spec.ts`) to drive a transient flap (RED) vs a settled
turn (AC3 guard). `test.use({ hasTouch: true })` is required for the `(pointer: coarse)`
gate to match (see the `(pointer: coarse)` note above).
