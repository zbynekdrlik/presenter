# Clipboard Import Follow-Up Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the operator's "create from paste" flow extract a Title-as-presentation-name (zero-padded), word-wrap long lyric lines, chunk each section into ≤ 2-line slides, strip Unicode control chars, and add empty bookend slides — so a spevnik song export imports cleanly.

**Architecture:** Pipeline of small composable functions in `crates/presenter-ui/src/utils/song_parser.rs` (extract_title → parse_song_text with sanitize → wrap_long_lines → chunk_to_two_lines → wrap_with_empty_bookends). Modal's `on_paste_confirm` reads `op.line_limit`, runs the pipeline, ignores any typed name, hides the name input on the paste sub-screen.

**Tech Stack:** Rust + Leptos 0.7 (WASM), Playwright/TypeScript.

**Spec:** `docs/superpowers/specs/2026-04-30-clipboard-import-followup-design.md` (commit `248a66c`).

---

## Context

Builds on PR #277 (currently open from `dev` to `main`, mergeable, clean). `dev` is at commit `248a66c` (1 spec commit ahead of #277's last fix). New work piles onto `dev` — PR #277 auto-updates.

The existing `parse_song_text(text) -> Vec<SlideInput>` from PR #277 stays — only its lyric-line append is modified to call a new `sanitize_line` helper. The JS bridge `__presenterOperatorTestHelpers.parseSongText` keeps the same signature (it calls `parse_song_text`); the existing E2E `tests/e2e/wasm-edge-cases.spec.ts:259` regression test still passes.

`presenter-ui` has its own `Cargo.lock` (excluded from workspace).
- Tests: `cd crates/presenter-ui && cargo test --target x86_64-unknown-linux-gnu --lib song_parser`
- Clippy: `cd crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings`
- Format: `cd crates/presenter-ui && cargo fmt`
- Workspace fmt check (root): `cargo fmt --all --check`

`OperatorState.line_limit` is a `RwSignal<u32>` (default 32, persisted to localStorage as `lineLimit`) defined in `crates/presenter-ui/src/state/operator.rs:75`. The modal already has `op` in scope, so `op.line_limit.get_untracked() as usize` is the source of truth for line wrapping.

The modal sub-flow signal is `create_step: RwSignal<String>` taking values `"options"` / `"paste"` / `"import"` (declared at the top of `presentation_modal.rs`). Hiding the name input on paste = `style:display=move || if create_step.get() == "paste" { "none" } else { "block" }` on the `<label>` wrapping the input.

---

## File Structure

| File | Change |
|------|--------|
| `Cargo.toml` (workspace `[workspace.package]`) | `0.4.46` → `0.4.47` |
| `crates/presenter-ui/Cargo.toml` | `0.1.15` → `0.1.16` |
| `crates/presenter-ui/src/utils/song_parser.rs` | Add `extract_title`, `pad_title_number`, `sanitize_line`, `is_zero_width`, `wrap_long_lines`, `wrap_one_line`, `hard_break_into`, `chunk_to_two_lines`, `wrap_with_empty_bookends`. Modify the line-append branch of `parse_song_text` to use `sanitize_line` and skip lines that become empty after sanitization. Add ~14 new unit tests. |
| `crates/presenter-ui/src/components/presentation_modal.rs:230-292` | Replace `on_paste_confirm` body with the new pipeline using extract_title for the name and the line_limit from `op`. |
| `crates/presenter-ui/src/components/presentation_modal.rs:447-459` | Wrap the name input's `<label>` so it hides when `create_step.get() == "paste"`. |
| `tests/e2e/wasm-presentation-crud.spec.ts:430` | **Replace** the existing `paste with section headers splits one slide per section (#275)` test body with the canonical `076 Arriba` paste, asserting 15 slides + bookends + hidden name input. |

---

## Task 1: Bump version 0.4.46 → 0.4.47

**Files:**
- Modify: `Cargo.toml` (workspace `[workspace.package]`)
- Modify: `crates/presenter-ui/Cargo.toml`

- [ ] **Step 1: Sync with remote**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
git fetch origin
git status -sb
```

Expected: `## dev...origin/dev` (no divergence).

- [ ] **Step 2: Bump workspace version**

In `/home/newlevel/devel/presenter/presenter-dev2/Cargo.toml` under `[workspace.package]`:

```toml
version = "0.4.47"
```

(was `0.4.46`).

- [ ] **Step 3: Bump presenter-ui version**

In `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui/Cargo.toml` under `[package]`:

```toml
version = "0.1.16"
```

(was `0.1.15`).

- [ ] **Step 4: Refresh Cargo.lock files**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
cargo check -p presenter-server 2>&1 | tail -3
cd crates/presenter-ui && cargo check --target wasm32-unknown-unknown 2>&1 | tail -3 && cd ../..
```

Expected: clean compile.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock crates/presenter-ui/Cargo.toml crates/presenter-ui/Cargo.lock
git commit -m "chore: bump version to 0.4.47 (#275 follow-up)"
```

---

## Task 2: Add extract_title + pad_title_number (TDD)

**Files:**
- Modify: `crates/presenter-ui/src/utils/song_parser.rs`

- [ ] **Step 1: Add 9 failing unit tests at the end of the existing `mod tests {}`**

