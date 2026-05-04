# AI Bible Composer Full-Passage Reference Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the AI bible composer emit the same full-passage reference label on every slide of one passage (e.g. all three slides of Numeri 13:17-20 show "Numeri 13:17-20 (SEB)" instead of per-slide ranges).

**Architecture:** In `compose_bible_items_into_slides`, pre-scan `items[]` once to build a `HashMap<(book, chapter, translation), BTreeSet<u32>>` of all verse numbers. The flush closure uses this map to emit the full-range reference instead of the per-slide range from `cur_numbers`. A new `format_verse_range` helper handles contiguous (`"17-20"`), single (`"17"`), and non-contiguous (`"1, 3, 5"`) cases.

**Tech Stack:** Rust (server-side only), `std::collections::{HashMap, BTreeSet}`. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-05-04-ai-bible-full-passage-reference-design.md` (commit 63fb3bc).

---

## Context

Issue #292: when the AI splits a multi-verse passage across slides, each slide currently shows a per-slide reference like "Numeri 13:17-18", "Numeri 13:19-20" instead of the full "Numeri 13:17-20" on every slide. The non-AI bible-load flow already does this correctly. The AI-path composer at `crates/presenter-server/src/state/slides.rs:91-95` derives the reference from `cur_numbers.first()`/`cur_numbers.last()` — i.e. only the verses on the current slide.

**Key existing code:**

- `crates/presenter-server/src/state/slides.rs:22-34` — `BibleItem` enum with fields `number: u32, text: String, book: String, chapter: u32, translation: String`.
- `crates/presenter-server/src/state/slides.rs:52-171` — `compose_bible_items_into_slides`. The flush closure `flush_verses` is at lines 69-102.
- `crates/presenter-server/src/state/slides.rs:500+` — existing `tests` module. Already imports `super::*`, `BibleReference`, and `HashMap`.
- `crates/presenter-server/src/ai/tools.rs:820-865` — existing test `create_bible_presentation_with_items_composes_server_side` asserts slide 0 ref = "Ján 1:1-2 (SEB)" and slide 2 ref = "Ján 1:3 (SEB)". Under the new behavior both should be "Ján 1:1-3 (SEB)".

---

## File Structure

### Modified Files

| File | Change |
|------|--------|
| `Cargo.toml` | Workspace version 0.4.58 → 0.4.59 |
| `crates/presenter-ui/Cargo.toml` | Version 0.1.27 → 0.1.28 |
| `crates/presenter-server/src/state/slides.rs` | Add Pass 1 group-verses precompute; add `format_verse_range` free function; modify `flush_verses` to use group lookup. Add 5 unit tests. |
| `crates/presenter-server/src/ai/tools.rs` | Update `create_bible_presentation_with_items_composes_server_side` test to assert the new full-range references. |

---

## Task 1: Version Bump

**Files:**
- Modify: `Cargo.toml:15`
- Modify: `crates/presenter-ui/Cargo.toml:3`
- Modify: `Cargo.lock` (auto)
- Modify: `crates/presenter-ui/Cargo.lock` (auto)

- [ ] **Step 1: Bump workspace version**

In `Cargo.toml`, change line 15:

```toml
[workspace.package]
version = "0.4.59"
```

- [ ] **Step 2: Bump presenter-ui version**

In `crates/presenter-ui/Cargo.toml`, change line 3:

```toml
version = "0.1.28"
```

- [ ] **Step 3: Update lockfiles**

```bash
cargo update --workspace
cargo update --workspace --manifest-path crates/presenter-ui/Cargo.toml
```

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock crates/presenter-ui/Cargo.toml crates/presenter-ui/Cargo.lock
git commit -m "chore: bump version to 0.4.59"
```

---

## Task 2: Implement full-passage reference computation

**Files:**
- Modify: `crates/presenter-server/src/state/slides.rs:1-10` (imports)
- Modify: `crates/presenter-server/src/state/slides.rs:52-171` (composer body)
- Modify: `crates/presenter-server/src/state/slides.rs` (add `format_verse_range` free function near `compose_bible_items_into_slides`)

