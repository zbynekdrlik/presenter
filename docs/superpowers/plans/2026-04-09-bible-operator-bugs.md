# Bible Operator Page Bug Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix 6 bugs on the Bible operator page: search stuck, translation not persisted, broom not clearing Resolume bible, verse text truncated, preview faded, edit textareas too small.

**Architecture:** All fixes are in the WASM UI crate (`presenter-ui`). Bugs 1-3 are Rust/Leptos logic changes in component files. Bugs 4-6 are CSS-only fixes. E2E tests validate all fixes in Playwright.

**Tech Stack:** Rust (Leptos WASM), CSS, Playwright E2E

**Spec:** `docs/superpowers/specs/2026-04-09-bible-operator-bugs-design.md`

---

## Context

Issue #219: Six bugs on the Bible operator page (`/ui/operator/bible`).

**Key existing code:**
- `crates/presenter-ui/src/pages/bible.rs` — `BiblePage`, `BookFilter`, `BookList`, `TranslationSelectors`, `BibleSettingsTab` components
- `crates/presenter-ui/src/state/bible.rs` — `BibleState` with signals: `book_filter`, `selected_book`, `selected_translation`, `secondary_translation`, `character_limit`
- `crates/presenter-ui/src/components/stage_preview.rs` — broom button `on_clear` handler (line 49-61)
- `crates/presenter-ui/src/pages/bible_controls.rs` — `ClearBroadcastButton` (defined but never mounted)
- `crates/presenter-ui/styles/bible.css` — Bible-specific CSS overrides
- `crates/presenter-ui/styles/operator.css` — base operator CSS (slide text, preview opacity, editor textareas)
- `crates/presenter-ui/src/api/bible.rs` — `clear_broadcast()`, `update_preferences()`, `get_preferences()`
- `tests/e2e/wasm-bible.spec.ts` — 60 existing E2E tests with `navigateToBible()`, `biblePanel()`, `hasBibleData()` helpers

---

## File Structure

### Modified Files
| File | Change |
|------|--------|
| `crates/presenter-ui/src/pages/bible.rs` | Bug 1: auto-deselect book when filter typed. Bug 2: auto-save translation preferences |
| `crates/presenter-ui/src/components/stage_preview.rs` | Bug 3: broom button also clears bible broadcast |
| `crates/presenter-ui/styles/bible.css` | Bug 4: verse text wrap override. Bug 6: remove fixed min-height on textareas |
| `crates/presenter-ui/styles/operator.css` | Bug 5: increase inactive preview opacity |
| `tests/e2e/wasm-bible.spec.ts` | E2E tests for all 6 bug fixes |

---

## Task 0: Version Bump

- [ ] **Step 1: Bump version from 0.4.9 to 0.4.10**

In `Cargo.toml`, change the workspace version:

```toml
version = "0.4.10"
```

- [ ] **Step 2: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "chore: bump version to 0.4.10"
```

---

## Task 1: Fix Book Search Stuck on 2nd Attempt

**Files:**
- Modify: `crates/presenter-ui/src/pages/bible.rs:284-310` (BookFilter component)

- [ ] **Step 1: Add auto-deselect effect to BookFilter**

In `crates/presenter-ui/src/pages/bible.rs`, replace the `BookFilter` component (lines 284-310) with:

```rust
#[component]
fn BookFilter() -> impl IntoView {
    let bs = use_ctx!(BibleState);
    let book_filter = bs.book_filter;
    let selected_book = bs.selected_book;

    let on_input = move |ev: web_sys::Event| {
        let target = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok());
        if let Some(input) = target {
            let val = input.value();
            book_filter.set(val.clone());
            // Auto-deselect current book when user starts typing a new search
            if !val.is_empty() && selected_book.get_untracked().is_some() {
                selected_book.set(None);
            }
        }
    };

    view! {
        <label class="operator__field">
            <span>"Find book"</span>
            <input
                type="search"
                data-role="book-filter"
                placeholder="Start typing\u{2026}"
                prop:value=move || book_filter.get()
                on:input=on_input
            />
        </label>
    }
}
```

The key addition: when the user types a non-empty value AND a book is already selected, clear `selected_book` so `BookList` shows the filtered list instead of the collapsed single-book view.

- [ ] **Step 2: Build WASM to verify compilation**

```bash
cargo build -p presenter-ui --target wasm32-unknown-unknown
```

Expected: Compiles without errors.

- [ ] **Step 3: Commit**

```bash
git add crates/presenter-ui/src/pages/bible.rs
git commit -m "fix(bible): auto-deselect book when filter is typed (#219)