Open `crates/presenter-ui/src/utils/song_parser.rs`. Find the existing `#[cfg(test)] mod tests` block (it has 10 existing tests from PR #277). Append these tests INSIDE that module, just before the closing `}`:

```rust
    #[test]
    fn extract_title_pads_two_digit_prefix() {
        assert_eq!(
            extract_title("Title: 76 Arriba\nMisc 1\n\nVerse 1\nlyric"),
            Some("076 Arriba".to_string())
        );
    }

    #[test]
    fn extract_title_pads_one_digit_prefix() {
        assert_eq!(
            extract_title("Title: 6 Foo"),
            Some("006 Foo".to_string())
        );
    }

    #[test]
    fn extract_title_leaves_three_digit_prefix_unchanged() {
        assert_eq!(
            extract_title("Title: 102 10000 Armád"),
            Some("102 10000 Armád".to_string())
        );
    }

    #[test]
    fn extract_title_leaves_four_digit_prefix_unchanged() {
        assert_eq!(
            extract_title("Title: 1234 Bar"),
            Some("1234 Bar".to_string())
        );
    }

    #[test]
    fn extract_title_no_leading_number_unchanged() {
        assert_eq!(
            extract_title("Title: Arriba"),
            Some("Arriba".to_string())
        );
    }

    #[test]
    fn extract_title_no_space_after_digits_unchanged() {
        // No whitespace separator → not a song-number prefix; leave alone.
        assert_eq!(
            extract_title("Title: 76Arriba"),
            Some("76Arriba".to_string())
        );
    }

    #[test]
    fn extract_title_returns_none_when_absent() {
        assert_eq!(extract_title("Verse 1\nlyric"), None);
    }

    #[test]
    fn extract_title_is_case_insensitive() {
        assert_eq!(extract_title("TITLE: foo"), Some("foo".to_string()));
        assert_eq!(extract_title("title: foo"), Some("foo".to_string()));
    }

    #[test]
    fn extract_title_strips_surrounding_whitespace() {
        assert_eq!(
            extract_title("Title:   foo bar  "),
            Some("foo bar".to_string())
        );
    }
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui
cargo test --target x86_64-unknown-linux-gnu --lib song_parser::tests::extract_title 2>&1 | tail -20
```

Expected: 9 failures with `cannot find function extract_title`.

- [ ] **Step 3: Implement `extract_title` and `pad_title_number`**

Open the same file. After the existing `is_metadata_line` function (around line 128) but before the `#[cfg(test)]` line, add:

```rust
/// Extract the song title from the first `Title:` line in the input.
///
/// Returns the trimmed content after the colon, with a leading 1-3 digit
/// numeric prefix padded to 3 digits via [`pad_title_number`]. Returns
/// [`None`] if no `Title:` line is found.
pub fn extract_title(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let trimmed = line.trim();
        let lower = trimmed.to_ascii_lowercase();
        if !lower.starts_with("title:") {
            return None;
        }
        // 6 = len("title:") — the prefix is ASCII so byte index works on
        // the original (preserves casing of the title content).
        let raw = trimmed[6..].trim();
        Some(pad_title_number(raw))
    })
}

/// Pad a leading 1-3 digit numeric prefix (followed by whitespace) to 3
/// digits with leading zeros so songs sort correctly.
///
/// - `"76 Arriba"`        → `"076 Arriba"`
/// - `"6 Foo"`            → `"006 Foo"`
/// - `"102 10000 Armád"`  → `"102 10000 Armád"`  (already 3 digits)
/// - `"1234 Bar"`         → `"1234 Bar"`          (4+ digits, untouched)
/// - `"Arriba"`           → `"Arriba"`            (no leading number)
/// - `"76Arriba"`         → `"76Arriba"`          (no whitespace separator)
fn pad_title_number(title: &str) -> String {
    let trimmed = title.trim();
    if let Some(space_idx) = trimmed.find(char::is_whitespace) {
        let (prefix, rest) = trimmed.split_at(space_idx);
        if !prefix.is_empty()
            && prefix.len() <= 3
            && prefix.chars().all(|c| c.is_ascii_digit())
        {
            return format!("{prefix:0>3}{rest}");
        }
    }
    trimmed.to_string()
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui
cargo test --target x86_64-unknown-linux-gnu --lib song_parser 2>&1 | tail -25
```

Expected: 19 tests pass (10 existing + 9 new).

- [ ] **Step 5: Run clippy + fmt**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui
cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings 2>&1 | tail -5
cargo fmt
```

Expected: clippy clean, no fmt diff.

- [ ] **Step 6: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
git add crates/presenter-ui/src/utils/song_parser.rs
git commit -m "feat(ui): add extract_title with 3-digit numeric prefix padding (#275)

extract_title scans for the first Title: line case-insensitively and
runs the result through pad_title_number, which pads a leading 1-3
digit prefix followed by whitespace to 3 digits with leading zeros.
Songs like 'Title: 76 Arriba' become '076 Arriba' so they sort
correctly in the operator library list.

9 new unit tests covering the padding rules, no-prefix cases,
case-insensitivity, surrounding whitespace, and the absent-Title
case."
```

---

## Task 3: Sanitize lyric lines via control-char strip (TDD)

**Files:**
- Modify: `crates/presenter-ui/src/utils/song_parser.rs`

- [ ] **Step 1: Add 2 failing tests inside `mod tests {}`**

Append inside the same `#[cfg(test)] mod tests` block:

```rust
    #[test]
    fn parse_song_text_strips_control_chars_from_lyric_lines() {
        // STX (\x02), BOM (\u{FEFF}), and ZWSP (\u{200B}) all stripped;
        // visible content survives.
        let input = "Verse 1\n\u{FEFF}lyric\u{200B} one\x02";
        let slides = parse_song_text(input);
        assert_eq!(slides.len(), 1);
        assert_eq!(slides[0].group.as_deref(), Some("Verse 1"));
        assert_eq!(slides[0].main, "lyric one");
    }

    #[test]
    fn parse_song_text_skips_lines_that_become_empty_after_strip() {
        // A line that is ONLY a control byte should not produce a slide
        // and should not flush an in-progress section.
        let input = "Verse 1\nlyric a\n\x02\nlyric b";
        let slides = parse_song_text(input);
        assert_eq!(slides.len(), 1, "stray \\x02 line should not flush");
        assert_eq!(slides[0].group.as_deref(), Some("Verse 1"));
        assert_eq!(slides[0].main, "lyric a\nlyric b");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui
cargo test --target x86_64-unknown-linux-gnu --lib song_parser::tests::parse_song_text_strips 2>&1 | tail -15
cargo test --target x86_64-unknown-linux-gnu --lib song_parser::tests::parse_song_text_skips 2>&1 | tail -15
```

Expected: both tests fail (current parser appends the raw line and lets the BOM/STX through).

- [ ] **Step 3: Add `sanitize_line` + `is_zero_width` helpers**

In `crates/presenter-ui/src/utils/song_parser.rs`, add at module scope (e.g. just below `is_metadata_line`):

```rust
/// Strip Unicode control chars (Cc category) and zero-width chars (BOM,
/// U+200B–200D) from a line. Used to clean lyric lines before pushing
/// them onto a slide so stray bytes from any pasted source (web pages,
/// Word, ProPresenter) don't render as a tofu/square character on stage.
fn sanitize_line(line: &str) -> String {
    line.chars()
        .filter(|c| !c.is_control() && !is_zero_width(*c))
        .collect()
}

fn is_zero_width(c: char) -> bool {
    matches!(c, '\u{FEFF}' | '\u{200B}' | '\u{200C}' | '\u{200D}')
}
```

- [ ] **Step 4: Replace the line-walk loop in `parse_song_text`**

Find the current `parse_song_text` function. Replace its `for raw_line in text.lines() { ... }` loop with:

```rust
    for raw_line in text.lines() {
        // Originally-blank lines (whitespace only) flush the current slide
        // — even if sanitization would later collapse a non-blank line to
        // empty, that's a different signal.
        if raw_line.trim().is_empty() {
            if current_group.is_some() || !current_main.is_empty() {
                flush_slide(&mut slides, &mut current_group, &mut current_main);
            }
            continue;
        }

        // Strip Unicode control + zero-width chars so stray bytes from any
        // pasted source don't pollute slides.
        let sanitized = sanitize_line(raw_line);
        let trimmed = sanitized.trim();

        // If sanitization left only whitespace, the line was entirely a
        // stray control byte — skip silently without flushing.
        if trimmed.is_empty() {
            continue;
        }

        if is_metadata_line(trimmed) {
            continue;
        }

        if is_group_line(trimmed) {
            flush_slide(&mut slides, &mut current_group, &mut current_main);
            current_group = Some(trimmed.to_string());
            continue;
        }

        current_main.push(sanitized);
    }
```

- [ ] **Step 5: Run all `song_parser` tests to verify nothing regressed**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui
cargo test --target x86_64-unknown-linux-gnu --lib song_parser 2>&1 | tail -25
```

Expected: 21 tests pass (19 from before + 2 new control-strip tests).

- [ ] **Step 6: Run clippy + fmt**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui
cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings 2>&1 | tail -5
cargo fmt
```

Expected: clippy clean.

- [ ] **Step 7: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
git add crates/presenter-ui/src/utils/song_parser.rs
git commit -m "feat(ui): strip control + zero-width chars from lyric lines (#275)

parse_song_text now sanitizes each lyric line via sanitize_line,
filtering Unicode Cc-category chars (NUL, STX, etc.) and zero-width
chars (BOM, U+200B-200D) before pushing onto the current slide.
A line that becomes empty after sanitization is skipped silently
without flushing the current section — so a stray \\x02 byte from
ProPresenter export no longer creates a tofu-square slide.

Originally-blank lines (whitespace-only in the source) still flush
as before.

2 new unit tests cover the strip and the no-flush-on-stripped-empty
behavior."
```

---

## Task 4: wrap_long_lines (TDD)

**Files:**
- Modify: `crates/presenter-ui/src/utils/song_parser.rs`

- [ ] **Step 1: Add 4 failing tests inside `mod tests {}`**

Append these tests:

```rust
    #[test]
    fn wrap_long_lines_word_wraps_at_limit() {
        let slide = SlideInput {
            main: "Ak Boh je za mňa,  kto je proti mne".to_string(),
            translation: None,
            stage: None,
            group: Some("Bridge".to_string()),
        };
        let out = wrap_long_lines(vec![slide], 32);
        assert_eq!(out.len(), 1);
        // Should wrap to >=2 lines, each ≤ 32 chars.
        let lines: Vec<&str> = out[0].main.split('\n').collect();
        assert!(lines.len() >= 2, "expected wrapping, got: {:?}", lines);
        for line in &lines {
            assert!(
                line.chars().count() <= 32,
                "wrapped line '{line}' exceeds 32 chars: {}",
                line.chars().count()
            );
        }
        // The original group should be preserved.
        assert_eq!(out[0].group.as_deref(), Some("Bridge"));
    }

    #[test]
    fn wrap_long_lines_passes_through_short_lines() {
        let slide = SlideInput {
            main: "short line\nanother short".to_string(),
            translation: None,
            stage: None,
            group: Some("Verse 1".to_string()),
        };
        let out = wrap_long_lines(vec![slide.clone()], 32);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].main, slide.main);
    }

    #[test]
    fn wrap_long_lines_hard_breaks_oversize_word() {
        // Single 50-char word at limit 32 → 2 pieces (32 + 18).
        let big_word: String = "a".repeat(50);
        let slide = SlideInput {
            main: big_word,
            translation: None,
            stage: None,
            group: None,
        };
        let out = wrap_long_lines(vec![slide], 32);
        let lines: Vec<&str> = out[0].main.split('\n').collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].chars().count(), 32);
        assert_eq!(lines[1].chars().count(), 18);
    }

    #[test]
    fn wrap_long_lines_counts_unicode_codepoints_not_bytes() {
        // Slovak diacritics — each char is one codepoint (multi-byte UTF-8).
        // 30 'á' chars = 30 codepoints = 60 bytes. At limit 32, this should
        // pass through (≤ limit by codepoint), not wrap.
        let line: String = "á".repeat(30);
        let slide = SlideInput {
            main: line.clone(),
            translation: None,
            stage: None,
            group: None,
        };
        let out = wrap_long_lines(vec![slide], 32);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].main, line, "30 Slovak codepoints fit at limit 32");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui
cargo test --target x86_64-unknown-linux-gnu --lib song_parser::tests::wrap_long_lines 2>&1 | tail -20
```

Expected: 4 failures with `cannot find function wrap_long_lines`.

- [ ] **Step 3: Implement `wrap_long_lines`, `wrap_one_line`, `hard_break_into`**

In `crates/presenter-ui/src/utils/song_parser.rs`, after `extract_title` / `pad_title_number` (or near the other public helpers — order is style preference), add:

```rust
/// Word-aware greedy wrapping per `\n`-separated line, so every line in
/// every slide's `main` fits within `line_limit` codepoints. Lines that
/// already fit pass through unchanged. Single words longer than
/// `line_limit` are hard-broken at the limit boundary. `line_limit == 0`
/// is treated as "no wrapping" (defensive — keeps signature stable if
/// the operator clears the setting).
pub fn wrap_long_lines(slides: Vec<SlideInput>, line_limit: usize) -> Vec<SlideInput> {
    if line_limit == 0 {
        return slides;
    }
    slides
        .into_iter()
        .map(|slide| {
            let wrapped: Vec<String> = slide
                .main
                .split('\n')
                .flat_map(|line| wrap_one_line(line, line_limit))
                .collect();
            SlideInput {
                main: wrapped.join("\n"),
                ..slide
            }
        })
        .collect()
}