- [ ] **Step 1: Update imports**

In `crates/presenter-server/src/state/slides.rs`, replace line 6:

```rust
use std::collections::HashMap;
```

with:

```rust
use std::collections::{BTreeSet, HashMap};
```

- [ ] **Step 2: Add `format_verse_range` free function**

In `crates/presenter-server/src/state/slides.rs`, add this free function IMMEDIATELY ABOVE `pub(crate) fn compose_bible_items_into_slides` (i.e. before line 52, after the existing doc comment block):

```rust
/// Format a sorted set of verse numbers as a reference suffix.
///
/// - Empty set → "" (caller skips the reference entirely).
/// - Single verse → "17".
/// - Contiguous range → "17-20".
/// - Non-contiguous → "1, 3, 5" (flat comma-list, no mixed range syntax).
fn format_verse_range(verses: &BTreeSet<u32>) -> String {
    let v: Vec<u32> = verses.iter().copied().collect();
    if v.is_empty() {
        return String::new();
    }
    let min = v[0];
    let max = v[v.len() - 1];
    let count = v.len() as u32;
    if min == max {
        format!("{}", min)
    } else if max - min + 1 == count {
        format!("{}-{}", min, max)
    } else {
        v.iter()
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    }
}
```

- [ ] **Step 3: Replace the body of `compose_bible_items_into_slides`**

Replace lines 52-171 (the entire `pub(crate) fn compose_bible_items_into_slides ... }` function body) with this version. The change vs. the original: a new Pass 1 builds `group_verses`, the `flush_verses` closure captures `&group_verses` and uses it for the reference, and the per-slide range derivation is removed.

```rust
pub(crate) fn compose_bible_items_into_slides(
    items: &[BibleItem],
    character_limit: u32,
) -> Vec<ComposedBibleSlide> {
    let limit = character_limit as usize;
    let mut slides: Vec<ComposedBibleSlide> = Vec::new();

    // Pass 1: collect every verse number per (book, chapter, translation)
    // group across the whole items[] stream. Slides flushed in pass 2 use
    // this group's full verse list for the reference label, so all slides
    // of one passage display the same reference (issue #292).
    let mut group_verses: HashMap<(String, u32, String), BTreeSet<u32>> = HashMap::new();
    for item in items {
        if let BibleItem::Verse {
            number,
            book,
            chapter,
            translation,
            ..
        } = item
        {
            group_verses
                .entry((book.clone(), *chapter, translation.clone()))
                .or_default()
                .insert(*number);
        }
    }

    // Accumulator for the current verse slide.
    let mut cur_lines: Vec<String> = Vec::new();
    let mut cur_numbers: Vec<u32> = Vec::new();
    let mut cur_group: Option<(String, u32, String)> = None; // (book, chapter, translation)

    // Flush the current verse accumulator into a slide. The reference is
    // derived from the GROUP's full verse set (built in pass 1), not from
    // cur_numbers, so every slide of one passage displays the same label.
    //
    // Uses let-else pattern matching instead of expect()/unwrap() so there
    // is no panic path — if an invariant is ever broken by a future
    // refactor (e.g., group set without lines), the flush is a no-op
    // rather than a crash.
    let flush_verses = |slides: &mut Vec<ComposedBibleSlide>,
                        lines: &mut Vec<String>,
                        numbers: &mut Vec<u32>,
                        group: &mut Option<(String, u32, String)>,
                        group_verses: &HashMap<(String, u32, String), BTreeSet<u32>>| {
        if lines.is_empty() {
            *group = None;
            return;
        }
        let Some((book, chapter, translation)) = group.take() else {
            lines.clear();
            numbers.clear();
            return;
        };
        if numbers.is_empty() {
            lines.clear();
            return;
        }
        let main = lines.join("\n");
        let reference = match group_verses.get(&(book.clone(), chapter, translation.clone())) {
            Some(verses) => format!(
                "{} {}:{} ({})",
                book,
                chapter,
                format_verse_range(verses),
                translation
            ),
            None => String::new(),
        };
        slides.push(ComposedBibleSlide {
            main,
            main_reference: reference,
        });
        lines.clear();
        numbers.clear();
    };

    for item in items {
        match item {
            BibleItem::Emphasis { text } => {
                flush_verses(
                    &mut slides,
                    &mut cur_lines,
                    &mut cur_numbers,
                    &mut cur_group,
                    &group_verses,
                );
                slides.push(ComposedBibleSlide {
                    main: text.clone(),
                    main_reference: String::new(),
                });
            }
            BibleItem::Verse {
                number,
                text,
                book,
                chapter,
                translation,
            } => {
                // Translation / book / chapter change forces a slide break.
                if let Some((cur_book, cur_chapter, cur_tr)) = &cur_group {
                    if cur_book != book || cur_chapter != chapter || cur_tr != translation {
                        flush_verses(
                            &mut slides,
                            &mut cur_lines,
                            &mut cur_numbers,
                            &mut cur_group,
                            &group_verses,
                        );
                    }
                }

                let line = format!("{}. {}", number, text);
                let existing_len: usize = cur_lines.iter().map(String::len).sum();
                let prospective = if cur_lines.is_empty() {
                    line.len()
                } else {
                    // existing lines joined by "\n" = existing_len + (cur_lines.len() - 1)
                    // plus "\n" + new line = + 1 + line.len()
                    // total = existing_len + cur_lines.len() + line.len()
                    existing_len + cur_lines.len() + line.len()
                };

                if prospective > limit && !cur_lines.is_empty() {
                    flush_verses(
                        &mut slides,
                        &mut cur_lines,
                        &mut cur_numbers,
                        &mut cur_group,
                        &group_verses,
                    );
                }

                cur_lines.push(line);
                cur_numbers.push(*number);
                cur_group = Some((book.clone(), *chapter, translation.clone()));
            }
        }
    }

    flush_verses(
        &mut slides,
        &mut cur_lines,
        &mut cur_numbers,
        &mut cur_group,
        &group_verses,
    );
    slides
}
```