When a book is selected and the user types in the filter input,
automatically clear the selected book so the full filtered book
list renders. Previously the collapsed single-book view blocked
searching for a different book."
```

---

## Task 2: Auto-Save Translation Preferences

**Files:**
- Modify: `crates/presenter-ui/src/pages/bible.rs:14-46` (BiblePage mount block)

- [ ] **Step 1: Add a `translations_loaded` flag signal and auto-save effect**

In `crates/presenter-ui/src/pages/bible.rs`, in the `BiblePage` component, add a `translations_loaded` signal and modify the mount block. Replace lines 14-46 with:

```rust
#[component]
pub fn BiblePage() -> impl IntoView {
    let ctx = use_ctx!(AppContext);
    let bs = BibleState::new();
    provide_context(bs.clone());

    // Flag: set to true after initial preferences + translations are loaded.
    // Prevents the auto-save effect from firing during initial hydration.
    let translations_loaded = RwSignal::new(false);

    // Load translations + preferences on mount
    {
        let translations = bs.translations;
        let selected_translation = bs.selected_translation;
        let secondary_translation = bs.secondary_translation;
        let character_limit = bs.character_limit;
        let translations_loaded = translations_loaded;
        leptos::task::spawn_local(async move {
            // Load preferences first to set saved translation choices
            if let Ok(prefs) = bible::get_preferences().await {
                if let Some(ref main) = prefs.main_translation {
                    selected_translation.set(Some(main.clone()));
                }
                if let Some(ref sec) = prefs.secondary_translation {
                    secondary_translation.set(Some(sec.clone()));
                }
                character_limit.set(prefs.character_limit);
            }
            if let Ok(trans) = bible::list_translations().await {
                // Set default if preferences didn't set one
                if selected_translation.get_untracked().is_none() {
                    if let Some(first) = trans.first() {
                        selected_translation.set(Some(first.code.clone()));
                    }
                }
                translations.set(trans);
            }
            // Mark loaded AFTER all initial sets are done
            translations_loaded.set(true);
        });
    }

    // Auto-save translation preferences when the user changes them
    {
        let selected_translation = bs.selected_translation;
        let secondary_translation = bs.secondary_translation;
        let character_limit = bs.character_limit;
        Effect::new(move || {
            let main = selected_translation.get();
            let sec = secondary_translation.get();
            // Skip if translations haven't loaded yet (initial hydration)
            if !translations_loaded.get_untracked() {
                return;
            }
            let limit = character_limit.get_untracked();
            leptos::task::spawn_local(async move {
                let draft = presenter_core::BiblePreferencesDraft {
                    main_translation: main,
                    secondary_translation: sec,
                    character_limit: Some(limit),
                };
                let _ = bible::update_preferences(&draft).await;
            });
        });
    }
```

Note: The `translations_loaded` flag ensures the auto-save `Effect` doesn't fire during the initial mount when signals are being set from the loaded preferences. It only fires on subsequent user-initiated changes.

- [ ] **Step 2: Build WASM to verify compilation**

```bash
cargo build -p presenter-ui --target wasm32-unknown-unknown
```

Expected: Compiles without errors.

- [ ] **Step 3: Commit**

```bash
git add crates/presenter-ui/src/pages/bible.rs
git commit -m "fix(bible): auto-save translation preferences on change (#219)

When the user changes the main or secondary translation dropdown
in the Live tab, automatically persist the preference to the server.
Previously, preferences were only saved via the Settings tab Save
button, so translation choices were lost on refresh."
```

---

## Task 3: Make Broom Button Clear Bible Broadcast

**Files:**
- Modify: `crates/presenter-ui/src/components/stage_preview.rs:49-61`

- [ ] **Step 1: Add bible clear to broom button handler**

In `crates/presenter-ui/src/components/stage_preview.rs`, replace the `on_clear` closure (lines 49-61) with:

```rust
    let on_clear = {
        let stage_snapshot = ctx.stage_snapshot;
        let active_bible_broadcast = ctx.active_bible_broadcast;
        let toast_message = ctx.toast_message;
        let toast_variant = ctx.toast_variant;
        move |_| {
            leptos::task::spawn_local(async move {
                let _ = crate::api::stage::clear().await;
                let _ = crate::api::bible::clear_broadcast().await;
                stage_snapshot.set(None);
                active_bible_broadcast.set(None);
                toast_variant.set("info".to_string());
                toast_message.set(Some("Slide outputs cleared".to_string()));
            });
        }
    };
```

The key additions: call `crate::api::bible::clear_broadcast().await` and set `active_bible_broadcast` to `None`.

- [ ] **Step 2: Build WASM to verify compilation**

```bash
cargo build -p presenter-ui --target wasm32-unknown-unknown
```

Expected: Compiles without errors.

- [ ] **Step 3: Commit**

```bash
git add crates/presenter-ui/src/components/stage_preview.rs
git commit -m "fix(bible): broom button now clears bible broadcast too (#219)

The broom button previously only cleared worship stage state.
Now it also calls POST /bible/clear to clear bible text from
Resolume outputs."
```

---

## Task 4: Fix CSS — Verse Text Truncation, Preview Opacity, Editor Size

**Files:**
- Modify: `crates/presenter-ui/styles/bible.css:547-560`
- Modify: `crates/presenter-ui/styles/operator.css:829-831`

- [ ] **Step 1: Fix verse text truncation (Bug 4) — add Bible slide text override**

In `crates/presenter-ui/styles/bible.css`, add after line 560 (after the textarea rule closing brace):

```css
/* Bug #219: Bible verse text was truncated by text-overflow: ellipsis
   inherited from .operator__slide-text. Override to wrap naturally. */
.operator__slide-card--bible .operator__slide-text {
  white-space: pre-wrap;
  text-overflow: clip;
  overflow-x: visible;
}
```

- [ ] **Step 2: Fix edit mode textarea size (Bug 6) — remove fixed min-height**

In `crates/presenter-ui/styles/bible.css`, in the rule at lines 547-560, remove the `min-height: 2.5rem;` line (line 559). The rule becomes:

```css
.operator__slide-card--bible .operator__slide-editor--bible textarea,
.operator__slide-card--bible .operator__slide-editor--bible input[type="text"] {
  width: 100%;
  box-sizing: border-box;
  padding: 0.4rem 0.5rem;
  border: 1px solid rgba(148, 163, 184, 0.35);
  border-radius: 6px;
  font-size: 0.85rem;
  font-family: inherit;
  background: #0f172a;
  color: #f8fafc;
  resize: vertical;
}
```

Without the fixed `min-height: 2.5rem`, the dynamic sizing from `operator.css` line 1336 (`.operator--bible .operator__slide-editor textarea` with `--bible-textarea-lines: 4`) takes effect, giving textareas 4 lines of height.

- [ ] **Step 3: Fix preview opacity (Bug 5)**

In `crates/presenter-ui/styles/operator.css`, replace line 830:

```css
/* Before: */
  opacity: 0.6;

/* After: */
  opacity: 0.85;
```

- [ ] **Step 4: Build WASM to verify CSS compiles into the bundle**

```bash
cargo build -p presenter-ui --target wasm32-unknown-unknown
```

Expected: Compiles without errors.

- [ ] **Step 5: Commit**

```bash
git add crates/presenter-ui/styles/bible.css crates/presenter-ui/styles/operator.css
git commit -m "fix(bible): fix verse truncation, preview opacity, editor size (#219)

- Bible verse text now wraps (pre-wrap) instead of being truncated
  with ellipsis dots (overrides operator.css text-overflow: ellipsis)
- Inactive preview opacity increased from 0.6 to 0.85 for legibility
- Removed fixed min-height: 2.5rem on Bible textareas so dynamic
  4-line sizing from operator.css takes effect"
```

---

## Task 5: E2E Tests for All 6 Bug Fixes

**Files:**
- Modify: `tests/e2e/wasm-bible.spec.ts`

- [ ] **Step 1: Add E2E test for Bug 1 — book search re-search**

In `tests/e2e/wasm-bible.spec.ts`, add inside the `test.describe("WASM Operator Bible Tests", ...)` block, before the closing `});`:

```typescript
  // -----------------------------------------------------------------------
  // Bug fixes (#219)
  // -----------------------------------------------------------------------

  test("book filter re-search after book selection (#219 bug 1)", async ({
    page,
  }) => {
    const consoleMessages: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
      }
    });

    await navigateToBible(page);
    if (!(await hasBibleData(page))) {
      test.skip();
      return;
    }

    const panel = biblePanel(page);
    const bookList = panel.locator('[data-role="book-list"]');
    const filterInput = panel.locator('[data-role="book-filter"]');

    // Select the first book
    const firstBook = bookList.locator('[data-role="book-item"]').first();
    await expect(firstBook).toBeVisible({ timeout: 10_000 });
    await firstBook.click();

    // Book should now be selected (collapsed view with "Change")
    await expect(
      bookList.locator('[data-role="book-item"][data-active="true"]'),
    ).toHaveCount(1);

    // Type a new search term — the selected book should auto-deselect
    await filterInput.fill("Psalm");
    await page.waitForTimeout(300);

    // Should see filtered book list (not the collapsed single-book view)
    const items = bookList.locator('[data-role="book-item"]');
    const count = await items.count();
    expect(count).toBeGreaterThan(0);

    // At least one item should contain "Psalm" in its label
    const labels = await items.allTextContents();
    const hasPsalm = labels.some((l) =>
      l.toLowerCase().includes("psalm"),
    );
    expect(hasPsalm).toBe(true);

    // Clear filter — should show all books
    await filterInput.fill("");
    await page.waitForTimeout(300);
    const allCount = await bookList
      .locator('[data-role="book-item"]')
      .count();
    expect(allCount).toBeGreaterThan(10);

    expect(consoleMessages).toEqual([]);
  });
