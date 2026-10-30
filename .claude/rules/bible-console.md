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