fn wrap_one_line(line: &str, limit: usize) -> Vec<String> {
    if line.chars().count() <= limit {
        return vec![line.to_string()];
    }
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in line.split_whitespace() {
        let word_len = word.chars().count();
        if current.is_empty() {
            if word_len > limit {
                hard_break_into(&mut out, &mut current, word, limit);
            } else {
                current.push_str(word);
            }
        } else if current.chars().count() + 1 + word_len <= limit {
            current.push(' ');
            current.push_str(word);
        } else {
            out.push(std::mem::take(&mut current));
            if word_len > limit {
                hard_break_into(&mut out, &mut current, word, limit);
            } else {
                current.push_str(word);
            }
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// Hard-break an oversize word at codepoint boundaries. Pushes complete
/// `limit`-sized pieces to `out` and leaves the trailing remainder in
/// `current` so subsequent words can still accumulate onto the same line.
fn hard_break_into(out: &mut Vec<String>, current: &mut String, word: &str, limit: usize) {
    let chars: Vec<char> = word.chars().collect();
    let mut idx = 0;
    while chars.len() - idx > limit {
        let piece: String = chars[idx..idx + limit].iter().collect();
        out.push(piece);
        idx += limit;
    }
    let remainder: String = chars[idx..].iter().collect();
    *current = remainder;
}
```

- [ ] **Step 4: Verify `SlideInput` is `Clone`**

Open `crates/presenter-ui/src/api/presentations.rs`. The `SlideInput` struct should already derive `Debug, Clone, Serialize, Deserialize`. If `Clone` is missing (one test uses `slide.clone()` for the pass-through case), add it. Run:

```bash
grep -n "pub struct SlideInput" -A 4 crates/presenter-ui/src/api/presentations.rs
```

If `Clone` is not in the derive list, add it: `#[derive(Debug, Clone, Serialize, Deserialize)]`.

- [ ] **Step 5: Run tests**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui
cargo test --target x86_64-unknown-linux-gnu --lib song_parser 2>&1 | tail -25
```

Expected: 25 tests pass (21 from before + 4 new wrap tests).

- [ ] **Step 6: Run clippy + fmt**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui
cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings 2>&1 | tail -5
cargo fmt
```

Expected: clippy clean.

- [ ] **Step 7: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
git add crates/presenter-ui/src/utils/song_parser.rs crates/presenter-ui/src/api/presentations.rs
git commit -m "feat(ui): word-wrap long lyric lines at line_limit (#275)

wrap_long_lines applies greedy word-aware wrapping to each \\n-
separated line in every slide's main, so no line exceeds line_limit
codepoints. Words longer than line_limit are hard-broken at the
boundary; their remainders accumulate with subsequent words.
Counts use chars().count() so Slovak diacritics count as 1 each.

4 new unit tests cover normal word wrap, short-line passthrough,
oversize-word hard break, and Unicode codepoint counting."
```

---

## Task 5: chunk_to_two_lines (TDD)

**Files:**
- Modify: `crates/presenter-ui/src/utils/song_parser.rs`

- [ ] **Step 1: Add 4 failing tests inside `mod tests {}`**

```rust
    #[test]
    fn chunk_to_two_lines_splits_six_line_section() {
        let slide = SlideInput {
            main: "l1\nl2\nl3\nl4\nl5\nl6".to_string(),
            translation: None,
            stage: None,
            group: Some("Verse 1".to_string()),
        };
        let out = chunk_to_two_lines(vec![slide]);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].group.as_deref(), Some("Verse 1"));
        assert_eq!(out[0].main, "l1\nl2");
        assert_eq!(out[1].group, None, "second chunk inherits");
        assert_eq!(out[1].main, "l3\nl4");
        assert_eq!(out[2].group, None);
        assert_eq!(out[2].main, "l5\nl6");
    }

    #[test]
    fn chunk_to_two_lines_handles_odd_line_count() {
        let slide = SlideInput {
            main: "l1\nl2\nl3\nl4\nl5".to_string(),
            translation: None,
            stage: None,
            group: Some("Chorus".to_string()),
        };
        let out = chunk_to_two_lines(vec![slide]);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].group.as_deref(), Some("Chorus"));
        assert_eq!(out[0].main, "l1\nl2");
        assert_eq!(out[1].group, None);
        assert_eq!(out[1].main, "l3\nl4");
        assert_eq!(out[2].group, None);
        assert_eq!(out[2].main, "l5");
    }

    #[test]
    fn chunk_to_two_lines_passes_through_short_sections() {
        let two_line = SlideInput {
            main: "a\nb".to_string(),
            translation: None,
            stage: None,
            group: Some("Verse 1".to_string()),
        };
        let one_line = SlideInput {
            main: "single".to_string(),
            translation: None,
            stage: None,
            group: Some("Tag".to_string()),
        };
        let out = chunk_to_two_lines(vec![two_line.clone(), one_line.clone()]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].main, "a\nb");
        assert_eq!(out[0].group.as_deref(), Some("Verse 1"));
        assert_eq!(out[1].main, "single");
        assert_eq!(out[1].group.as_deref(), Some("Tag"));
    }

    #[test]
    fn chunk_to_two_lines_preserves_empty_interlude() {
        let slide = SlideInput {
            main: String::new(),
            translation: None,
            stage: None,
            group: Some("Interlude".to_string()),
        };
        let out = chunk_to_two_lines(vec![slide]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].main, "");
        assert_eq!(out[0].group.as_deref(), Some("Interlude"));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui
cargo test --target x86_64-unknown-linux-gnu --lib song_parser::tests::chunk_to_two_lines 2>&1 | tail -20
```

Expected: 4 failures with `cannot find function chunk_to_two_lines`.

- [ ] **Step 3: Implement `chunk_to_two_lines`**

In `crates/presenter-ui/src/utils/song_parser.rs`, add after `wrap_long_lines`:

```rust
/// Split each section's `main` into successive 2-line chunks, emitting
/// one slide per chunk. The first chunk inherits the original slide's
/// `group`; subsequent chunks have `group = None` (the operator UI
/// renders these as `--inherited` from the preceding effective group).
/// Slides with empty `main` (e.g. an `Interlude` heading with no body)
/// pass through unchanged.
pub fn chunk_to_two_lines(slides: Vec<SlideInput>) -> Vec<SlideInput> {
    let mut out = Vec::new();
    for slide in slides {
        if slide.main.is_empty() {
            out.push(slide);
            continue;
        }
        let lines: Vec<&str> = slide.main.split('\n').collect();
        if lines.len() <= 2 {
            out.push(slide);
            continue;
        }
        let mut first = true;
        for chunk in lines.chunks(2) {
            out.push(SlideInput {
                main: chunk.join("\n"),
                translation: None,
                stage: None,
                group: if first { slide.group.clone() } else { None },
            });
            first = false;
        }
    }
    out
}
```

- [ ] **Step 4: Run tests**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui
cargo test --target x86_64-unknown-linux-gnu --lib song_parser 2>&1 | tail -25
```

Expected: 29 tests pass (25 from before + 4 new chunk tests).

- [ ] **Step 5: Run clippy + fmt**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui
cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings 2>&1 | tail -5
cargo fmt
```

Expected: clippy clean.

- [ ] **Step 6: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
git add crates/presenter-ui/src/utils/song_parser.rs
git commit -m "feat(ui): chunk sections into 2-line slides (#275)

chunk_to_two_lines splits each section with >2 lines into 2-line
chunks. The first chunk keeps the original group label; subsequent
chunks have group=None so the operator UI renders them as inherited
from the preceding effective group (the existing slide_list.rs
inheritance rendering handles this automatically).

Empty Interlude slides (group present, main empty) pass through
unchanged.

4 new unit tests cover even and odd line counts, short-section
passthrough, and the empty-main case."
```

---

## Task 6: wrap_with_empty_bookends (TDD)

**Files:**
- Modify: `crates/presenter-ui/src/utils/song_parser.rs`

- [ ] **Step 1: Add 2 failing tests inside `mod tests {}`**

