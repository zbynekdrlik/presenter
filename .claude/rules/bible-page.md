---
paths:
  - "crates/presenter-ui/src/pages/bible.rs"
  - "crates/presenter-ui/src/pages/bible_reference.rs"
  - "crates/presenter-ui/src/state/bible.rs"
  - "crates/presenter-ui/src/state/bible_range.rs"
  - "tests/e2e/wasm-bible.spec.ts"
  - "tests/e2e/bible-range-hint.spec.ts"
---

# Bible page (`/ui/operator/bible`) — DOM contract & E2E determinism

## Book-list has TWO render variants — keep their `data-*` contract identical (#727)

`BookList` renders either the FULL list (one `<button data-role="book-item" data-book-code=… data-active=…>` per book) OR, when a book is selected AND `book_filter` is empty, a COLLAPSED single item. Both variants MUST expose the SAME automation attributes — `data-role="book-item"`, `data-book-code`, `data-active`. The collapsed variant once dropped `data-book-code`, so any test reading the active book's code got `null`; the "preserves book" E2E could then only ever pass via the "cleared" branch and timed out (10 s) whenever the book was actually preserved. When you add/remove a `data-*` on one variant, mirror it on the other.

## Async-effect settle signal, NOT `expect.poll`-with-timeout (#727)

The translation-switch effect (`selected_translation` change → `spawn_local(list_books)` → preserve-or-clear `selected_book`) is async; nothing in the DOM signalled *when it finished*, so E2E raced it. Fix pattern: the effect publishes a **settle marker as its LAST synchronous write** — `books_translation.set(Some(code))` after `books`/`selected_book` are set — exposed on the book-list container as `data-books-translation`. Leptos coalesces the block's synchronous signal writes into one render flush, so the render that first shows `data-books-translation == <newTrans>` already reflects the settled selection. Tests `await expect(bookList).toHaveAttribute("data-books-translation", target)` (a real async-completion gate, load-tolerant) then read the settled state — never a poll that guesses render timing. Per `no-timeout-band-aids.md` a bigger poll timeout cannot fix a predicate the preserved branch never satisfies.

## Reproducing bible UI behaviour without a local build (Tier-0)

Local cargo builds are banned here. Drive the live dev server instead: `http://10.77.8.134:8080/ui/operator/bible`, `/bible/translations`, `/bible/books?translation=<code>`. Book codes are canonical and SHARED across full translations (`eng-kjv`, `slk-seb`, `slk-roh` all carry `1CH`/`1JN`); the list is sorted by code (first book = `1CH`). Partial translations exist (`slk-mil` = 4 gospels only) — switching TO one whose books lack the selected code exercises the "cleared" path.

## Chapter / verse inputs bound to the selected book (#825)

`ReferenceInputs` lives in `pages/bible_reference.rs` (moved out of the
1000-line-capped `bible.rs`). A typed chapter / verse is bounded on change /
input AND on Enter by `state::bible_range::{bound_chapter, bound_verse}`
(host-tested, reusing `clamp_selection`), the boxes carry `max=` and a "/ N"
next to the label (`data-role="chapter-max" | "verse-max" | "verse-end-max"`),
and a clamp shows `data-role="bible-range-hint"` ("Kniha má len N kapitol" /
"Kapitola má len M veršov", Slovak plural forms) until the next valid value or a
book change. Three traps:

- Never pass a typed verse END through `clamp_selection`'s end logic: it turns
  `end <= start` into `None` ("whole chapter"), but the #702 mirror sets
  `end = start` for the single-verse fast path. `bound_verse` bounds the end like
  the start.
- After a clamp, write the bounded value back into the `<input>` itself
  (`show_bounded`): the signal may already hold that value, so a re-render alone
  can leave "60" visible.
- That write-back makes the browser fire `change` with the CLAMPED value as soon
  as the focus moves (Enter → focus to the next box; verse end's Enter blurs) —
  synchronously, inside the keydown handler. A plain "valid value → clear the
  note" then wipes the note before it ever renders. Every commit goes through
  `bible_range::next_hint`: a commit that changes nothing keeps the note,
  whichever box it came from (Enter on the untouched mirrored end box too);
  only a NEW valid value clears it. A verse-start commit also moves the end
  through the #702 mirror, so it goes through `next_verse_start_hint` (an
  unchanged start that moves the end is a new value). Likewise a re-commit of
  the same chapter must not reset the verses.
