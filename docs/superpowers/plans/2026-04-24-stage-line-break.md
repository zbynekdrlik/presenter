# Stage Single-Line Auto-Break Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Auto-break single-line worship slides longer than 26 characters at the rightmost space (tail-bias) so autofit can render them at a larger font, and fix the operator UI overflow warning to count characters instead of bytes.

**Architecture:** A pure-Rust text helper `break_if_long` lives in `crates/presenter-ui/src/utils/text.rs`. The `WorshipSnv` component pipes both `current_text` and `next_text` through it before passing to the DOM. Separately, `slide_list_utils.rs` switches `line.len()` (bytes) to `line.chars().count()` (characters) in two call sites. Both changes are unit tested; a Playwright E2E verifies the stage splits the line and the operator UI no longer flags a 28-char Slovak line.

**Tech Stack:** Rust (stable), Leptos WASM, Playwright TypeScript, Trunk build.

**Spec:** `docs/superpowers/specs/2026-04-24-stage-line-break-design.md`

---

## Context

The worship-snv stage layout renders single lines verbatim via `white-space: pre` and shrinks the font via a binary-search autofit to fit the whole line horizontally. A 30-character Slovak line forces the font low, wasting vertical space. Splitting it at the rightmost space ≤ 26 chars preserves musical phrasing (tail-break only moves the last word or two) while giving autofit a much larger ceiling. The `api` layout falls through to the same `WorshipSnv` component in `stage.rs:167-169`, so one code change covers both layouts.

The operator UI shows a red warning when any line exceeds the configured `--operator-line-limit-ch` (default 32). The check uses `str::len()` which returns bytes. Slovak diacritics (`á č š ž`) are 2 bytes in UTF-8, so `"Nad všetkých vyvyšený bude"` (26 visible chars) counts as 30 bytes — still below the 32-byte limit, but a 28-visible-char diacritic line may cross it. The fix is a one-character swap: `.chars().count()`.

**Key existing code:**
- `crates/presenter-ui/src/components/stage/worship_snv.rs` — `current_text` and `next_text` closures return `String`.
- `crates/presenter-ui/src/components/slide_list_utils.rs:8,26` — the two lines using `line.len() as u32`.
- `crates/presenter-ui/src/utils/color.rs` — example of a `#[cfg(test)]` module inside a utility file in this crate.
- `crates/presenter-ui/src/utils/mod.rs` — registers utility submodules.
- `tests/e2e/stage-layout.spec.ts` — `openStageDisplay`, `triggerSlide`, `TEST_SLIDES`.
- `Cargo.toml:15` — workspace version (currently `0.4.31`; bump to `0.4.32`).

---

## File Structure

### New Files
| File | Purpose |
|------|---------|
| `crates/presenter-ui/src/utils/text.rs` | Pure `break_if_long(text, threshold)` helper + unit tests. |

### Modified Files
| File | Change |
|------|--------|
| `Cargo.toml:15` | Version bump `0.4.31` → `0.4.32`. |
| `crates/presenter-ui/src/utils/mod.rs` | Register `pub mod text;`. |
| `crates/presenter-ui/src/components/stage/worship_snv.rs` | Wrap `current_text` and `next_text` closure outputs with `break_if_long`. |
| `crates/presenter-ui/src/components/slide_list_utils.rs:8,26` | Swap `line.len()` → `line.chars().count()`; add `#[cfg(test)]` module. |
| `tests/e2e/stage-layout.spec.ts` | Add two slides to `TEST_SLIDES` (positive + negative) and two new tests. |
| `tests/e2e/stage-layout.spec.ts` | Add one test for operator UI diacritic warning. (Optional: split into `tests/e2e/slide-warning.spec.ts` if operator test proves complex — see Task 6.) |

---

## Task 1: Version bump

**Files:**
- Modify: `Cargo.toml:15`

- [ ] **Step 1: Bump version**

Edit `Cargo.toml` line 15:

```toml
# Before
version = "0.4.31"

# After
version = "0.4.32"
```

- [ ] **Step 2: Update lockfile**

