# Bible page keyboard-nav + diacritic-insensitive book search

> **Closes #257.** Two related improvements on `/ui/operator/bible` (live tab) so the operator can drive the page from the keyboard during a service without touching the mouse.

## Goal

1. **Keyboard chain.** Open the bible page, start typing the book name — Enter commits the top-of-list match and moves focus to the next field in the chain: `book-filter → chapter-input → verse-start → verse-end → book-filter (reset)`.
2. **Diacritic-insensitive book search.** Typing `lukas` matches `Lukáš`. Typing `1moj` matches `1. Mojžišova`. Matching also ignores case, punctuation, and whitespace (same normalisation already used server-side by `normalise_book_key`).

## Architecture

- **Shared normalisation:** Promote `presenter_core::bible::search::normalise_book_key` from `pub(crate)` to `pub` and re-export it via `presenter_core::bible::normalise_book_key`. The WASM UI calls the same function the server uses for canonical name lookup — single source of truth.
- **Filtering:** `BibleState::filtered_books()` in `crates/presenter-ui/src/state/bible.rs` switches from `book.book.to_lowercase().contains(&filter)` to `normalise_book_key(book.book).contains(&normalise_book_key(filter))`.
- **Keyboard chain:** Each input in the bible live panel (`book-filter`, `chapter-input`, `verse-start`, `verse-end`) gets an `on:keydown` handler that intercepts `Enter`. The handler commits the current value into the underlying `RwSignal`, then calls `.focus()` on the next input via a `NodeRef`.
- **Auto-focus:** A `NodeRef` on the `book-filter` input plus a `node_ref` mount-time `.focus()` call on `BibleLiveTab` covers the "just start typing" requirement on initial load. The verse-end → book-filter step in the chain reuses the same `NodeRef`.
- **Filter reset after chain completes:** When the user presses Enter on `verse-end`, the chain ends: focus returns to `book-filter` AND `bs.book_filter` resets to empty so the list collapses to the loaded book (existing collapse behaviour at `bible.rs:431`) and the operator can immediately start typing the next book.

## Decisions (from brainstorm)

- **Enter target:** first book in the filtered list (top visible). Matches user wording "highest book".
- **Verse-end Enter:** blur + focus returns to book-filter. The existing 300 ms debounced auto-load already triggers the passage fetch on signal change; no extra Load Passage click is needed.
- **Auto-focus:** on mount AND after every chain completes.

## Out of scope (deferred)

- **Synonym book names** (e.g. Slovak `1. Mojžišova` ↔ canonical `Genezis`). Different from substring/diacritic-insensitive matching — synonym handling needs the server-side canonical alias table. That's #310's territory.
- **Chapter / verse normalisation:** numeric inputs only, no normalisation needed.
- **Diacritic handling in chapter/verse number parsing:** N/A — numeric.

## Testing

- **Unit:** `presenter_core::bible::search::normalise_book_key` already has unit coverage for diacritic stripping. Add one more case asserting Slovak `Lukáš` → `lukas`.
- **Unit:** `BibleState::filtered_books` add a test case for diacritic-insensitive matching (requires lift to module-public if not already).
- **E2E:** new Playwright spec `tests/e2e/wasm-bible-keyboard-nav.spec.ts` with two scenarios:
  - **A. Diacritic search:** load `/ui/operator/bible`, type `lukas` in book-filter, assert filtered list shows `Lukáš` and that book is the top match.
  - **B. Keyboard chain:** type `lukas`, Enter → assert chapter-input focused. Type `6`, Enter → assert verse-start focused. Type `38`, Enter → assert verse-end focused. Press Enter on empty verse-end → assert book-filter focused AND book_filter input value is empty.
- Browser console must be clean (no errors/warnings) per `browser-console-zero-errors`.

## Files touched

| File | Change |
|---|---|
| `crates/presenter-core/src/bible/search.rs` | `normalise_book_key` → `pub`. Add one more test case. |
| `crates/presenter-core/src/bible/mod.rs` | `pub mod search;` + `pub use search::normalise_book_key;`. |
| `crates/presenter-ui/src/state/bible.rs` | `filtered_books()` uses `normalise_book_key`. Add module-public test for diacritic-insensitive match. |
| `crates/presenter-ui/src/pages/bible.rs` | Add `NodeRef`s, `on:keydown` handlers on 4 inputs, mount-time `.focus()`. |
| `tests/e2e/wasm-bible-keyboard-nav.spec.ts` | New file with two scenarios above. |

No schema change. No API break. ≤200 LoC est. Bundle-safe.
