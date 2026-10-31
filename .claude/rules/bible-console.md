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
- **`Picked`** (a tapped row / chip / search hit, a step) is valid only for the input it was made
  in; otherwise the first row is the target. It outlives a translation switch through
  `bible_console_step::rebase` (new book name; a translation without that verse falls back to the
  rows), and a walk compares by code + segment, never the renamed target. One effect clears it on
  every input change (a comparison alone brings it back when the same text is typed again), and
  disarms `focus_show`, on a main change too: Enter arms that only with a target, any later edit cancels.
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

- **Zhasnúť lives in the on-air strip**, at the other end of the page from ZOBRAZIŤ, so a second
  tap on a split verse never clears the wall. The strip's `min-height` holds two lines of text and
  the codes run on with the reference (a separate flex item left it three lines at 360 px); only a
  reference longer than that, a long verse list, moves the field.
- **A show in flight is `aria-disabled`, never `disabled`:** a disabled button drops the focus to
  `<body>`, and the next Space goes to the global search, not to the next part. `show` itself
  refuses a second send. A failed show's inline error is set only while its verse is still the
  target; the toast always.
- **A held key never sends:** the desktop Enter moves the focus to ZOBRAZIŤ and its auto-repeat
  lands there, so the button drops an Enter or Space `keydown` with `repeat` set. E2E: a second `keyboard.down`
  without an `up` is the repeat.
- **Whoever sends is on the strip:** `/bible/trigger` (Companion, AI) stores the wall output and
  publishes `BibleSlide` too; only its Resolume path stays legacy until A2b.
- **No layout shift under a press:** ZOBRAZIŤ and Zhasnúť prevent `pointerdown` / `mousedown`'s
  default too. A blur removes the inline button, everything below moves up and the click lands on
  nothing: on a short verse the first tap with the keyboard open was lost (a long one hid it, the
  sticky bar does not move).
- **Only the primary button sends** (`show`): enabled while `can_show` (the preview shows the
  current key, no show in flight); it posts exactly the previewed slide (`trigger_request`).
- **The inline ZOBRAZIŤ** on the preview card exists only while the field has focus and prevents
  `pointerdown` / `mousedown`'s default: the field keeps focus (the iOS keyboard stays) and the
  button never disappears under a blur re-render before its `click`.
- **On-air strip = server truth:** `BibleSlide` / `BibleCleared`, the console's own show answer,
  `GET /bible/active-slide` on mount, on a reconnect and on `visibilitychange` → visible (a locked
  phone's socket may have missed the event). An answer is dropped when the signal changed while it
  was on its way; the show's own answer never replaces a later `triggered_at`, and over an empty strip (maybe a clear) it is set and the server re-read. Visibility in E2E: `Object.defineProperty(document,
  "visibilityState", …)` + a dispatched `visibilitychange`.
- **Phone layout:** the Live tab hides the slide grid; the bar is `fixed` above the tab bar (a show
  empties the field and the rows go, a sticky bar jumped 180 px under the finger). ◀ / ▶ prevent
  the press default like ZOBRAZIŤ. `overflow: visible` up to `.operator__panel--bible` stays.
- **Errors:** a failed preview shows inline (Enter retries); a failed show shows inline AND a
  toast; neither changes the strip.

- **Stepping** (`state::bible_console_step`, `pages/bible_console/stepping.rs`): split slides →
  the typed segments (`walk`, kept while the field is empty after a show) → the neighbouring
  verse via `Versification::step`, never into another book. ◀ / ▶ only move the preview (a
  `Picked` for the current input); after a show `advance_after_show` clears the field, pins the
  next target for the empty input and sets `after_show` → "ĎALŠÍ: …". Typing resets it. The
  input effect keeps a `Picked` made for the value the input just took, or the pin dies with the
  clear; a load from the old inputs drops it itself (the input may not change). The primary is
  `disabled` only with no target; every wait is `aria-disabled`, so the focus stays for the next
  Enter or Space. No dead tap: over a failed preview it reads "SKÚSIŤ ZNOVA" and asks again, past
  the book's last verse it reads "KONIEC KNIHY" (`at_end`, cleared by ◀, typing or an old-inputs
  load) and refuses. ◀ / ▶ refuse while a show is on its way: the show moves on from the preview
  when it lands, a step in between skips a part.
- **iOS keyboard:** `state::keyboard_inset` writes `body`'s `--kb-inset` from the visual
  viewport (0 when pinch-zoomed); the phone bar uses `bottom: var(--kb-inset)`; the tab bar hides
  while the console field has focus (`:has()`, iOS 15.4+). Playwright cannot open an iPhone
  keyboard — the inline ZOBRAZIŤ is the guaranteed path.

## One owner of `bs.slides` (until B2c)

The grid holds the current target only. The console empties it the moment its target changes, so
no card of an older verse stays tappable while a new one loads or fails. A load from the old inputs
bumps `classic_loads` (the console clears its field and lets go); the console bumps `console_takes`
on every new target, and an old-inputs answer that started before it is dropped. The 300 ms
auto-load skips too when the console took over during its wait, and when it is only the echo of a
translation switch (`translation_echo`: `list_books` re-sets chapter and verse, `set` always
notifies) while `console_owns_grid`: it would bump `classic_loads` and wipe what is being typed.