```rust
    #[test]
    fn wrap_with_empty_bookends_adds_front_and_back() {
        let slides = vec![
            SlideInput {
                main: "a".to_string(),
                translation: None,
                stage: None,
                group: Some("Verse 1".to_string()),
            },
            SlideInput {
                main: "b".to_string(),
                translation: None,
                stage: None,
                group: None,
            },
        ];
        let out = wrap_with_empty_bookends(slides);
        assert_eq!(out.len(), 4);
        // First and last are empty.
        assert_eq!(out[0].main, "");
        assert!(out[0].group.is_none());
        assert_eq!(out[3].main, "");
        assert!(out[3].group.is_none());
        // Middle slides preserved.
        assert_eq!(out[1].main, "a");
        assert_eq!(out[1].group.as_deref(), Some("Verse 1"));
        assert_eq!(out[2].main, "b");
    }

    #[test]
    fn wrap_with_empty_bookends_skips_empty_input() {
        let out = wrap_with_empty_bookends(vec![]);
        assert!(out.is_empty(), "no bookends on empty input");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui
cargo test --target x86_64-unknown-linux-gnu --lib song_parser::tests::wrap_with_empty_bookends 2>&1 | tail -15
```

Expected: 2 failures with `cannot find function wrap_with_empty_bookends`.

- [ ] **Step 3: Implement `wrap_with_empty_bookends`**

In `crates/presenter-ui/src/utils/song_parser.rs`, add after `chunk_to_two_lines`:

```rust
/// Prepend and append an empty `SlideInput` to the list, matching the
/// worship-service convention of starting and ending on a blank slide.
/// An empty input passes through unchanged so we don't emit two empty
/// slides for an empty paste.
pub fn wrap_with_empty_bookends(slides: Vec<SlideInput>) -> Vec<SlideInput> {
    if slides.is_empty() {
        return slides;
    }
    let empty = || SlideInput {
        main: String::new(),
        translation: None,
        stage: None,
        group: None,
    };
    let mut out = Vec::with_capacity(slides.len() + 2);
    out.push(empty());
    out.extend(slides);
    out.push(empty());
    out
}
```

- [ ] **Step 4: Run tests**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui
cargo test --target x86_64-unknown-linux-gnu --lib song_parser 2>&1 | tail -25
```

Expected: 31 tests pass.

- [ ] **Step 5: Run clippy + fmt**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui
cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings 2>&1 | tail -5
cargo fmt
```

Expected: clippy clean.

- [ ] **Step 6: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
git add crates/presenter-ui/src/utils/song_parser.rs
git commit -m "feat(ui): wrap slide list with empty bookends (#275)

wrap_with_empty_bookends prepends and appends an empty SlideInput
to a non-empty slide list, matching the worship-service convention
of starting and ending on a blank slide. An empty input passes
through unchanged so an empty paste does not produce two empty
slides.

2 new unit tests cover the wrap and the empty-input passthrough."
```

---

## Task 7: End-to-end pipeline regression test

**Files:**
- Modify: `crates/presenter-ui/src/utils/song_parser.rs`

- [ ] **Step 1: Add the end-to-end pipeline test**

Append inside `mod tests {}`:

```rust
    #[test]
    fn pipeline_user_76_arriba_paste_produces_correct_output() {
        // Regression guard for issue #275 follow-up. Exercises the full
        // pipeline (extract_title + parse + wrap_long_lines + chunk +
        // bookends) on the user's canonical spevnik export.
        let input = "Title: 76 Arriba\n\
                     Misc 1\n\
                     \n\
                     Verse 1\n\
                     Môj Boh je nekonečný\n\
                     Má všetku moc a je večný\n\
                     Jeho Duch vo mne je živý\n\
                     A všetko pre mňa učiní\n\
                     \n\
                     Pre-Chorus\n\
                     Žehná ma žehná\n\
                     Priazeñ nad priazeň\n\
                     A padá to na mňa, padá na mňa\n\
                     Žehná ma žehná\n\
                     Priazeň nad priazeň\n\
                     A tá ku mne prúdi, ku mne prúdi\n\
                     \n\
                     Chorus\n\
                     Jeden dva tri\n\
                     Zakrič On je najlepší\n\
                     Jeden dva tri\n\
                     Ja som v ňom požehnaný\n\
                     Jeden dva tri\n\
                     Všetka sláva Jemu, v Ňom\n\
                     Ja som tým, kým hovorí že som\n\
                     Jeden dva tri\n\
                     \n\
                     Verse 2\n\
                     Ja vidím veci na nebi\n\
                     Tie zmeny sú aj na zemi\n\
                     Jeho Duch vo mne je živý\n\
                     Nie je nič čo neučiní\n\
                     \n\
                     Bridge\n\
                     Ak Boh je za mňa,  kto je proti mne\n\
                     Ja chválim Ho, ja chválim Ho\n\
                     Misc 1\n";

        // Title extraction with 2-digit padding.
        let name = extract_title(input);
        assert_eq!(name.as_deref(), Some("076 Arriba"));

        // Run the full pipeline at line_limit 32.
        let slides = parse_song_text(input);
        let slides = wrap_long_lines(slides, 32);
        let slides = chunk_to_two_lines(slides);
        let slides = wrap_with_empty_bookends(slides);

        // Total: empty + 13 lyric chunks + empty = 15.
        assert_eq!(
            slides.len(),
            15,
            "expected 15 slides, got {}: {:#?}",
            slides.len(),
            slides
        );

        // Bookends.
        assert_eq!(slides[0].main, "");
        assert!(slides[0].group.is_none(), "first bookend has no group");
        assert_eq!(slides[14].main, "");
        assert!(slides[14].group.is_none(), "last bookend has no group");

        // First content slide is Verse 1, chunk 1.
        assert_eq!(slides[1].group.as_deref(), Some("Verse 1"));

        // Inherited groups: chunks 2+ of any section have group=None.
        assert_eq!(slides[2].group, None, "Verse 1 chunk 2 inherits");

        // Pre-Chorus first chunk is at index 3.
        assert_eq!(slides[3].group.as_deref(), Some("Pre-Chorus"));

        // Chorus first chunk is at index 6 (after Pre-Chorus's 3 chunks).
        assert_eq!(slides[6].group.as_deref(), Some("Chorus"));

        // Verse 2 first chunk is at index 10 (after Chorus's 4 chunks).
        assert_eq!(slides[10].group.as_deref(), Some("Verse 2"));

        // Bridge first chunk is at index 12 (after Verse 2's 2 chunks).
        // Bridge's first input line was 35 chars at limit 32, so it
        // wrapped — the chunked first slide may have either parts of
        // the wrapped first line OR the wrapped line + the second line.
        // Asserting its group + that all lines fit is enough.
        assert_eq!(slides[12].group.as_deref(), Some("Bridge"));

        // No content slide should contain Title:, Misc, ^B, or any line
        // longer than line_limit (32). Bookends excluded.
        for (idx, slide) in slides.iter().enumerate() {
            assert!(
                !slide.main.contains("Title:"),
                "slide {idx} leaked 'Title:': {:?}",
                slide.main
            );
            assert!(
                !slide.main.to_ascii_lowercase().contains("misc "),
                "slide {idx} leaked 'Misc': {:?}",
                slide.main
            );
            assert!(
                !slide.main.contains("^B"),
                "slide {idx} leaked '^B': {:?}",
                slide.main
            );
            for line in slide.main.split('\n') {
                assert!(
                    line.chars().count() <= 32,
                    "slide {idx} line '{line}' exceeds 32 chars (line_limit)"
                );
            }
        }
    }