```

- [ ] **Step 2: Add E2E test for Bug 2 — translation persistence**

```typescript
  test("translation selection persists after reload (#219 bug 2)", async ({
    page,
  }) => {
    const consoleMessages: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
      }
    });

    await navigateToBible(page);
    if (!(await hasBibleData(page))) {
      test.skip();
      return;
    }

    const panel = biblePanel(page);
    const mainSelect = panel.locator('[data-role="main-translation"]');
    await expect(mainSelect).toBeVisible({ timeout: 10_000 });

    // Get available translations
    const options = await mainSelect.locator("option").allTextContents();
    if (options.length < 2) {
      test.skip();
      return;
    }

    // Select the second translation
    const secondValue = await mainSelect
      .locator("option")
      .nth(1)
      .getAttribute("value");
    await mainSelect.selectOption(secondValue!);

    // Wait for auto-save to complete
    await page.waitForTimeout(1000);

    // Reload and navigate back to bible
    await navigateToBible(page);
    await page.waitForTimeout(2000);

    // Verify the translation is still selected
    const currentValue = await panel
      .locator('[data-role="main-translation"]')
      .inputValue();
    expect(currentValue).toBe(secondValue);

    expect(consoleMessages).toEqual([]);
  });
```

- [ ] **Step 3: Add E2E test for Bug 3 — broom clears bible broadcast**

```typescript
  test("broom button clears bible broadcast (#219 bug 3)", async ({
    page,
  }) => {
    const consoleMessages: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
      }
    });

    await navigateToBible(page);
    if (!(await hasBibleData(page))) {
      test.skip();
      return;
    }

    const panel = biblePanel(page);

    // Select a book and load verses to create a broadcast
    const firstBook = panel
      .locator('[data-role="book-list"] [data-role="book-item"]')
      .first();
    await expect(firstBook).toBeVisible({ timeout: 10_000 });
    await firstBook.click();

    // Wait for chapter/verse inputs and load
    const loadButton = panel.locator('[data-role="load-verses"]');
    if ((await loadButton.count()) > 0) {
      await loadButton.click();
      await page.waitForTimeout(2000);
    }

    // Trigger the first slide to broadcast it
    const slideCard = page
      .locator('[data-role="slide-card"]')
      .first();
    if ((await slideCard.count()) > 0) {
      await slideCard.click();
      await page.waitForTimeout(1000);
    }

    // Click broom button
    const broomButton = page.locator('[data-role="clear-slide"]');
    await expect(broomButton).toBeVisible({ timeout: 5_000 });
    await broomButton.click();

    // Wait for toast confirmation
    await page.waitForTimeout(500);

    // Verify bible broadcast is cleared via API
    const response = await page.evaluate(async (url) => {
      const resp = await fetch(`${url}/bible/broadcast`);
      return resp.json();
    }, baseURL);

    // broadcast should be null/empty after clear
    expect(response).toBeNull();

    expect(consoleMessages).toEqual([]);
  });
