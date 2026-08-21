---
paths:
  - "crates/presenter-ui/src/pages/bible.rs"
  - "crates/presenter-ui/src/state/bible.rs"
  - "tests/e2e/wasm-bible.spec.ts"
---

# Bible page (`/ui/operator/bible`) — DOM contract & E2E determinism

## Book-list has TWO render variants — keep their `data-*` contract identical (#727)

`BookList` renders either the FULL list (one `<button data-role="book-item" data-book-code=… data-active=…>` per book) OR, when a book is selected AND `book_filter` is empty, a COLLAPSED single item. Both variants MUST expose the SAME automation attributes — `data-role="book-item"`, `data-book-code`, `data-active`. The collapsed variant once dropped `data-book-code`, so any test reading the active book's code got `null`; the "preserves book" E2E could then only ever pass via the "cleared" branch and timed out (10 s) whenever the book was actually preserved. When you add/remove a `data-*` on one variant, mirror it on the other.

## Async-effect settle signal, NOT `expect.poll`-with-timeout (#727)

The translation-switch effect (`selected_translation` change → `spawn_local(list_books)` → preserve-or-clear `selected_book`) is async; nothing in the DOM signalled *when it finished*, so E2E raced it. Fix pattern: the effect publishes a **settle marker as its LAST synchronous write** — `books_translation.set(Some(code))` after `books`/`selected_book` are set — exposed on the book-list container as `data-books-translation`. Leptos coalesces the block's synchronous signal writes into one render flush, so the render that first shows `data-books-translation == <newTrans>` already reflects the settled selection. Tests `await expect(bookList).toHaveAttribute("data-books-translation", target)` (a real async-completion gate, load-tolerant) then read the settled state — never a poll that guesses render timing. Per `no-timeout-band-aids.md` a bigger poll timeout cannot fix a predicate the preserved branch never satisfies.

## Reproducing bible UI behaviour without a local build (Tier-0)

Local cargo builds are banned here. Drive the live dev server instead: `http://10.77.8.134:8080/ui/operator/bible`, `/bible/translations`, `/bible/books?translation=<code>`. Book codes are canonical and SHARED across full translations (`eng-kjv`, `slk-seb`, `slk-roh` all carry `1CH`/`1JN`); the list is sorted by code (first book = `1CH`). Partial translations exist (`slk-mil` = 4 gospels only) — switching TO one whose books lack the selected code exercises the "cleared" path.