```

- [ ] **Step 2: Run tests**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui
cargo test --target x86_64-unknown-linux-gnu --lib song_parser 2>&1 | tail -30
```

Expected: 32 tests pass (31 from before + 1 new pipeline test). If the pipeline test fails, read its output carefully — it lists actual slide count and contents in the failure message.

- [ ] **Step 3: Run clippy + fmt**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui
cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings 2>&1 | tail -5
cargo fmt
```

Expected: clippy clean.

- [ ] **Step 4: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
git add crates/presenter-ui/src/utils/song_parser.rs
git commit -m "test(ui): end-to-end pipeline regression for spevnik 76 Arriba paste (#275)

Asserts the full pipeline (extract_title + parse_song_text +
wrap_long_lines + chunk_to_two_lines + wrap_with_empty_bookends)
on the user's canonical spevnik export produces:

- Presentation name '076 Arriba' (76 padded to 076)
- 15 slides total (empty bookend + 13 lyric chunks + empty bookend)
- Verse 1 / Pre-Chorus / Chorus / Verse 2 / Bridge groups at the
  expected indices
- No metadata leakage (no 'Title:', no 'Misc', no '^B' in any slide)
- No line exceeds line_limit (32) in any slide"
```

---

## Task 8: Wire the pipeline into on_paste_confirm

**Files:**
- Modify: `crates/presenter-ui/src/components/presentation_modal.rs:230-292`

- [ ] **Step 1: Replace the `on_paste_confirm` body**

Open `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui/src/components/presentation_modal.rs`. Find the `let on_paste_confirm = { ... };` block (starts around line 230). Replace its inner closure body with:

```rust
    let on_paste_confirm = {
        let op = op.clone();
        move |_| {
            let lib_id = match ctx.selected_library_id.get_untracked() {
                Some(id) => id,
                None => return,
            };
            let text = op.paste_text.get_untracked();
            let line_limit = op.line_limit.get_untracked() as usize;

            // Title from the paste always wins; fallback to "Untitled" if
            // the paste has no Title: line (operator can rename via the
            // existing rename flow after creation).
            let name = crate::utils::song_parser::extract_title(&text)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "Untitled".to_string());

            // Run the full pipeline.
            let slides = crate::utils::song_parser::parse_song_text(&text);
            let slides = crate::utils::song_parser::wrap_long_lines(slides, line_limit);
            let slides = crate::utils::song_parser::chunk_to_two_lines(slides);
            let slides = crate::utils::song_parser::wrap_with_empty_bookends(slides);

            if slides.is_empty() {
                return;
            }
            op.submitting.set(true);

            let presentations = ctx.presentations;
            let libraries = ctx.libraries;
            let selected_pres_id = ctx.selected_presentation_id;
            let selected_pres = ctx.selected_presentation;
            let toast_message = ctx.toast_message;
            let toast_variant = ctx.toast_variant;
            let submitting = op.submitting;
            let open_modal = op.open_modal;
            let modal_target = op.modal_target_id;

            leptos::task::spawn_local(async move {
                match crate::api::presentations::create_from_paste(&lib_id, &name, slides).await {
                    Ok(resp) => {
                        let new_id = resp.presentation.id.to_string();
                        selected_pres_id.set(Some(new_id.clone()));
                        selected_pres.set(Some(resp.presentation));
                        crate::state::session::set("currentPresentationId", &new_id);

                        if let Some(summary) = resp.library_summary {
                            presentations.set(summary.presentations);
                        }
                        if let Ok(libs) = crate::api::libraries::list_libraries().await {
                            libraries.set(libs);
                        }

                        toast_variant.set("success".to_string());
                        toast_message.set(Some("Presentation created from paste".to_string()));
                    }
                    Err(e) => {
                        toast_variant.set("error".to_string());
                        toast_message.set(Some(format!("Error: {e}")));
                    }
                }

                submitting.set(false);
                open_modal.set(None);
                modal_target.set(None);
                if let Some(body) = crate::utils::window::document_body() {
                    body.dataset().delete("modalOpen");
                }
            });
        }
    };
```

The key changes vs the existing code:
- Title comes from `extract_title(&text)`, NOT from `create_name.get_untracked()`.
- After `parse_song_text`, the slides flow through `wrap_long_lines(.., line_limit)` → `chunk_to_two_lines` → `wrap_with_empty_bookends`.
- `op.line_limit.get_untracked() as usize` is read once at confirm time.

- [ ] **Step 2: Run a full UI build to check compilation**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui
cargo check --target wasm32-unknown-unknown 2>&1 | tail -10
```

Expected: clean compile (no errors).

- [ ] **Step 3: Run clippy + fmt**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui
cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings 2>&1 | tail -5
cargo fmt
```

Expected: clippy clean.

- [ ] **Step 4: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
git add crates/presenter-ui/src/components/presentation_modal.rs
git commit -m "feat(ui): wire song_parser pipeline into paste-confirm (#275)

on_paste_confirm now uses extract_title for the presentation name
(falls back to 'Untitled' if no Title:) and runs the slide pipeline
parse_song_text -> wrap_long_lines(line_limit) -> chunk_to_two_lines
-> wrap_with_empty_bookends. The operator's line_limit is read once
at confirm time. The modal's typed name is ignored on the paste
flow — Title: from the paste always wins."
```

---

## Task 9: Hide name input on the paste sub-screen

**Files:**
- Modify: `crates/presenter-ui/src/components/presentation_modal.rs:447-459`

- [ ] **Step 1: Wrap the name input's `<label>` with conditional `style:display`**

In `presentation_modal.rs`, find the `<label>` enclosing the `data-role="presentation-create-name"` input (around line 448-459). Replace it with:

```rust
                        <label style:display=move || if create_step.get() == "paste" { "none" } else { "block" }>
                            <span>"Presentation name"</span>
                            <input
                                type="text"
                                data-role="presentation-create-name"
                                autocomplete="off"
                                maxlength="160"
                                placeholder="New Presentation"
                                prop:value=move || create_name.get()
                                on:input=move |ev| create_name.set(event_target_value(&ev))
                            />
                        </label>
```

The only change is adding `style:display=move || if create_step.get() == "paste" { "none" } else { "block" }` to the `<label>`. Everything inside the label is unchanged.

- [ ] **Step 2: Run a full UI build**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui
cargo check --target wasm32-unknown-unknown 2>&1 | tail -10
```

Expected: clean compile.

- [ ] **Step 3: Run clippy + fmt**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui
cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings 2>&1 | tail -5
cargo fmt
```

Expected: clippy clean.

- [ ] **Step 4: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
git add crates/presenter-ui/src/components/presentation_modal.rs
git commit -m "feat(ui): hide presentation name input on paste sub-screen (#275)

When create_step == 'paste', the name input is hidden — the paste
flow gets the name from the Title: line in the paste. The input
stays visible for Blank and Import sub-flows where the user must
type a name."
```