- [ ] **Step 4: Verify build**

```bash
cargo build -p presenter-server
```

Expected: build passes. If clippy flags the unused `cur_numbers` variable — note that `cur_numbers` is still used as the accumulator and as the non-empty guard inside `flush_verses` (the `if numbers.is_empty()` check), so it must stay. If clippy doesn't flag it, no action needed.

- [ ] **Step 5: Run existing tests in slides.rs**

```bash
cargo test -p presenter-server -- state::slides --nocapture
```

Expected: all existing tests in `slides.rs` pass. The `compose_bible_slides_*` tests (line 525, 571, 596) test the OTHER composer (`compose_bible_slides`) which is unchanged — they should continue passing.

- [ ] **Step 6: Run the full presenter-server test suite**

```bash
cargo test -p presenter-server -- --nocapture
```

Expected: ONE test fails — `create_bible_presentation_with_items_composes_server_side` in `tools.rs:820`. It asserts the old per-slide behavior. Task 4 will update it. Other tests pass.

If MORE than one test fails, the changes have a regression — investigate before proceeding.

- [ ] **Step 7: Commit**

```bash
cargo fmt --all
git add crates/presenter-server/src/state/slides.rs
git commit -m "feat(ai): full-passage reference on every bible slide (#292)

compose_bible_items_into_slides now pre-scans items[] to collect
every verse number per (book, chapter, translation) group, and
emits the same full-range reference label on every slide of that
group. Contiguous verses render as 'a-b'; non-contiguous as
'a, b, c'. Single-verse passages stay 'a'.

The non-AI compose_bible_slides path was already correct; this
fix aligns the AI path with the documented style guide."
```

---

## Task 3: Unit tests for the new behavior

**Files:**
- Modify: `crates/presenter-server/src/state/slides.rs` (existing tests module starting at line 500)

- [ ] **Step 1: Add test helpers if needed**

Check whether the `tests` module already has helpers for constructing `BibleItem` values:

```bash
grep -n "BibleItem::Verse\|BibleItem::Emphasis" crates/presenter-server/src/state/slides.rs
```