```

- [ ] **Step 4: Add E2E test for Bug 4 — verse text not truncated**

```typescript
  test("bible verse text is not truncated with ellipsis (#219 bug 4)", async ({
    page,
  }) => {
    const consoleMessages: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
      }
    });

    await navigateToBible(page);
    if (!(await hasBibleData(page))) {
      test.skip();
      return;
    }

    const panel = biblePanel(page);

    // Select a book and load verses
    const firstBook = panel
      .locator('[data-role="book-list"] [data-role="book-item"]')
      .first();
    await expect(firstBook).toBeVisible({ timeout: 10_000 });
    await firstBook.click();
    const loadButton = panel.locator('[data-role="load-verses"]');
    if ((await loadButton.count()) > 0) {
      await loadButton.click();
      await page.waitForTimeout(2000);
    }

    // Check that slide text uses pre-wrap, not ellipsis
    const slideText = page.locator(".operator__slide-card--bible .operator__slide-text").first();
    if ((await slideText.count()) > 0) {
      const styles = await slideText.evaluate((el) => {
        const cs = window.getComputedStyle(el);
        return {
          whiteSpace: cs.whiteSpace,
          textOverflow: cs.textOverflow,
        };
      });
      expect(styles.whiteSpace).toBe("pre-wrap");
      expect(styles.textOverflow).not.toBe("ellipsis");
    }

    expect(consoleMessages).toEqual([]);
  });
