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
- **First read = one answer (#850 S8):** `GET /bible/console-start?translation=<this browser's main>`
  brings preferences + books (same shape as `/bible/books`, `null` when unknown or unread). The books
  effect takes them only when the code matches; anything else reads `/bible/books`, so a switch and a
  failure behave as before. Two requests in a row cost a cold phone ~760 ms in the connection queue.
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
- **The console is the whole Live tab** on a phone and a desktop (B2c removed the old inputs,
  the selects and the header verse search). On a desktop the field takes the focus whenever
  Biblia's Live tab becomes the shown page (`field.rs`, deferred like #819); never on a phone.
  Desktop layout and order: `bible-page.md` (mockup v3).
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
  clear. The primary is
  `disabled` only with no target; every wait is `aria-disabled`, so the focus stays for the next
  Enter or Space. No dead tap: over a failed preview it reads "SKÚSIŤ ZNOVA" and asks again, past
  the book's last verse it reads "KONIEC KNIHY" (`at_end`, cleared by ◀, typing, or a
  translation that goes on past it: `recheck_end`) and refuses. ◀ / ▶ refuse while a show is on its way: the show moves on from the preview
  when it lands, a step in between skips a part.
- **iOS keyboard:** `state::keyboard_inset` writes `body`'s `--kb-inset` from the visual
  viewport (0 when pinch-zoomed); the phone bar uses `bottom: var(--kb-inset)`; the tab bar hides
  while the console field has focus (`:has()`, iOS 15.4+). Playwright cannot open an iPhone
  keyboard — the inline ZOBRAZIŤ is the guaranteed path.

## The console is the only writer of `bs.slides` (B2c)

The grid holds the current target only. The console empties it the moment its target changes (and
when there is no target), so no card of an older verse stays tappable while a new one loads or
fails. The B2a/B2b reconciliation with the old inputs (`classic_loads`, `console_takes`,
`console_owns_grid`, `translation_echo`, the 300 ms auto-load) went with them in B2c.