Run: `cargo check --workspace --quiet`
Expected: `Cargo.lock` updates the `presenter` package version to `0.4.32`. No compile errors.

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "chore: bump version to 0.4.32"
```

---

## Task 2: `break_if_long` helper with unit tests (TDD)

**Files:**
- Create: `crates/presenter-ui/src/utils/text.rs`
- Modify: `crates/presenter-ui/src/utils/mod.rs`

- [ ] **Step 1: Create the file with failing tests first**

Create `crates/presenter-ui/src/utils/text.rs`:

```rust
/// Auto-break a single-line string longer than `threshold` characters at the
/// rightmost space that keeps line 1 within the threshold (tail-break).
///
/// Returns the input unchanged when:
/// - the text already contains a newline (respect author-chosen breaks),
/// - `chars().count()` is `<= threshold`,
/// - no space produces a valid split (e.g. pathological single-word line).
///
/// Uses Unicode character counts, not bytes — safe for Slovak/Czech diacritics.
pub fn break_if_long(text: &str, threshold: usize) -> String {
    if text.contains('\n') {
        return text.to_string();
    }
    if text.chars().count() <= threshold {
        return text.to_string();
    }

    // Walk char indices; remember the rightmost space where the prefix char
    // count is <= threshold.
    let mut best_split: Option<usize> = None;
    let mut char_count: usize = 0;
    for (byte_idx, ch) in text.char_indices() {
        if ch == ' ' && char_count <= threshold {
            best_split = Some(byte_idx);
        }
        char_count += 1;
    }

    match best_split {
        Some(i) => {
            let (head, tail) = text.split_at(i);
            // tail starts with the space; skip it.
            let tail_trimmed = tail.strip_prefix(' ').unwrap_or(tail);
            format!("{head}\n{tail_trimmed}")
        }
        None => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_ascii_line_unchanged() {
        assert_eq!(break_if_long("Short line", 26), "Short line");
    }

    #[test]
    fn exactly_threshold_unchanged() {
        // 26 chars
        let s = "abcdefghijklmnopqrstuvwxyz";
        assert_eq!(s.chars().count(), 26);
        assert_eq!(break_if_long(s, 26), s);
    }

    #[test]
    fn breaks_at_rightmost_space_under_threshold() {
        // 30 chars, spaces at positions 3, 12, 21, 26
        let input = "Nad všetkých vyvyšený bude Pán";
        assert_eq!(input.chars().count(), 30);
        // Rightmost space where prefix <= 26 chars is the one after "bude".
        // Prefix "Nad všetkých vyvyšený bude" is 26 chars; the next space
        // (after "Pán") would be beyond the end.
        let out = break_if_long(input, 26);
        assert_eq!(out, "Nad všetkých vyvyšený bude\nPán");
    }

    #[test]
    fn multi_line_input_unchanged() {
        let input = "Line one\nLine two that is long enough";
        assert_eq!(break_if_long(input, 26), input);
    }

    #[test]
    fn single_word_longer_than_threshold_unchanged() {
        let input = "Supercalifragilisticexpialidocious";
        assert_eq!(input.chars().count(), 34);
        assert_eq!(break_if_long(input, 26), input);
    }

    #[test]
    fn diacritics_counted_as_one_char_each() {
        // 28 Slovak chars with many diacritics (bytes are significantly more).
        let input = "Pán je môj pastier som v bezp";
        assert_eq!(input.chars().count(), 29);
        // Rightmost space where prefix <= 26 chars is after "som" (prefix = "Pán je môj pastier som" = 22 chars).
        // Next space after "v" gives prefix "Pán je môj pastier som v" = 24 chars — still <= 26, so that's the rightmost.
        let out = break_if_long(input, 26);
        assert_eq!(out, "Pán je môj pastier som v\nbezp");
    }

    #[test]
    fn empty_string_unchanged() {
        assert_eq!(break_if_long("", 26), "");
    }

    #[test]
    fn only_whitespace_unchanged() {
        assert_eq!(break_if_long("   ", 26), "   ");
    }
}
```

- [ ] **Step 2: Register the module**

Edit `crates/presenter-ui/src/utils/mod.rs`:

```rust
// Before
pub mod autofit;
pub mod color;
pub mod keyboard;
pub mod test_helpers;
pub mod window;

// After
pub mod autofit;
pub mod color;
pub mod keyboard;
pub mod test_helpers;
pub mod text;
pub mod window;
```

- [ ] **Step 3: Run the new tests**

Run: `cargo test -p presenter-ui --lib utils::text`
Expected: 8 tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/presenter-ui/src/utils/text.rs crates/presenter-ui/src/utils/mod.rs
git commit -m "feat(stage): add break_if_long helper for single-line slides"
```

---

## Task 3: Byte→char fix in operator overflow warning (TDD)

**Files:**
- Modify: `crates/presenter-ui/src/components/slide_list_utils.rs`

- [ ] **Step 1: Write the failing tests first**

Append to `crates/presenter-ui/src/components/slide_list_utils.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_line_over_limit_warns() {
        // 33-char ASCII line with limit 32
        let long_line = "abcdefghijklmnopqrstuvwxyz0123456";
        assert_eq!(long_line.chars().count(), 33);
        assert!(field_has_warning(long_line, 32));
    }

    #[test]
    fn ascii_line_at_limit_does_not_warn() {
        let line = "abcdefghijklmnopqrstuvwxyz012345";
        assert_eq!(line.chars().count(), 32);
        assert!(!field_has_warning(line, 32));
    }

    #[test]
    fn diacritic_line_within_char_limit_does_not_warn() {
        // 28 visible chars, but many 2-byte diacritics → > 32 bytes with old logic
        let line = "Nad všetkých vyvyšený bude P";
        assert_eq!(line.chars().count(), 28);
        assert!(line.len() > 32, "sanity: byte length exceeds 32");
        assert!(!field_has_warning(line, 32), "28 visible chars must not warn at limit 32");
    }

    #[test]
    fn diacritic_line_over_char_limit_warns() {
        // 33 visible chars of diacritics
        let line = "Požehnaný kto prichádza v mene P";
        assert_eq!(line.chars().count(), 32);
        assert!(!field_has_warning(line, 32));

        let longer = format!("{line}a");
        assert_eq!(longer.chars().count(), 33);
        assert!(field_has_warning(&longer, 32));
    }

    #[test]
    fn format_multiline_does_not_wrap_diacritic_line_under_limit() {
        let line = "Nad všetkých vyvyšený bude P"; // 28 chars
        let html = format_multiline(line, 32);
        assert!(
            !html.contains("operator__slide-overflow"),
            "28-char diacritic line should not be wrapped, got: {html}"
        );
    }

    #[test]
    fn format_multiline_wraps_ascii_line_over_limit() {
        let line = "abcdefghijklmnopqrstuvwxyz0123456"; // 33 chars
        let html = format_multiline(line, 32);
        assert!(
            html.contains("operator__slide-overflow"),
            "33-char ASCII line should be wrapped, got: {html}"
        );
    }

    #[test]
    fn slide_has_any_warning_considers_all_fields() {
        let ok = "short";
        let long = "abcdefghijklmnopqrstuvwxyz0123456";
        assert!(!slide_has_any_warning(ok, ok, ok, 32));
        assert!(slide_has_any_warning(long, ok, ok, 32));
        assert!(slide_has_any_warning(ok, long, ok, 32));
        assert!(slide_has_any_warning(ok, ok, long, 32));
    }

    #[test]
    fn zero_limit_disables_warnings() {
        let long = "abcdefghijklmnopqrstuvwxyz0123456";
        assert!(!field_has_warning(long, 0));
    }
}
```

- [ ] **Step 2: Run to confirm the diacritic tests fail (RED)**

Run: `cargo test -p presenter-ui --lib components::slide_list_utils`
Expected: `diacritic_line_within_char_limit_does_not_warn` and `format_multiline_does_not_wrap_diacritic_line_under_limit` FAIL because the current code counts bytes.

- [ ] **Step 3: Apply the byte→char fix**

Edit `crates/presenter-ui/src/components/slide_list_utils.rs:8`:

```rust
// Before
if limit > 0 && line.len() as u32 > limit {

// After
if limit > 0 && line.chars().count() as u32 > limit {
```

Edit `crates/presenter-ui/src/components/slide_list_utils.rs:26`:

```rust
// Before
pub(super) fn field_has_warning(text: &str, limit: u32) -> bool {
    limit > 0 && text.lines().any(|line| line.len() as u32 > limit)
}

// After
pub(super) fn field_has_warning(text: &str, limit: u32) -> bool {
    limit > 0 && text.lines().any(|line| line.chars().count() as u32 > limit)
}
```

- [ ] **Step 4: Run tests to confirm GREEN**

Run: `cargo test -p presenter-ui --lib components::slide_list_utils`
Expected: all 7 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/presenter-ui/src/components/slide_list_utils.rs
git commit -m "fix(operator): count characters not bytes for slide overflow warning"
```

---

## Task 4: Wire `break_if_long` into `WorshipSnv`

**Files:**
- Modify: `crates/presenter-ui/src/components/stage/worship_snv.rs`

- [ ] **Step 1: Import `break_if_long` and wrap both text closures**

Edit `crates/presenter-ui/src/components/stage/worship_snv.rs`:

Add to imports after existing `use crate::utils::color::group_pill_style;`:

```rust
use crate::utils::text::break_if_long;
```

Add a module-level constant after the existing font constants (around line 13):

```rust
const STAGE_SLIDE_BREAK_THRESHOLD: usize = 26;
```

Replace the `current_text` closure (lines 29-42):

```rust
    let current_text = move || {
        let raw = ctx
            .snapshot
            .get()
            .and_then(|s| {
                s.current.map(|slide| {
                    if !slide.stage.is_empty() {
                        slide.stage
                    } else {
                        slide.main
                    }
                })
            })
            .unwrap_or_default();
        break_if_long(&raw, STAGE_SLIDE_BREAK_THRESHOLD)
    };
```

Replace the `next_text` closure (lines 44-57):

```rust
    let next_text = move || {
        let raw = ctx
            .snapshot
            .get()
            .and_then(|s| {
                s.next.map(|slide| {
                    if !slide.stage.is_empty() {
                        slide.stage
                    } else {
                        slide.main
                    }
                })
            })
            .unwrap_or_default();
        break_if_long(&raw, STAGE_SLIDE_BREAK_THRESHOLD)
    };
```

- [ ] **Step 2: Compile to confirm no type errors**

Run: `cargo check -p presenter-ui --target wasm32-unknown-unknown`
Expected: no errors. (If `wasm32-unknown-unknown` is not installed locally, `cargo check -p presenter-ui` compiles the non-WASM target and is acceptable for type-check.)

- [ ] **Step 3: Rebuild the WASM bundle embedded in the server**

Run: `cd crates/presenter-ui && trunk build --release && cd -`
Expected: `crates/presenter-ui/dist/` updated; `presenter-ui*.js` and `presenter-ui*.wasm` regenerated.

- [ ] **Step 4: Rebuild the server to pick up the new embedded dist**

Run: `cargo build --release -p presenter-server`
Expected: `target/release/presenter-server` binary rebuilt.

- [ ] **Step 5: Commit**

```bash
git add crates/presenter-ui/src/components/stage/worship_snv.rs
git commit -m "feat(stage): auto-break single-line slides over 26 chars"
```

---

## Task 5: Playwright E2E — stage auto-break

**Files:**
- Modify: `tests/e2e/stage-layout.spec.ts`

- [ ] **Step 1: Add two slides to `TEST_SLIDES`**

Edit `tests/e2e/stage-layout.spec.ts` lines 18-27. Append two entries at the END of the array (so existing indices 0-4 do not shift):

```typescript
const TEST_SLIDES = [
  { main: "Hosana, Hosana\nHosana v výšinách", group: "Chorus" },
  { main: "Požehnaný kto prichádza\nv mene Pánovom", group: "Žalm Ť" },
  {
    main: "Sláva Ti Lev z Júdy\nnech teraz reve Lev",
    group: "Muži // Ženy",
  },
  { main: "Veľký je náš Boh\na hoden chvály", group: "Všetci" },
  { main: "Short line", group: "A" },
  // index 5: single-line, 30 chars with diacritics — should auto-break
  { main: "Nad všetkých vyvyšený bude Pán", group: "Auto" },
  // index 6: single-line, 12 chars — below threshold, must NOT break
  { main: "Ježiš je Pán", group: "Auto" },
];
```

- [ ] **Step 2: Add the split assertion test**

Append to `tests/e2e/stage-layout.spec.ts` (after the last existing test in the file):

```typescript
test("stage auto-breaks single-line slide over 26 chars", async ({
  context,
}) => {
  const consoleMessages: string[] = [];

  const stagePage = await openStageDisplay(context);
  stagePage.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  await triggerSlide(context, 5);
  await stagePage.waitForTimeout(2_000);

  const currentText = await stagePage
    .locator(".stage__current-slide .stage__slide-text")
    .first()
    .evaluate((el) => el.textContent ?? "");

  expect(
    currentText.includes("\n"),
    `Expected a newline to be injected into the slide text, got: ${JSON.stringify(currentText)}`,
  ).toBe(true);

  // Tail-break: the last word "Pán" should be alone on line 2.
  const lines = currentText.split("\n");
  expect(lines.length).toBe(2);
  expect(lines[1].trim()).toBe("Pán");
  // Line 1 must be within the threshold (26 chars).
  expect([...lines[0]].length).toBeLessThanOrEqual(26);

  expect(consoleMessages).toEqual([]);
});

test("stage does not break slide below threshold", async ({ context }) => {
  const consoleMessages: string[] = [];

  const stagePage = await openStageDisplay(context);
  stagePage.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  await triggerSlide(context, 6);
  await stagePage.waitForTimeout(2_000);

  const currentText = await stagePage
    .locator(".stage__current-slide .stage__slide-text")
    .first()
    .evaluate((el) => el.textContent ?? "");

  expect(currentText).toBe("Ježiš je Pán");
  expect(currentText.includes("\n")).toBe(false);

  expect(consoleMessages).toEqual([]);
});
```

- [ ] **Step 3: Run the Playwright tests locally**

Run: `npm run test:playwright -- stage-layout`
Expected: both new tests pass. All prior tests in the file still pass.

- [ ] **Step 4: Commit**

```bash
git add tests/e2e/stage-layout.spec.ts
git commit -m "test(stage): verify single-line auto-break behaviour"
```

---

## Task 6: Playwright E2E — operator UI diacritic warning

The operator UI is a WASM page; slide overflow warning is applied via the `operator__slide-overflow` class inside `.stage__slide-text`-equivalent content in the operator slide list. To keep the test focused and avoid CRUD scaffolding, reuse the existing test presentation created in `stage-layout.spec.ts`'s `beforeAll` (we already add a 28-char diacritic slide at index 5).

**Files:**
- Modify: `tests/e2e/stage-layout.spec.ts`

- [ ] **Step 1: Add a single-line 28-char diacritic slide to `TEST_SLIDES`**

Edit `TEST_SLIDES` in `tests/e2e/stage-layout.spec.ts` (Task 5 already added indices 5 and 6; add index 7):

```typescript
const TEST_SLIDES = [
  // ... entries 0-6 from Task 5 ...
  // index 7: 28 visible chars with diacritics — must NOT trigger overflow warning at limit 32.
  { main: "Nad všetkých vyvyšený bude P", group: "Auto" },
];
```

Verify in your editor that `"Nad všetkých vyvyšený bude P"` is 28 characters (á, ý each count as 1 character).

- [ ] **Step 2: Add the operator warning assertion test**

Append to `tests/e2e/stage-layout.spec.ts`:

```typescript
test("operator UI does not flag 28-char diacritic line as overflow", async ({
  context,
}) => {
  const consoleMessages: string[] = [];
  const page = await context.newPage();
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  await page.goto(new URL("/ui/operator", baseURL).toString(), {
    waitUntil: "domcontentloaded",
  });
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });

  // Navigate to the test presentation.
  await page.waitForSelector('[data-role="library-list"]', { timeout: 15_000 });
  await page.getByText("_E2E Layout Test").first().click();
  await page.getByText("Layout Edge Cases").first().click();

  // Find the slide card for the 28-char diacritic slide (index 7).
  const slideCard = page.locator(
    `[data-slide-id="${testSlideIds[7]}"]`,
  );
  await expect(slideCard).toBeVisible({ timeout: 10_000 });

  // Assert the slide card does NOT contain an overflow warning span.
  const overflowCount = await slideCard
    .locator(".operator__slide-overflow")
    .count();
  expect(
    overflowCount,
    "28-char diacritic line must not trigger overflow warning",
  ).toBe(0);

  expect(consoleMessages).toEqual([]);
});
```

- [ ] **Step 3: Run the test locally**

Run: `npm run test:playwright -- stage-layout`
Expected: the new test passes plus prior tests.

- [ ] **Step 4: Commit**

```bash
git add tests/e2e/stage-layout.spec.ts
git commit -m "test(operator): verify diacritic slide does not trigger overflow warning"
```

---

## Task 7: Format, clippy, push, monitor CI

- [ ] **Step 1: Format check**

Run: `cargo fmt --all --check`
If it fails: `cargo fmt --all` and re-commit with `style: cargo fmt`.

- [ ] **Step 2: Clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings -W clippy::all`
Expected: zero warnings. If warnings appear, fix in one commit.