If the tests module doesn't have a helper, add one near the other test helpers (search `grep -n "fn test_translation\|fn test_passage" crates/presenter-server/src/state/slides.rs` for the existing helper section). Add this helper function inside the `mod tests` block, before the first `#[test]`:

```rust
    fn verse(number: u32, text: &str, book: &str, chapter: u32, translation: &str) -> BibleItem {
        BibleItem::Verse {
            number,
            text: text.to_string(),
            book: book.to_string(),
            chapter,
            translation: translation.to_string(),
        }
    }

    fn emphasis(text: &str) -> BibleItem {
        BibleItem::Emphasis {
            text: text.to_string(),
        }
    }
```

If a helper already exists, reuse it (and skip this step).

- [ ] **Step 2: Add `compose_uses_full_passage_range_across_split_slides` test**

At the end of the `tests` module (before the final `}`), add:

```rust
    #[test]
    fn compose_uses_full_passage_range_across_split_slides() {
        // Numeri 13:17-20 forced to split into 2 slides via low char limit.
        // Both slides must show the FULL range "Numeri 13:17-20 (SEB)".
        let items = vec![
            verse(17, "Verse seventeen text long enough to fill", "Numeri", 13, "SEB"),
            verse(18, "Verse eighteen text", "Numeri", 13, "SEB"),
            verse(19, "Verse nineteen text long enough to fill", "Numeri", 13, "SEB"),
            verse(20, "Verse twenty text", "Numeri", 13, "SEB"),
        ];
        // Char limit chosen so that 2 verses pack into one slide and the
        // next two pack into the second slide.
        let slides = compose_bible_items_into_slides(&items, 80);
        assert!(
            slides.len() >= 2,
            "expected at least 2 slides, got {}",
            slides.len()
        );
        for (i, slide) in slides.iter().enumerate() {
            assert_eq!(
                slide.main_reference, "Numeri 13:17-20 (SEB)",
                "slide {} must show full range",
                i
            );
        }
    }
```

- [ ] **Step 3: Add `compose_handles_emphasis_between_verses_with_full_range` test**

```rust
    #[test]
    fn compose_handles_emphasis_between_verses_with_full_range() {
        let items = vec![
            verse(17, "Verse seventeen", "Numeri", 13, "SEB"),
            emphasis("DÔLEŽITÉ SLOVO"),
            verse(18, "Verse eighteen", "Numeri", 13, "SEB"),
            verse(19, "Verse nineteen", "Numeri", 13, "SEB"),
            verse(20, "Verse twenty", "Numeri", 13, "SEB"),
        ];
        let slides = compose_bible_items_into_slides(&items, 320);
        // Find the emphasis slide (empty reference, main = "DÔLEŽITÉ SLOVO")
        let emphasis_slide = slides
            .iter()
            .find(|s| s.main == "DÔLEŽITÉ SLOVO")
            .expect("emphasis slide present");
        assert_eq!(
            emphasis_slide.main_reference, "",
            "emphasis slide has empty reference"
        );
        // Every verse slide (the ones whose main contains "Verse ") must
        // show the full passage range.
        let verse_slides: Vec<&ComposedBibleSlide> =
            slides.iter().filter(|s| s.main.contains("Verse ")).collect();
        assert!(verse_slides.len() >= 2, "expected at least 2 verse slides");
        for (i, slide) in verse_slides.iter().enumerate() {
            assert_eq!(
                slide.main_reference, "Numeri 13:17-20 (SEB)",
                "verse slide {} must show full range across emphasis interruption",
                i
            );
        }
    }
```

- [ ] **Step 4: Add `compose_two_distinct_passages_get_independent_ranges` test**