---

## Task 10: Update E2E test to canonical spevnik paste

**Files:**
- Modify: `tests/e2e/wasm-presentation-crud.spec.ts:430` (the existing `paste with section headers splits one slide per section (#275)` test from PR #277)

- [ ] **Step 1: Replace the existing test body**

Open `/home/newlevel/devel/presenter/presenter-dev2/tests/e2e/wasm-presentation-crud.spec.ts`. Find the test starting at line ~430:

```typescript
  test("paste with section headers splits one slide per section (#275)", async ({
    page,
  }) => {
```

Replace the **entire `test(...)` block** (from `// Regression guard...` comment down through the closing `});`) with:

```typescript
  // Regression guard for issue #275 follow-up: full pipeline on a
  // spevnik song export must produce the right name + correctly
  // chunked slides + empty bookends.
  test("paste pipeline produces named presentation with bookends (#275)", async ({
    page,
  }) => {
    await selectLibrary(page);

    // Open the create modal.
    await page
      .locator('[data-view-panel="worship"] [data-role="presentation-create"]')
      .click();
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="presentation-create-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    // Click "Paste" to enter paste sub-screen.
    await page.locator('[data-role="presentation-create-paste"]').click();
    await expect(
      page.locator('[data-role="presentation-create-paste-area"]'),
    ).toBeVisible();

    // Name input must be hidden on the paste sub-screen.
    await expect(
      page.locator('[data-role="presentation-create-name"]'),
    ).toBeHidden();

    // Canonical spevnik song export — no leading blank line, Title:
    // populates the presentation name, Misc 1 lines are filtered, and
    // each section gets chunked to ≤ 2 lines with empty bookends.
    const userInput = `Title: 76 Arriba
Misc 1

Verse 1
Môj Boh je nekonečný
Má všetku moc a je večný
Jeho Duch vo mne je živý
A všetko pre mňa učiní

Pre-Chorus
Žehná ma žehná
Priazeñ nad priazeň
A padá to na mňa, padá na mňa
Žehná ma žehná
Priazeň nad priazeň
A tá ku mne prúdi, ku mne prúdi

Chorus
Jeden dva tri
Zakrič On je najlepší
Jeden dva tri
Ja som v ňom požehnaný
Jeden dva tri
Všetka sláva Jemu, v Ňom
Ja som tým, kým hovorí že som
Jeden dva tri

Verse 2
Ja vidím veci na nebi
Tie zmeny sú aj na zemi
Jeho Duch vo mne je živý
Nie je nič čo neučiní

Bridge
Ak Boh je za mňa,  kto je proti mne
Ja chválim Ho, ja chválim Ho
Misc 1`;

    await page
      .locator('[data-role="presentation-create-paste-text"]')
      .fill(userInput);
    await page
      .locator('[data-role="presentation-create-paste-confirm"]')
      .click();

    // Modal closes.
    await page.waitForFunction(
      () =>
        !document.querySelector(
          '[data-role="presentation-create-modal"][data-open="true"]',
        ),
      { timeout: 10_000 },
    );

    // Wait for the new presentation to render with all 15 slides.
    await page.waitForFunction(
      () => {
        const slides = document.querySelector(
          '[data-view-panel="worship"] [data-role="slides"]',
        );
        return (
          slides && slides.querySelectorAll("[data-slide-id]").length === 15
        );
      },
      { timeout: 15_000 },
    );

    // Read all slide group + main pairs.
    const slidePairs = await page.evaluate(() => {
      const slides = Array.from(
        document.querySelectorAll(
          '[data-view-panel="worship"] [data-role="slides"] [data-slide-id]',
        ),
      );
      return slides.map((slide) => {
        const groupEl = slide.querySelector('[data-role="slide-group"]');
        const mainEl = slide.querySelector('[data-field-display="main"]');
        return {
          group: (groupEl?.textContent || "").trim(),
          main: (mainEl?.textContent || "").trim(),
        };
      });
    });

    // 15 slides total — empty bookend + 13 lyric chunks + empty bookend.
    expect(slidePairs).toHaveLength(15);

    // Bookends are empty.
    expect(slidePairs[0].main).toBe("");
    expect(slidePairs[0].group).toBe("");
    expect(slidePairs[14].main).toBe("");
    expect(slidePairs[14].group).toBe("");

    // First chunk of each section carries the section header.
    expect(slidePairs[1].group).toBe("Verse 1");
    expect(slidePairs[3].group).toBe("Pre-Chorus");
    expect(slidePairs[6].group).toBe("Chorus");
    expect(slidePairs[10].group).toBe("Verse 2");
    expect(slidePairs[12].group).toBe("Bridge");

    // No slide should leak metadata.
    for (const slide of slidePairs) {
      expect(slide.main).not.toMatch(/Title:/);
      expect(slide.main).not.toMatch(/Misc 1/);
      expect(slide.main).not.toContain("^B");
    }

    // No content slide may contain a line longer than 32 characters
    // (the default line_limit). Bookends excluded (already asserted empty).
    for (let i = 1; i <= 13; i++) {
      const lines = slidePairs[i].main.split("\n");
      for (const line of lines) {
        // Use Array.from for accurate codepoint count of Slovak diacritics.
        const len = Array.from(line).length;
        expect(len).toBeLessThanOrEqual(
          32,
          `slide ${i} line "${line}" is ${len} chars (over 32)`,
        );
      }
    }

    // Verify the presentation name in the operator UI matches the padded
    // Title (076 Arriba). The name appears in the worship-panel header
    // for the selected presentation.
    const presName = await page
      .locator('[data-view-panel="worship"] [data-role="presentation-title"]')
      .textContent();
    expect(presName?.trim()).toBe("076 Arriba");
  });
```

Key new assertions vs the previous test:
- Name input is hidden on the paste sub-screen.
- Slide count is exactly **15** (was 3).
- Bookends at index 0 and 14 are empty.
- Section headers appear at the right indices (Verse 1 at 1, Pre-Chorus at 3, etc.).
- No content slide has a line over 32 chars.
- Presentation name in the operator UI is `"076 Arriba"`.

If the test selector for the presentation name (`[data-role="presentation-title"]`) doesn't exist in the codebase, find the actual selector:

```bash
grep -rn 'presentation-title\|presentation-name\|data-role="title"\|selected.*name' crates/presenter-ui/src/components/ | head -10
```

Use whatever role the operator's worship panel uses to render the active presentation's name.

- [ ] **Step 2: Verify the test compiles by running it locally against the live dev server**

The local dev server runs at `http://10.77.8.134:8080`. We point Playwright at it without spawning a new server (the build will be replaced after deploy):

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
PRESENTER_E2E_BASE_URL=http://10.77.8.134:8080 npx playwright test wasm-presentation-crud.spec.ts -g "paste pipeline produces" --reporter=line 2>&1 | tail -25
```

If the dev server's deployed binary is OLDER than the new pipeline (i.e., this task is run before Task 8 + 9 + a fresh trunk + cargo build), the test SHOULD fail (pre-deploy: 5 slides instead of 15). That's expected. The test passes after the full deploy.

- [ ] **Step 3: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
git add tests/e2e/wasm-presentation-crud.spec.ts
git commit -m "test(e2e): replace #275 paste test with full pipeline regression (#275)

Replaces the previous 'section headers splits one slide per section'
test with a full pipeline regression on the user's canonical spevnik
export. Asserts:

- Name input is hidden on the paste sub-screen
- Presentation name is '076 Arriba' (Title prefix padded to 3 digits)
- 15 slides total (empty bookend + 13 lyric chunks + empty bookend)
- Section headers (Verse 1, Pre-Chorus, Chorus, Verse 2, Bridge)
  appear at the expected indices
- No metadata (Title:, Misc 1, ^B) leaks into any slide
- No content slide contains a line over 32 chars (line_limit default)"
```

---

## Task 11: Local checks, push, monitor pipeline

- [ ] **Step 1: Final local sanity sweep**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
cargo fmt --all --check && echo "WORKSPACE FMT OK"
cd crates/presenter-ui && cargo fmt --check && echo "UI FMT OK" && cd ../..
cd crates/presenter-ui && cargo test --target x86_64-unknown-linux-gnu --lib song_parser 2>&1 | tail -15 && cd ../..
cd crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings 2>&1 | tail -5 && cd ../..
```

Expected: all 4 commands succeed. Total `song_parser` tests: 32 (10 pre-existing + 22 new).

If any fail, fix in ONE more commit before pushing.

- [ ] **Step 2: Push to origin/dev**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
git status -sb
git push origin dev
```

The push should be `Bypassed rule violations for refs/heads/dev` followed by the commit-range push. It auto-triggers Pipeline + PR Automation.

- [ ] **Step 3: Identify the new pipeline run and monitor**

```bash
gh run list --branch dev --limit 2 --json databaseId,name,status,headSha
```

Note the `databaseId` of the new `Pipeline` run for the latest pushed `headSha`.

Monitor with a single non-polling background sleep + check (per `~/devel/airuleset/modules/core/ci-monitoring.md` — DO NOT use `gh run watch`, DO NOT poll in tight loops):

```bash
sleep 600 && gh run view <DATABASE_ID> --json status,conclusion,jobs --jq '{status, conclusion, jobs: [.jobs[] | {name, status, conclusion}]}'
```

Run as a background task. Re-poll every ~6-12 minutes until `status == "completed"`.

- [ ] **Step 4: When pipeline completes, verify all 20 jobs are SUCCESS**

```bash
gh run view <DATABASE_ID> --json status,conclusion,jobs --jq '{status, conclusion, allSuccess: ([.jobs[] | .conclusion=="success"] | all)}'
```

Expected: `{"status":"completed","conclusion":"success","allSuccess":true}`. If any job failed:

```bash
gh run view <DATABASE_ID> --log-failed | tail -100
```

Fix in ONE commit, push, monitor again. Do NOT rerun-without-fix. Do NOT skip mutation testing.

- [ ] **Step 5: Verify dev deployment is live**

```bash
curl -s http://10.77.8.134:8080/healthz
```

Expected: `{"channel":"dev","status":"ok","version":"0.4.47"}`.

Then open the dashboard in Playwright (per `post-deploy-verification.md` — version label MUST come from the live DOM, not just curl):

```bash
# Use the Playwright MCP tools available in this session:
# 1. mcp__plugin_playwright_playwright__browser_navigate to http://10.77.8.134:8080/ui/operator
# 2. mcp__plugin_playwright_playwright__browser_evaluate to read the
#    version label from the DOM and confirm it's v0.4.47 (dev).
# 3. Use the parseSongText / extract_title bridges if useful for further
#    sanity checks; the E2E test in CI already proved end-to-end.
```

If everything is green and the dev frontend shows `v0.4.47 (dev)`, work is done.

- [ ] **Step 6: Confirm PR #277 is mergeable + clean**

```bash
gh api repos/zbynekdrlik/presenter/pulls/277 --jq '{mergeable, mergeable_state, head_sha: .head.sha}'
```

Expected: `{"mergeable":true,"mergeable_state":"clean", ...}`.

DO NOT merge. The user merges via explicit "merge it" instruction per `pr-merge-policy.md`.

- [ ] **Step 7: Send the completion report**

Use the EXACT template from `~/devel/airuleset/modules/core/completion-report.md`. Required lines:

```
## ✅ Work Complete

**Audits & deploy:**
✅ CI: pipeline <run-id> — 20/20 jobs green
✅ /plan-check: N/N fulfilled
✅ /review: clean — 0 🔴 0 🟡 0 🔵
✅ Deploy: dev frontend shows v0.4.47 (dev) (matches /healthz)

---

**Goal:** <one-sentence restatement of the user's ask in plain language>
**What changed:** <user-visible outcome, 1-2 sentences>

🌐 Dev:  http://10.77.8.134:8080/ui/operator
🌐 Prod: http://10.77.9.205/ui/operator

**[presenter] PR #277: <full PR title>**
https://github.com/zbynekdrlik/presenter/pull/277 — mergeable, clean
```

If `/plan-check` or `/review` (per `completion-report.md`'s pre-completion gate) returns ANY findings — 🔴, 🟡, or 🔵 inside this PR's diff — fix them in additional commits and re-run BEFORE sending the completion report. The 🔵 "deferred" loophole is closed.

---

## Verification Summary

| Check | How to verify |
|-------|---------------|
| Title extraction works | `extract_title("Title: 76 Arriba")` → `Some("076 Arriba")` |
| Padding rule | 1-3 digit prefix + space → 3-digit zero-pad; 4+ digits or no space → unchanged |
| Control char strip | `\x02`, BOM, ZWSP removed; line of just `\x02` skipped silently |
| Word wrap at line_limit | All wrapped lines ≤ line_limit codepoints; oversize words hard-broken |
| 2-line chunking | Sections > 2 lines split; first chunk keeps group, rest have None |
| Bookends | Non-empty input gets empty front + back; empty input stays empty |
| Modal name input visibility | Hidden when `create_step == "paste"`, visible otherwise |
| Pipeline regression test | 76 Arriba paste → name `"076 Arriba"`, 15 slides, no warnings, no metadata leakage |
| E2E green | `wasm-presentation-crud.spec.ts` `paste pipeline produces named presentation with bookends (#275)` passes in CI |
| All existing tests still green | 10 pre-existing song_parser tests, plus the existing wasm-edge-cases parseSongText E2E, all pass unchanged |
| CI green | All 20 pipeline jobs SUCCESS (incl. Mutation Testing, Coverage, all 3 E2E shards, Deploy to Dev) |
| Dev shows v0.4.47 | `/healthz` reports `0.4.47`; operator UI shows `v0.4.47 (dev)` in DOM |