- [ ] **Step 3: Full unit-test sweep**

Run: `cargo test --workspace`
Expected: all tests pass.

- [ ] **Step 4: Push and capture latest run id**

```bash
git push origin dev
gh run list --branch dev --limit 3 --json databaseId,headSha,status,conclusion,workflowName
```

Record the newest `databaseId` for the pipeline that matches your head SHA.

- [ ] **Step 5: Monitor the pipeline**

Poll the run (no `/loop`, no `gh run watch`, no custom bash monitor script):

```bash
sleep 300 && gh run view <run-id> --json status,conclusion,jobs
```

Re-run the same command (with `run_in_background: true`) as needed until status is `completed`.

All jobs must be `success`: Branch Sync, Format, Clippy, Cargo Deny (x2), Cargo Audit, Test, Coverage, Mutation, Build, Playwright E2E (3/3), Merge E2E Reports, Deploy Dev, Companion Tests, Companion Deploy, Quality Checks, Validate Version, npm Audit.

If any job fails: `gh run view <run-id> --log-failed`, fix all issues in ONE commit, push once, monitor again.

- [ ] **Step 6: Open the PR once CI is green**

```bash
gh pr create --base main --head dev --title "Stage single-line auto-break + operator diacritic warning fix" --body "$(cat <<'EOF'
## Summary
- Single-line worship slides over 26 characters auto-break at the rightmost space (tail-bias) so autofit can render them at a larger font.
- Operator UI slide-overflow warning now counts characters instead of bytes, fixing premature red-flagging of Slovak diacritic lines.

## Test plan
- [x] Unit tests for `break_if_long` cover threshold boundary, multi-line input, single-word overflow, diacritics, empty string.
- [x] Unit tests for `field_has_warning` and `format_multiline` cover ASCII over/under limit and diacritic under limit.
- [x] Playwright E2E: stage splits a 30-char diacritic slide and preserves a 12-char slide.
- [x] Playwright E2E: operator UI does not flag a 28-char diacritic line.
- [x] Zero browser console errors.
EOF
)"
```

Verify mergeability:

```bash
gh api repos/zbynekdrlik/presenter/pulls/$(gh pr view --json number --jq .number) --jq '{mergeable: .mergeable, mergeable_state: .mergeable_state}'
```

Expected: `mergeable: true`, `mergeable_state: "clean"`.

Report the PR URL and wait for explicit user merge approval.

---

## Verification Summary

| Check | How to verify |
|-------|---------------|
| `break_if_long` correctness | `cargo test -p presenter-ui --lib utils::text` — 8 tests pass. |
| Operator byte→char fix | `cargo test -p presenter-ui --lib components::slide_list_utils` — 7 tests pass including diacritic assertions. |
| Stage auto-break at runtime | Playwright: `.stage__current-slide .stage__slide-text` textContent contains `\n` for index-5 slide, does not for index-6. |
| Operator UI char-count fix | Playwright: `.operator__slide-overflow` count is `0` on the index-7 slide card. |
| No console errors | Every Playwright test asserts `consoleMessages` is empty. |
| CI green | All jobs in the pipeline run succeed. |
| No regressions | All prior unit tests and Playwright tests still pass. |