- **A secondary that did not load** (`resp.warning`, #826 NLT down) is a toast as well as the
  card's inline text: a desktop draws no preview card.
- **Translation bar** (`translation_bar.rs`, pure half `state::bible_translations`, #921 owner
  2026-11-09: "keď budem mať 100 prekladov, celá plocha budú tlačidlá"): each row is ONE line that
  never grows or scrolls. It holds the PREFERRED translations (`BibleState::preferred_main` /
  `preferred_secondary`, default SEB ROH SEVP KJV / NLT KJV) up to `console_chip_cap` (phone 3,
  desktop 4; the second row one fewer, "žiadny" takes a place — measured: phone row 330 px beside its
  label, desktop 284 px at 900–1024 px), the selected translation in the LAST place when it is not
  preferred (`row_chips`, never an extra chip), and ▾ (`console-translation-menu`, never
  `aria-pressed`) opening `components::translation_list` as a popover: every installed translation,
  short code + full name, grouped Slovenské / České / Anglické / Ďalšie (`translation_groups`), the
  main list without the NLT (`offered_as_main`). A pick selects and closes; Escape (root keydown)
  and the transparent backdrop close without one; an open list counts as an overlay for the window
  keys (`operator.rs`). Chips: `title` = full name, `aria-label` = "SEB — <name>",
  `min-width: max-content` (the cap is what keeps the line inside the panel, pinned by
  `bible-translation-bar.spec.ts` with four SNV translations routed in from the KJV,
  `tests/e2e/extra-translations.ts`). Labels: "Preklad" / "Pod ním" inline on a phone, "Hlavný
  preklad" / "Druhý preklad" as row headings on a desktop. It writes the same `BibleState`
  signals as the selects, so #701 holds: localStorage per browser + the server default. "žiadny"
  cannot clear the server default (`BiblePreferencesDraft` reads None as "keep").
- **Preferované preklady** (Biblia → Nastavenie, `pages/bible_preferred.rs`): one checkbox per
  translation and row (`preferred-translation`, data-kind + data-translation); a tick appends, an
  untick removes, saved at once as `BiblePreferencesDraft::preferred_main|secondary` (`Some` =
  replace, normalised in core: trimmed, deduplicated, ≤ 12). Stored inside the existing
  `bible-preferences` app-setting JSON (`#[serde(default)]`, no migration); `None` = the defaults,
  never written. The console's auto-save sends no lists, so it never clears them.
- **Log fields:** `/bible/resolve` carries `query` (non-empty input) + `device`; `/bible/trigger-slide`
  carries `source: "console"` + `device`. The server keeps `device` only for `touch` / `desktop`
  and counts `console` only when claimed. A prepared show keeps its own route (`source=prepared`).
- **Text search** (`text_search.rs`, `state::bible_console_search`): only when
  `wants_text_search` (words without a reference; never `zid 11 1`), 300 ms after the last
  keystroke, 10 hits labelled with the translation; a tap previews (a `Picked` from `hit_target`),
  never shows. An answer counts only for its input AND main translation. `build_fts_query` splits
  on punctuation, counts characters and keeps a combining accent inside its word (persistence
  `fts_query_tests`). Numbers stay as tokens: the AI tool calls it too, `Ján 3:16` must not
  answer with every verse holding "Ján". The console is the operator's only full-text verse
  search since B2c (the header search is gone on Biblia).
- **Window keys on Biblia (B2d, `state::global_keys`, host-tested):** with the focus outside a
  field, ←/→ press the console's ◀ / ▶ (the button's own `click()`, so its refusals hold: nothing
  loaded, a show on its way, the end of the book) and never move the song; Space or `/` focus the
  field (prevented, nothing typed). Only on the Live tab (`[data-bible-panel=live][data-visible=true]`),
  in Live and Edit mode alike. Piesne keeps ←/→ = song slides and Space = header search in Live
  mode; other sections: Space = header search, arrows nothing. Never taken: any key while a modal
  or the Viac sheet is open (`overlay_open`: `op.open_modal`, `more_sheet_open`, the "Upraviť
  prezentáciu" dialog), and Space / `/` / Enter while a button, link or `role="button"` has the
  focus (`KeyFocus::Button`): Space presses Zhasnúť, ◀ / ▶, a translation button like any button
  (review of 65eab96c, a swallowed Space jumped to the field). Arrows still step from a focused
  button: after the desktop Enter the focus sits on Zobraziť. Pinned by `bible-console.spec.ts`
  (stage POST count unchanged on Biblia, one more on Piesne).

## "Táto bohoslužba" (B2d, `pages/bible_console/shown_today.rs`, server `state/bible_shown.rs`)

- Server list, in memory, shared by every device: max 20, one entry per passage (code + chapter + verses, so another translation's name is the same entry), a repeat moves up,
  empty after the local date changes. `show_bible_on_wall` records it BEFORE publishing `BibleSlide`,
  because the console reads `GET /bible/shown-today` when `active_bible_slide` changes (so Companion,
  AI and another operator's shows arrive too). No `LiveEvent` carries it.
- A sermon's slide is never recorded (`BibleShowSource::Sermon`, logged as `prepared`): its text is the
  preacher's cut, a chip would preview the whole verse, and it has its row under "Pripravené kazateľom".
  A plain presentation's slide is. Companion and the AI name only the book, so the code comes from
  `canonical_book`, never from the reference alone.
- One chip per shown slide's verses and a list of 20 are accepted: the label says what was on the wall.
  A chip for a verse another translation lacks toasts and keeps the preview; a tap waits for a show on its way, like ◀ / ▶. A tap on the previewed chip goes
  back to part 1 (the key does not change, so the preview effect will not).
- A chip is a `Picked` for the empty field with the entry's translation as `main`, so `rebase` renames
  it after a translation switch. A tap only previews. Hidden while empty and while the field has text.
- Phone: mounted after the preview, a long list never hides what Zobraziť sends. Desktop: `order` under
  the bar, so a chip appearing after a show never moves Zobraziť. The spec has
  its own server because the list starts empty (`bible-shown-today.spec.ts`).