```rust
    #[test]
    fn compose_two_distinct_passages_get_independent_ranges() {
        let items = vec![
            verse(1, "In the beginning was the Word.", "Ján", 1, "SEB"),
            verse(2, "He was in the beginning with God.", "Ján", 1, "SEB"),
            verse(3, "Blessed are the poor in spirit.", "Mat", 5, "SEB"),
        ];
        let slides = compose_bible_items_into_slides(&items, 320);
        let jan_slides: Vec<&ComposedBibleSlide> = slides
            .iter()
            .filter(|s| s.main_reference.starts_with("Ján"))
            .collect();
        let mat_slides: Vec<&ComposedBibleSlide> = slides
            .iter()
            .filter(|s| s.main_reference.starts_with("Mat"))
            .collect();
        assert!(!jan_slides.is_empty(), "Ján slides present");
        assert!(!mat_slides.is_empty(), "Mat slides present");
        for slide in &jan_slides {
            assert_eq!(slide.main_reference, "Ján 1:1-2 (SEB)");
        }
        for slide in &mat_slides {
            assert_eq!(slide.main_reference, "Mat 5:3 (SEB)");
        }
    }
```

- [ ] **Step 5: Add `compose_non_contiguous_verses_render_as_comma_list` test**

```rust
    #[test]
    fn compose_non_contiguous_verses_render_as_comma_list() {
        // Pastor cited only verses 1, 3, 5 — non-contiguous. Reference
        // must show the explicit list, not a misleading 1-5 range.
        let items = vec![
            verse(1, "First cited verse content here", "Numeri", 13, "SEB"),
            verse(3, "Third cited verse content here", "Numeri", 13, "SEB"),
            verse(5, "Fifth cited verse content here", "Numeri", 13, "SEB"),
        ];
        let slides = compose_bible_items_into_slides(&items, 60);
        assert!(!slides.is_empty(), "at least one slide");
        for (i, slide) in slides.iter().enumerate() {
            assert_eq!(
                slide.main_reference, "Numeri 13:1, 3, 5 (SEB)",
                "slide {} must show comma-list of cited verses",
                i
            );
        }
    }
```

- [ ] **Step 6: Add `compose_mixed_gap_renders_as_comma_list` test**

```rust
    #[test]
    fn compose_mixed_gap_renders_as_comma_list() {
        // Mixed gap (skip verse 3): 1, 2, 4, 5. Not perfectly contiguous,
        // so the reference is a flat comma-list — no mixed "1-2, 4-5" syntax.
        let items = vec![
            verse(1, "Verse one", "Numeri", 13, "SEB"),
            verse(2, "Verse two", "Numeri", 13, "SEB"),
            verse(4, "Verse four", "Numeri", 13, "SEB"),
            verse(5, "Verse five", "Numeri", 13, "SEB"),
        ];
        let slides = compose_bible_items_into_slides(&items, 320);
        assert!(!slides.is_empty(), "at least one slide");
        for (i, slide) in slides.iter().enumerate() {
            assert_eq!(
                slide.main_reference, "Numeri 13:1, 2, 4, 5 (SEB)",
                "slide {} must show flat comma-list",
                i
            );
        }
    }
```

- [ ] **Step 7: Run the new tests**

```bash
cargo test -p presenter-server -- state::slides::tests::compose_uses_full_passage_range_across_split_slides state::slides::tests::compose_handles_emphasis_between_verses_with_full_range state::slides::tests::compose_two_distinct_passages_get_independent_ranges state::slides::tests::compose_non_contiguous_verses_render_as_comma_list state::slides::tests::compose_mixed_gap_renders_as_comma_list --nocapture
```

Expected: all 5 new tests pass.

If a test fails because the char limit triggers more or fewer slides than expected, adjust the limit value. The intent is: tests 1, 4 force a split (low limit); tests 2, 3, 5 use a high limit and let the natural slide breaks happen.

- [ ] **Step 8: Commit**

```bash
cargo fmt --all
git add crates/presenter-server/src/state/slides.rs
git commit -m "test(slides): cover full-passage reference on every slide (#292)

Five new unit tests in state::slides::tests:
- compose_uses_full_passage_range_across_split_slides
- compose_handles_emphasis_between_verses_with_full_range
- compose_two_distinct_passages_get_independent_ranges
- compose_non_contiguous_verses_render_as_comma_list
- compose_mixed_gap_renders_as_comma_list"
```

---

## Task 4: Update existing test for the new behavior

**Files:**
- Modify: `crates/presenter-server/src/ai/tools.rs:855-864`

