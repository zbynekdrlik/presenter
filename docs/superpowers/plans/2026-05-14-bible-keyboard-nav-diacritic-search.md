# Bible keyboard-nav + diacritic search — implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans (inline) — controller will run tasks in this session. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add keyboard-driven navigation chain and diacritic-insensitive book search to the bible live page. Closes #257.

**Spec:** `docs/superpowers/specs/2026-05-14-bible-keyboard-nav-diacritic-search-design.md`.

**Architecture:** Promote `normalise_book_key` to public in `presenter-core`. Switch `BibleState::filtered_books` to use it. Add `NodeRef`s + `on:keydown` Enter handlers on the four inputs (`book-filter`, `chapter-input`, `verse-start`, `verse-end`) to step focus through the chain, returning to `book-filter` after verse-end.

**Version status:** dev = 0.4.76, main = 0.4.75 → dev > main, no bump needed.

---

### Task 1: RED — Playwright E2E spec asserting full keyboard chain + diacritic match

**Files:**
- Create: `tests/e2e/wasm-bible-keyboard-nav.spec.ts`

- [ ] **Step 1: Write the failing test**

Use existing `tests/e2e/wasm-bible.spec.ts` patterns for server start, navigation, and `data-role="book-item"` selectors. Two scenarios:
1. Type `lukas` → assert at least one `[data-role="book-item"]` rendered AND its label contains `Lukáš`.
2. Type `lukas`, press Enter → assert `document.activeElement` matches `[data-role="chapter-input"]`. Type `6`, press Enter → activeElement is `[data-role="verse-start"]`. Type `38`, press Enter → activeElement is `[data-role="verse-end"]`. Press Enter on empty verse-end → activeElement is `[data-role="book-filter"]`, value empty. Assert console clean.

- [ ] **Step 2: Run test, see RED**

```bash
npm run test:playwright -- wasm-bible-keyboard-nav
```

Expected: BOTH scenarios fail (no normalisation, no Enter handlers).

- [ ] **Step 3: Commit RED**

```bash
git add tests/e2e/wasm-bible-keyboard-nav.spec.ts
git commit -m "test(e2e): assert bible keyboard chain + diacritic book search (#257)"
```

### Task 2: Promote `normalise_book_key` to `pub`

**Files:**
- Modify: `crates/presenter-core/src/bible/search.rs:3` — `pub(crate)` → `pub`
- Modify: `crates/presenter-core/src/bible/mod.rs:4` — `mod search;` → `pub mod search;` + add `pub use search::normalise_book_key;` to the `pub use` block

- [ ] **Step 1: Edit search.rs**

```rust
pub fn normalise_book_key(input: &str) -> String {
```

- [ ] **Step 2: Edit mod.rs — promote and re-export**

```rust
pub mod search;

pub use search::normalise_book_key;
```

(Keep the existing `pub use bible::...` chain in `lib.rs` working — `normalise_book_key` will be reachable as `presenter_core::bible::normalise_book_key`.)

- [ ] **Step 3: Add a diacritic test case**

Append to the `#[cfg(test)] mod tests` block in `search.rs`:

```rust
    #[test]
    fn strips_slovak_diacritics() {
        assert_eq!(normalise_book_key("Lukáš"), "lukas");
        assert_eq!(normalise_book_key("1. Mojžišova"), "1mojzisova");
    }
```

- [ ] **Step 4: Run unit tests**

```bash
cargo test -p presenter-core bible::search
```

Expected: 2 tests pass (existing `strips_non_alphanumeric_characters` + new `strips_slovak_diacritics`).

### Task 3: Switch `BibleState::filtered_books` to normalised matching

**Files:**
- Modify: `crates/presenter-ui/src/state/bible.rs:109-118`

- [ ] **Step 1: Replace the body of `filtered_books`**

```rust
pub fn filtered_books(&self) -> Vec<BibleBookDto> {
    let filter_raw = self.book_filter.get();
    let filter = presenter_core::bible::normalise_book_key(&filter_raw);
    let all = self.books.get();
    if filter.is_empty() {
        return all;
    }
    all.into_iter()
        .filter(|b| presenter_core::bible::normalise_book_key(&b.book).contains(&filter))
        .collect()
}
```

- [ ] **Step 2: Add a module-public diacritic test**

Append to the `#[cfg(test)] mod tests` block in `state/bible.rs`:

```rust
    #[test]
    fn book_filter_matches_diacritic_insensitively() {
        use crate::api::bible::BibleBookDto;
        // Build a minimal BibleBookDto fixture for Lukáš
        let book = BibleBookDto {
            book: "Lukáš".into(),
            code: "LUK".into(),
            number: 42,
            chapters: vec![],
        };
        // Substring match: ASCII "lukas" matches "Lukáš"
        let filter = presenter_core::bible::normalise_book_key("lukas");
        let key = presenter_core::bible::normalise_book_key(&book.book);
        assert!(key.contains(&filter));
    }
```

(The test exercises the same code path `filtered_books` uses — exhaustive end-to-end via state requires a Leptos runtime, which isn't worth setting up in a `#[cfg(test)]` block.)

- [ ] **Step 3: Run UI crate tests**

```bash
cargo test -p presenter-ui state::bible
```

Expected: existing 6 clamp tests + new 1 pass.

### Task 4: Add `NodeRef`s, `on:keydown` handlers, and mount auto-focus on `BibleLiveTab`

**Files:**
- Modify: `crates/presenter-ui/src/pages/bible.rs` — `BookFilter`, `ReferenceInputs`, and `BibleLiveTab` components

- [ ] **Step 1: Promote the four inputs to use shared `NodeRef`s**

Replace `BookFilter` and `ReferenceInputs` so they accept `NodeRef<leptos::html::Input>` props for focus chaining. Or — simpler given Leptos 0.7 — make the four `NodeRef`s in `BibleLiveTab` and pass into the children as props.

Working approach: keep the existing component structure. Define four `NodeRef`s in `BibleLiveTab`, store them in a small struct shared via `provide_context`, and have each child read them via `use_context`. The `KeyboardEvent` handler in `BookFilter` reads `chapter_ref.get()` and calls `.focus()` on it.

Concrete shared-context type at top of `pages/bible.rs`:

```rust
#[derive(Copy, Clone)]
struct BibleFocusRefs {
    book_filter: NodeRef<leptos::html::Input>,
    chapter: NodeRef<leptos::html::Input>,
    verse_start: NodeRef<leptos::html::Input>,
    verse_end: NodeRef<leptos::html::Input>,
}
```

Provide via `provide_context(BibleFocusRefs { ... })` inside `BibleLiveTab` BEFORE rendering children. Each child does `let refs = use_context::<BibleFocusRefs>().expect("BibleFocusRefs provided");`.

- [ ] **Step 2: Attach `node_ref` to each of the four `<input>`s and add `on:keydown`**

`book-filter` (in `BookFilter`):

```rust
let refs = expect_context::<BibleFocusRefs>();
let bs = use_ctx!(BibleState);
let on_keydown = move |ev: web_sys::KeyboardEvent| {
    if ev.key() == "Enter" {
        ev.prevent_default();
        // Pick the first filtered book — same path as click handler.
        let books = bs.filtered_books();
        if let Some(book) = books.first().cloned() {
            let chapter_count = book.chapters.len() as u16;
            let verse_counts: Vec<u16> = book.chapters.iter().map(|c| c.verse_count).collect();
            let current_chapter = bs.selected_chapter.get_untracked();
            let current_v_start = bs.verse_start.get_untracked();
            let current_v_end = bs.verse_end.get_untracked();
            let clamped = crate::state::bible::clamp_selection(
                chapter_count,
                &verse_counts,
                current_chapter,
                current_v_start,
                current_v_end,
            );
            bs.selected_book.set(Some(SelectedBook {
                book: book.book.clone(),
                code: book.code.clone(),
                number: book.number,
                chapter_count,
                verse_counts,
            }));
            bs.selected_chapter.set(clamped.chapter);
            bs.verse_start.set(clamped.verse_start);
            bs.verse_end.set(clamped.verse_end);
            bs.book_filter.set(String::new());
            if let Some(el) = refs.chapter.get() {
                let _ = el.focus();
            }
        }
    }
};
```

Then on the `<input>`: `node_ref=refs.book_filter on:keydown=on_keydown`.

`chapter-input` (in `ReferenceInputs`):

```rust
let refs = expect_context::<BibleFocusRefs>();
let on_chapter_keydown = move |ev: web_sys::KeyboardEvent| {
    if ev.key() == "Enter" {
        ev.prevent_default();
        // Commit current value via existing on_chapter logic
        if let Some(input) = refs.chapter.get() {
            if let Ok(val) = input.value().parse::<u16>() {
                selected_chapter.set(val.max(1));
                verse_start_signal.set(1);
                verse_end_signal.set(None);
            }
            if let Some(el) = refs.verse_start.get() {
                let _ = el.focus();
                let _ = el.select();
            }
        }
    }
};
```

Same shape for `verse-start` (advance to `verse-end`).

`verse-end` (chain terminator):

```rust
let on_verse_end_keydown = move |ev: web_sys::KeyboardEvent| {
    if ev.key() == "Enter" {
        ev.prevent_default();
        if let Some(input) = refs.verse_end.get() {
            let val_str = input.value();
            if val_str.is_empty() {
                verse_end_signal.set(None);
            } else if let Ok(val) = val_str.parse::<u16>() {
                verse_end_signal.set(Some(val.max(1)));
            }
            let _ = input.blur();
        }
        // Return focus to book-filter; book_filter signal is already empty
        // because book-filter Enter cleared it. Defensive reset:
        bs.book_filter.set(String::new());
        if let Some(el) = refs.book_filter.get() {
            let _ = el.focus();
        }
    }
};
```

- [ ] **Step 3: Mount-time auto-focus**

Inside `BibleLiveTab`, AFTER `provide_context(refs)`:

```rust
let refs = ...;
Effect::new(move |prev: Option<()>| {
    if prev.is_none() {
        if let Some(el) = refs.book_filter.get() {
            let _ = el.focus();
        }
    }
});
```

- [ ] **Step 4: Local lint + check**

```bash
cargo fmt --all --check
cd crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings -W clippy::all
cd - && cargo clippy --workspace --all-targets -- -D warnings -W clippy::all
```

Expected: no warnings.

- [ ] **Step 5: Build WASM**

```bash
cd crates/presenter-ui && trunk build --release
```

Expected: success. (Pre-existing local-build policy allowed per CLAUDE.md.)

### Task 5: GREEN — Run E2E again, expect pass

- [ ] **Step 1: Run dev server locally**

```bash
sudo systemctl restart presenter-dev
sleep 3
curl -sf http://10.77.8.134:8080/healthz
```

- [ ] **Step 2: Run Playwright spec**

```bash
npm run test:playwright -- wasm-bible-keyboard-nav
```

Expected: BOTH scenarios PASS.

- [ ] **Step 3: Run full Playwright suite (sanity, regressions)**

Optional locally; CI runs the full suite in 3 shards anyway. Skip locally.

- [ ] **Step 4: Commit GREEN**

```bash
git add crates/presenter-core/src/bible/search.rs \
        crates/presenter-core/src/bible/mod.rs \
        crates/presenter-ui/src/state/bible.rs \
        crates/presenter-ui/src/pages/bible.rs
git commit -m "$(cat <<'EOF'
feat(bible): keyboard chain + diacritic-insensitive book search

- Promote presenter_core::bible::search::normalise_book_key to pub and
  re-export at presenter_core::bible::normalise_book_key.
- BibleState::filtered_books matches via normalised keys so typing "lukas"
  finds "Lukáš" and "1moj" finds "1. Mojžišova".
- Add Enter-key handlers on book-filter, chapter, verse-start, verse-end
  inputs that step focus through the chain. Verse-end terminator returns
  focus to book-filter and clears the filter.
- Auto-focus book-filter on /ui/operator/bible mount via NodeRef.

Closes #257.
EOF
)"
```

### Task 6: Controller-handled push, monitor, PR, wait for merge

- [ ] **Step 1: Push dev**

```bash
git push origin dev
```

- [ ] **Step 2: Monitor pipeline to terminal green per `ci-monitoring.md`** — single background `sleep 300 && gh run view <id>`.

- [ ] **Step 3: Open PR dev → main** with body listing `Closes #257`.

- [ ] **Step 4: Verify mergeable: true AND mergeable_state: clean.**

- [ ] **Step 5: Send completion report.** Wait for explicit user merge instruction.

- [ ] **Step 6: Post-deploy verification on dev (10.77.8.134:8080)** — open `/ui/operator/bible` in Playwright, type `lukas`, verify Lukáš shown; press Enter chain manually; assert console clean.