```

- [ ] **Step 5: Add E2E test for Bug 5 — preview not overly faded**

```typescript
  test("stage preview is legible when inactive (#219 bug 5)", async ({
    page,
  }) => {
    const consoleMessages: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
      }
    });

    await navigateToBible(page);

    const preview = page.locator(".operator__stage-preview");
    if ((await preview.count()) > 0) {
      const opacity = await preview.evaluate((el) => {
        return parseFloat(window.getComputedStyle(el).opacity);
      });
      // Should be at least 0.8 (we set it to 0.85)
      expect(opacity).toBeGreaterThanOrEqual(0.8);
    }

    expect(consoleMessages).toEqual([]);
  });
```

- [ ] **Step 6: Add E2E test for Bug 6 — edit mode textarea size**

```typescript
  test("bible edit mode textareas are not too small (#219 bug 6)", async ({
    page,
  }) => {
    const consoleMessages: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
      }
    });

    await navigateToBible(page);
    if (!(await hasBibleData(page))) {
      test.skip();
      return;
    }

    const panel = biblePanel(page);

    // Load some verses
    const firstBook = panel
      .locator('[data-role="book-list"] [data-role="book-item"]')
      .first();
    await expect(firstBook).toBeVisible({ timeout: 10_000 });
    await firstBook.click();
    const loadButton = panel.locator('[data-role="load-verses"]');
    if ((await loadButton.count()) > 0) {
      await loadButton.click();
      await page.waitForTimeout(2000);
    }

    // Switch to edit mode
    const editButton = page.locator(
      '[data-role="mode-toggle"][data-mode="edit"]',
    );
    await editButton.click();
    await page.waitForTimeout(500);

    // Check textarea min-height is at least 4rem (roughly 64px at default font size)
    const textarea = page
      .locator(".operator--bible .operator__slide-editor textarea")
      .first();
    if ((await textarea.count()) > 0) {
      const height = await textarea.evaluate((el) => {
        return el.getBoundingClientRect().height;
      });
      // 4 lines of text at ~1.35 line-height should be ~50px minimum
      expect(height).toBeGreaterThan(45);
    }

    // Switch back to live mode
    const liveButton = page.locator(
      '[data-role="mode-toggle"][data-mode="live"]',
    );
    await liveButton.click();

    expect(consoleMessages).toEqual([]);
  });
```

- [ ] **Step 7: Commit**

```bash
git add tests/e2e/wasm-bible.spec.ts
git commit -m "test(e2e): add tests for bible operator bug fixes (#219)

6 new E2E tests covering: book filter re-search after selection,
translation persistence after reload, broom button clearing bible
broadcast, verse text not truncated, preview opacity legible,
edit mode textarea sizing."
```

---

## Task 6: Local Checks, Push, Monitor CI

- [ ] **Step 1: Run cargo fmt**

```bash
cargo fmt --all --check
```

Fix if needed: `cargo fmt --all`

- [ ] **Step 2: Run clippy locally**

```bash
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all
```

Fix any issues.

- [ ] **Step 3: Run Rust tests locally**

```bash
cargo test -p presenter-server --lib
cargo test -p presenter-core
```

- [ ] **Step 4: Push and monitor CI**

```bash
git push origin dev
gh run list --branch dev --limit 3
```

Monitor until ALL jobs reach terminal state. If any fail: `gh run view <run-id> --log-failed`, fix ALL issues in ONE commit, push again.

---

## Verification Summary

| Bug | How to verify |
|-----|---------------|
| 1. Search stuck | Select book → type new name in filter → book list shows matching books |
| 2. Translation not saved | Change main translation → reload → same translation selected |
| 3. Broom doesn't clear Resolume | Broadcast bible slide → click broom → GET /bible/broadcast returns null |
| 4. Verse truncated | Bible slide text uses `white-space: pre-wrap`, no ellipsis |
| 5. Preview faded | Inactive preview opacity ≥ 0.8 |
| 6. Edit textareas small | Bible edit mode textarea height > 45px (4+ lines) |