The existing test `create_bible_presentation_with_items_composes_server_side` asserts the OLD per-slide behavior. Items in that test:

- verse 1 (Ján 1)
- verse 2 (Ján 1)
- emphasis "NOVÁ ZMLUVA"
- verse 3 (Ján 1)

Old assertions: slide 0 ref = "Ján 1:1-2", slide 2 ref = "Ján 1:3".

New behavior: every verse slide shows the full group range = verses 1, 2, 3 of Ján 1. Verses 1, 2, 3 are contiguous → "Ján 1:1-3 (SEB)" on every verse slide. Emphasis stays empty.

- [ ] **Step 1: Update slide 0 assertion**

In `crates/presenter-server/src/ai/tools.rs`, find the line containing `assert_eq!(pres.slides[0].main_reference, "Ján 1:1-2 (SEB)");` (around line 855). Replace it with:

```rust
        // Slide 0: verses 1-2, ref shows full passage range across all verse items
        assert_eq!(pres.slides[0].main_reference, "Ján 1:1-3 (SEB)");
```

- [ ] **Step 2: Update slide 2 assertion**

Find the line containing `assert_eq!(pres.slides[2].main_reference, "Ján 1:3 (SEB)");` (around line 864). Replace it with:

```rust
        // Slide 2: verse 3, same full passage range as slide 0
        assert_eq!(pres.slides[2].main_reference, "Ján 1:1-3 (SEB)");
```

The `assert_eq!(pres.slides[1].main_reference, "");` line (emphasis slide) stays unchanged.

- [ ] **Step 3: Run the test**

```bash
cargo test -p presenter-server -- create_bible_presentation_with_items_composes_server_side --nocapture
```

Expected: passes.

- [ ] **Step 4: Run the full test suite**

```bash
cargo test -p presenter-server -- --nocapture
```

Expected: ALL tests pass (no failures, no ignores).

- [ ] **Step 5: Run clippy**

```bash
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all
```

Expected: clean. Zero warnings.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add crates/presenter-server/src/ai/tools.rs
git commit -m "test(ai): update bible composer test for full-range reference (#292)

create_bible_presentation_with_items_composes_server_side now
asserts every verse slide shows the same full-passage reference
'Ján 1:1-3 (SEB)' instead of per-slide ranges. Aligns with the
new compose_bible_items_into_slides behavior from #292."
```

---

## Task 5: Local checks, push, monitor CI, deploy verify, PR, completion report

**Controller-handled task.** Each step is what the controller (or a human) does after Tasks 1-4 are committed.

- [ ] **Step 1: Run all local checks**

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all
cargo test -p presenter-server -- --nocapture
```

If any fail, fix in ONE commit and re-run.

- [ ] **Step 2: Push to dev**

```bash
git push origin dev
```

- [ ] **Step 3: Monitor CI to terminal state**

```bash
gh run list --branch dev --limit 1 --json databaseId --jq '.[0].databaseId'
# Capture run id, then:
sleep 1500 && gh run view <run-id> --json status,conclusion,jobs --jq '{status, conclusion, failed: [.jobs[] | select(.conclusion == "failure") | .name]}'
```

If any job fails, `gh run view <run-id> --log-failed`, fix in ONE commit, push again, re-monitor.

- [ ] **Step 4: Verify dev deployment is live**

```bash
curl -s http://10.77.8.134:8080/healthz
```

Expected: `{"channel":"dev","status":"ok","version":"0.4.59"}`.

- [ ] **Step 5: Manual verification via real AI request**

Open the operator UI in Playwright at `http://10.77.8.134:8080/ui/operator`, navigate to the AI tab, and ask the AI to create a bible presentation that spans multiple slides. Example prompt:

```
Vytvor bibliu z pasáže Numeri 13:17-20 v preklade SEB.
```

After the AI calls `create_bible_presentation`, fetch the resulting bible presentation via the API:

```bash
# List bible presentations
curl -s http://10.77.8.134:8080/bibles | python3 -m json.tool

# Find the new one and fetch its slides
curl -s http://10.77.8.134:8080/bibles/<id> | python3 -c "import json,sys; d=json.load(sys.stdin); [print(s['main_reference']) for s in d.get('slides', [])]"
```

Verify EVERY slide's `main_reference` is `"Numeri 13:17-20 (SEB)"` (the full range), not per-slide ranges.

If the AI couldn't complete the request (model error, missing translation, etc.), fall back to a curl-based test using the `/bibles/items` API directly (or whatever endpoint `create_bible_presentation` exposes). The point is to exercise `compose_bible_items_into_slides` against the live deployed binary, not just unit tests.

- [ ] **Step 6: Open PR**

```bash
gh pr create --title "fix(ai): full-passage reference on every bible slide (#292)" --body "$(cat <<'EOF'
## Summary

Fixes #292: when the AI composes a multi-verse bible passage that spans multiple slides, every slide now displays the SAME full-passage reference (e.g. "Numeri 13:17-20 (SEB)") instead of per-slide ranges. Matches the documented style guide and the non-AI bible-load flow.

## What changed

In `compose_bible_items_into_slides` (`crates/presenter-server/src/state/slides.rs`):

1. Pre-scan `items[]` to collect every verse number per `(book, chapter, translation)` group into a `BTreeSet<u32>`.
2. New free function `format_verse_range` emits the reference suffix:
   - Single verse → `"17"`
   - Contiguous → `"17-20"`
   - Non-contiguous → `"1, 3, 5"` (flat comma-list)
3. The `flush_verses` closure uses the group's full verse set for the reference label, not the per-slide range.

## Test plan

- [ ] All existing tests pass
- [ ] 5 new unit tests pass:
  - `compose_uses_full_passage_range_across_split_slides`
  - `compose_handles_emphasis_between_verses_with_full_range`
  - `compose_two_distinct_passages_get_independent_ranges`
  - `compose_non_contiguous_verses_render_as_comma_list`
  - `compose_mixed_gap_renders_as_comma_list`
- [ ] `create_bible_presentation_with_items_composes_server_side` updated to assert new full-range behavior
- [ ] Dev `/healthz` reports v0.4.59
- [ ] Manual verification: AI creates Numeri 13:17-20 presentation → every slide shows "Numeri 13:17-20 (SEB)"
- [ ] Browser console clean

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 7: Verify PR is mergeable**

```bash
gh pr view <pr-number> --json mergeable,mergeStateStatus
```

Expected: `mergeable: true`, `mergeStateStatus: CLEAN`. If not, investigate.

- [ ] **Step 8: Run pre-completion gates**

Invoke `/plan-check` skill — must come back N/N fulfilled. Invoke `/review` skill on this PR — must come back `0 🔴 0 🟡 0 🔵`. Fix any findings inside the diff before sending the completion report.

- [ ] **Step 9: Send completion report**

Per `core/completion-report.md`. Include CI run ID, plan-check N/N, review clean, deploy verification (dev shows v0.4.59 with the AI-generated multi-slide bible passage showing identical references), URLs, PR title + URL.

---

## Verification Summary

| Check | How to verify |
|-------|---------------|
| Full-range reference contiguous | Unit test `compose_uses_full_passage_range_across_split_slides` passes |
| Emphasis interruption preserved | Unit test `compose_handles_emphasis_between_verses_with_full_range` passes |
| Independent passage ranges | Unit test `compose_two_distinct_passages_get_independent_ranges` passes |
| Non-contiguous comma-list | Unit test `compose_non_contiguous_verses_render_as_comma_list` passes |
| Mixed gap comma-list | Unit test `compose_mixed_gap_renders_as_comma_list` passes |
| Single verse stays single | Existing tests already cover (slide 1 of Ján 1:3 was "Ján 1:3" — now "Ján 1:1-3" because of the multi-verse group; the `min == max` branch in `format_verse_range` handles a true single-verse passage) |
| No regressions | All 192 existing presenter-server tests still pass |
| Live behavior on dev | Real AI request produces presentation where every slide shows the full passage reference |
| Clean console | Playwright session shows zero console errors |
