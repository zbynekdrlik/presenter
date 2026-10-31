---
paths:
  - "crates/presenter-ui/src/pages/bible_console/**"
  - "crates/presenter-ui/src/state/bible_console*.rs"
  - "crates/presenter-ui/src/state/bible_translations.rs"
  - "crates/presenter-ui/src/state/keyboard_inset.rs"
  - "crates/presenter-ui/styles/bible_console.css"
  - "tests/e2e/bible-console.spec.ts"
---

# The verse console (#850 B2)

- **The reference is resolved in the browser**: `presenter_core::bible::resolver::lookup` against
  `state::bible_console::versification(bs.books)` — the MAIN translation's `/bible/books`, verse
  counts placed by chapter NUMBER. Never a server round trip per keystroke.
- **Pure half / thin half.** Everything decidable without a DOM lives in `state/bible_console*.rs`
  with host tests over the core's structure fixtures (`include_str!` of
  `presenter-core/src/bible/resolver/testdata/structure-*.json` — they are `/bible/books`
  snapshots). Components only wire signals.
- **Row keys carry label + detail.** A translation switch renames a row with the same passage; a
  keyed `<For>` never re-renders an unchanged key (ui skill #496).
- **`Picked`** (a tapped row / chip / search hit, a step) is valid only for the input AND the main
  translation it was made in; otherwise the first row is the target.
- **Stale answers are dropped.** `preview_key` = passage + main + secondary + character limit; a
  `/bible/resolve` answer is applied only while it still equals the target's key (slow Wi-Fi).
- **Settle markers, never timing:** `[data-role=bible-console]` `data-preview-state` /
  `data-preview-ready` / `data-preview-reference`.
- **On a phone the console replaces `.bible__classic`** (the old inputs, removed in B2c); the
  desktop keeps them and their autofocus until then.
- **iOS autocorrect** is set with `set_attribute` (tachys 0.1.9 has no typed `autocorrect`).
- **E2E idioms:** a failed request is a malformed 200 (`not-json`); a race holds the request with
  `page.route` and releases it after the competing input is confirmed; a request is never
  unrouted before it answered.

- **Only the primary button sends** (`show`): enabled while `can_show` (the preview shows the
  current key, no show in flight); it posts exactly the previewed slide (`trigger_request`).
- **The inline ZOBRAZIŤ** on the preview card exists only while the field has focus and prevents
  `pointerdown` / `mousedown`'s default: the field keeps focus (the iOS keyboard stays) and the
  button never disappears under a blur re-render before its `click`.
- **On-air strip = server truth:** `BibleSlide` / `BibleCleared`, the console's own show answer,
  `GET /bible/active-slide` on mount and on `visibilitychange` → visible (a locked phone's socket
  may have missed the event). Visibility in E2E: `Object.defineProperty(document,
  "visibilityState", …)` + a dispatched `visibilitychange`.
- **Phone layout:** the Live tab hides the slide grid; the sticky bar works only because every
  ancestor up to `.operator__panel--bible` has `overflow: visible` — a new `overflow` on the
  catalog or the tab panel breaks it.
- **Errors:** a failed preview shows inline (Enter retries); a failed show shows inline AND a
  toast; neither changes the strip.

## One owner of `bs.slides` (until B2c)

The grid holds the current target only. The console empties it the moment its target changes, so
no card of an older verse stays tappable while a new one loads or fails. A load from the old inputs
bumps `classic_loads` (the console clears its field and lets go); the console bumps `console_takes`
on every new target, and an old-inputs answer that started before it is dropped.
