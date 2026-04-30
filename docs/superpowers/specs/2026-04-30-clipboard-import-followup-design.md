# Clipboard Import Follow-Up: Title, Wrap, 2-Line Chunking, Bookends, Control-Char Strip — Design

**Date:** 2026-04-30
**Status:** Proposed
**Scope:** Frontend (presenter-ui WASM) — extend `song_parser.rs`, update `presentation_modal.rs`, update one E2E test
**Builds on:** PR #277 (issue #275 — section-aware splitting)

## Goal

Make the operator's "create presentation from paste" flow do the right thing for spevnik song exports:

1. Use the pasted `Title:` line as the presentation **name** (always wins; modal name input hidden in the paste sub-screen; fallback `"Untitled"` if no `Title:`). Pad a leading numeric prefix in the title to 3 digits with leading zeros (e.g., `76 Arriba` → `076 Arriba`) so songs sort correctly in the operator's library list.
2. **Word-wrap** any lyric line longer than the operator's `line_limit` (default 32 chars) so no slide triggers the "Line exceeds N characters" warning.
3. Chunk each section's lyrics into slides of **at most 2 lines** each.
4. Strip Unicode control characters (`Cc`) and zero-width characters from every line so stray bytes don't pollute slides.
5. Wrap the resulting slide list with **empty bookend slides** at start and end, matching worship-service convention.

## Canonical input — spevnik export

```
Title: 76 Arriba
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
Misc 1
```

Expected output: presentation name `"076 Arriba"` (numeric prefix padded to 3 digits), **15 slides** total — empty bookend, then 13 lyric slides each with ≤ 2 lines and no line exceeding `line_limit`, then empty bookend. Every slide displays without the `!` warning superscript.

## Architecture

Pipeline of small composable functions in `crates/presenter-ui/src/utils/song_parser.rs`. The existing `parse_song_text` from PR #277 stays focused on section-splitting. New helpers handle title extraction, line-wrapping, 2-line chunking, and bookends. Each piece is independently testable.

```
clipboard text
   │
   ├─► extract_title()              → Option<String>   (presentation name)
   │
   └─► parse_song_text()            → Vec<SlideInput>  (section-split, control-char-clean)
         │
         └─► wrap_long_lines(.., line_limit) → Vec<SlideInput>  (each line ≤ line_limit, word-aware)
               │
               └─► chunk_to_two_lines() → Vec<SlideInput>  (each slide ≤ 2 lines)
                     │
                     └─► wrap_with_empty_bookends() → Vec<SlideInput>  (empty front+back)
```

The modal calls these functions in `on_paste_confirm`, reads `line_limit` from `OperatorState`, and submits the result via the existing `create_from_paste` API. No server changes.

## Components

### `extract_title(text: &str) -> Option<String>`

Scans lines top-to-bottom, returns the trimmed content after the first case-insensitive `Title:` prefix, with the leading numeric prefix padded to 3 digits via `pad_title_number`. Returns `None` if no `Title:` line is found.

```rust
pub fn extract_title(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let trimmed = line.trim();
        let lower = trimmed.to_ascii_lowercase();
        if !lower.starts_with("title:") {
            return None;
        }
        let raw = trimmed[6..].trim(); // 6 = len("title:")
        Some(pad_title_number(raw))
    })
}

/// Pad a leading 1-3 digit numeric prefix (followed by whitespace) to 3
/// digits with leading zeros. Leaves 4+ digit prefixes and titles without
/// a leading number unchanged. Examples:
///   "76 Arriba"          → "076 Arriba"
///   "6 Foo"              → "006 Foo"
///   "102 10000 Armád"    → "102 10000 Armád"  (already 3 digits)
///   "1234 Bar"           → "1234 Bar"          (4+ digits, untouched)
///   "Arriba"             → "Arriba"            (no leading number)
///   "76Arriba"           → "76Arriba"          (no whitespace separator)
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

Edge cases:
- `Title:` with empty content → `Some("")`. Caller treats `Some("")` like `None` (fallback to `"Untitled"`).
- Multiple `Title:` lines — first wins (rare in practice).
- Padding only applies when the digits are followed by whitespace (so `76Arriba` is left unchanged because we cannot tell which characters are the number vs the name).

### `parse_song_text` — modified for control-char strip

Existing function from PR #277 stays the same. **One change:** when appending a non-empty, non-group, non-metadata line to `current_main`, sanitize it first by stripping Unicode control chars (`char::is_control`) and zero-width chars (BOM `U+FEFF`, `U+200B`–`U+200D`).

```rust
fn sanitize_line(line: &str) -> String {
    line.chars()
        .filter(|c| !c.is_control() && !is_zero_width(*c))
        .collect()
}

fn is_zero_width(c: char) -> bool {
    matches!(c, '\u{FEFF}' | '\u{200B}' | '\u{200C}' | '\u{200D}')
}
```

If sanitization leaves the line empty/whitespace-only, skip pushing it to `current_main`.

### `wrap_long_lines(slides: Vec<SlideInput>, line_limit: usize) -> Vec<SlideInput>`

For each slide, for each `\n`-separated line in `main`, word-wrap it so every wrapped piece is at most `line_limit` characters (counted by `chars().count()` for Unicode safety). Emits the wrapped pieces in order, joined with `\n`. Lines that already fit pass through unchanged.

Word-wrap rules:
- Greedy: accumulate words separated by whitespace until adding the next word would exceed `line_limit`; emit current piece, start a new one.
- Single word longer than `line_limit` (rare for lyrics) is hard-broken at the limit.
- Lines that fit (`chars().count() <= line_limit`) pass through unchanged with original whitespace preserved.
- `line_limit == 0` (defensive) → pass through unchanged.

Sketch:

```rust
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
                hard_break_into(&mut out, word, limit);
            } else {
                current.push_str(word);
            }
        } else if current.chars().count() + 1 + word_len <= limit {
            current.push(' ');
            current.push_str(word);
        } else {
            out.push(std::mem::take(&mut current));
            if word_len > limit {
                hard_break_into(&mut out, word, limit);
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
```

`split_whitespace` collapses runs of whitespace — this is acceptable for lyrics. The user's `"Ak Boh je za mňa,  kto je proti mne"` (35 chars, double space) wraps to `"Ak Boh je za mňa, kto je"` (24 chars) + `"proti mne"` (9 chars). Bridge slide goes from 1 long line to 2 short lines.

### `chunk_to_two_lines(slides: Vec<SlideInput>) -> Vec<SlideInput>`

For each input slide:
- **`main` is empty** (e.g., empty Interlude): pass through unchanged.
- **≤ 2 lines in `main`**: pass through unchanged.
- **> 2 lines**: split into successive 2-line chunks.
  - First chunk inherits the original `group`.
  - Subsequent chunks have `group = None` (slide_list.rs already renders these with `--inherited` style based on the previous slide's effective group).
  - The last chunk has 1 line if the original line count was odd.

Lines are joined with `\n` per chunk.

```rust
pub fn chunk_to_two_lines(slides: Vec<SlideInput>) -> Vec<SlideInput> {
    let mut out = Vec::new();
    for slide in slides {
        let lines: Vec<&str> = slide.main.split('\n').collect();
        let line_count = if slide.main.is_empty() { 0 } else { lines.len() };
        if line_count <= 2 {
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

### `wrap_with_empty_bookends(slides: Vec<SlideInput>) -> Vec<SlideInput>`

If empty input, return empty (no bookends-on-nothing). Otherwise prepend and append `SlideInput { main: "", translation: None, stage: None, group: None }`.

```rust
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

### Modal call site (`presentation_modal.rs:230-292`)

Replace the `on_paste_confirm` body. The modal already has `op` (OperatorState) in scope, so `op.line_limit.get_untracked()` is the source of truth.

```rust
let text = op.paste_text.get_untracked();
let line_limit = op.line_limit.get_untracked() as usize;

let name = song_parser::extract_title(&text)
    .filter(|s| !s.is_empty())
    .unwrap_or_else(|| "Untitled".to_string());

let slides = song_parser::parse_song_text(&text);
let slides = song_parser::wrap_long_lines(slides, line_limit);
let slides = song_parser::chunk_to_two_lines(slides);
let slides = song_parser::wrap_with_empty_bookends(slides);

if slides.is_empty() {
    return;
}
// submit (library_id, name, slides) via create_from_paste
```

The modal's typed `create_name` is **ignored** for the paste flow.

### Modal UX — hide name input on paste sub-screen (Option B confirmed)

The shared `data-role="presentation-create-name"` input at the top of the modal stays visible for the **Blank** and **Import** sub-flows (both still need a typed name) but **hides** when the **Paste** sub-screen (`presentation-create-paste-area`) is active.

Implementation: wrap the name input's parent `<label>`/`<div>` in a Leptos conditional that returns the element only when `op.modal_target_id` (or whichever signal already drives sub-flow visibility) is NOT in paste mode. The current modal already tracks sub-flow state via `op.paste_text`-and-friends signals; reuse that.

The presentation rename flow is unchanged — the operator can rename via the existing `presentation-edit-modal` after creation.

### JS bridge & E2E compatibility

- `__presenterOperatorTestHelpers.parseSongText(text)` keeps its existing signature returning `Vec<SlideInput>` (just the section-split result) so the existing regression test in `tests/e2e/wasm-edge-cases.spec.ts:259` still passes.
- The full pipeline (title + wrap + chunk + bookends) runs only in the modal's `on_paste_confirm`. The E2E test asserts the user-visible end result via the operator UI, not via the bridge.

## Worked example — "76 Arriba"

After `extract_title`: name = `"076 Arriba"` (the `76` is padded to `076`).

After `parse_song_text` (5 sections): Verse 1 (4 lines), Pre-Chorus (6 lines), Chorus (8 lines), Verse 2 (4 lines), Bridge (2 lines, first is 35 chars).

After `wrap_long_lines(.., 32)`: Bridge's first line splits into `"Ak Boh je za mňa, kto je"` + `"proti mne"`. Bridge now has 3 lines. All other sections unchanged.

After `chunk_to_two_lines`:

| Slide | group | content |
|-------|-------|---------|
| 1 | Verse 1 | "Môj Boh je nekonečný" + "Má všetku moc a je večný" |
| 2 | (none, inherits Verse 1) | "Jeho Duch vo mne je živý" + "A všetko pre mňa učiní" |
| 3 | Pre-Chorus | "Žehná ma žehná" + "Priazeñ nad priazeň" |
| 4 | (inherited) | "A padá to na mňa, padá na mňa" + "Žehná ma žehná" |
| 5 | (inherited) | "Priazeň nad priazeň" + "A tá ku mne prúdi, ku mne prúdi" |
| 6 | Chorus | "Jeden dva tri" + "Zakrič On je najlepší" |
| 7 | (inherited) | "Jeden dva tri" + "Ja som v ňom požehnaný" |
| 8 | (inherited) | "Jeden dva tri" + "Všetka sláva Jemu, v Ňom" |
| 9 | (inherited) | "Ja som tým, kým hovorí že som" + "Jeden dva tri" |
| 10 | Verse 2 | "Ja vidím veci na nebi" + "Tie zmeny sú aj na zemi" |
| 11 | (inherited) | "Jeho Duch vo mne je živý" + "Nie je nič čo neučiní" |
| 12 | Bridge | "Ak Boh je za mňa, kto je" + "proti mne" |
| 13 | (inherited) | "Ja chválim Ho, ja chválim Ho" |

After `wrap_with_empty_bookends`: 15 slides — empty + the 13 above + empty.

Operator card UI numbers them 1..15. No `!` warning superscript anywhere because every line fits within `line_limit`.

## Testing

### Unit tests in `song_parser.rs` (new — added on top of the 10 existing)

| # | Test | What it verifies |
|---|------|------------------|
| 1 | `extract_title_pads_two_digit_prefix` | `Title: 76 Arriba` → `Some("076 Arriba")` |
| 2 | `extract_title_pads_one_digit_prefix` | `Title: 6 Foo` → `Some("006 Foo")` |
| 3 | `extract_title_leaves_three_digit_prefix_unchanged` | `Title: 102 10000 Armád` → `Some("102 10000 Armád")` |
| 4 | `extract_title_leaves_four_digit_prefix_unchanged` | `Title: 1234 Bar` → `Some("1234 Bar")` |
| 5 | `extract_title_no_leading_number_unchanged` | `Title: Arriba` → `Some("Arriba")` |
| 6 | `extract_title_no_space_after_digits_unchanged` | `Title: 76Arriba` → `Some("76Arriba")` |
| 7 | `extract_title_returns_none_when_absent` | Input with no `Title:` → `None` |
| 8 | `extract_title_is_case_insensitive` | `TITLE: foo` → `Some("foo")` |
| 9 | `extract_title_strips_surrounding_whitespace` | `Title:   foo  ` → `Some("foo")` |
| 10 | `wrap_long_lines_word_wraps_at_limit` | "Ak Boh je za mňa, kto je proti mne" (35) at 32 → 2 lines, both ≤ 32 |
| 11 | `wrap_long_lines_passes_through_short_lines` | All lines ≤ limit → unchanged |
| 12 | `wrap_long_lines_hard_breaks_oversize_word` | Single 50-char word at 32 → 2 lines (32 + 18) |
| 13 | `wrap_long_lines_preserves_unicode_chars` | Slovak diacritics counted as 1 char (`chars().count()`) |
| 14 | `chunk_to_two_lines_splits_six_line_section` | 6 lines → 3 slides; first has group, rest have `None` |
| 15 | `chunk_to_two_lines_handles_odd_line_count` | 5 lines → 3 slides; last has 1 line |
| 16 | `chunk_to_two_lines_passes_through_short_sections` | 2-line and 1-line sections unchanged |
| 17 | `chunk_to_two_lines_preserves_empty_interlude` | `group="Interlude", main=""` slide unchanged |
| 18 | `wrap_with_empty_bookends_adds_front_and_back` | N input → N+2 |
| 19 | `wrap_with_empty_bookends_skips_empty_input` | Empty input → empty output |
| 20 | `parse_song_text_strips_control_chars` | Line containing `\x02` (STX), BOM, ZWSP → cleaned |
| 21 | `parse_song_text_skips_lines_that_become_empty_after_strip` | Line of just `\x02` → no slide |
| 22 | **End-to-end pipeline regression: user's `76 Arriba` paste** → name `"076 Arriba"` (padded), 15 slides matching the worked example exactly (groups, line counts, bookends) |

### E2E test update — `tests/e2e/wasm-presentation-crud.spec.ts:430`

Replace the test body with the canonical `76 Arriba` paste. Updated assertions:

- The name input is hidden when the paste sub-screen is active (`expect(page.locator('[data-role="presentation-create-name"]')).toBeHidden()`).
- After confirm: presentation name in the operator UI is `"076 Arriba"` (the leading `76` is padded to `076`).
- Slide count is **15**.
- First and last slides have empty `main` (bookends).
- Slide 12's main is exactly `"Ak Boh je za mňa, kto je\nproti mne"` (proves wrap_long_lines worked).
- No slide's `main` contains `Title:`, `Misc`, control chars, or more than one `\n`.
- No slide card shows the `!` warning superscript (count of `[data-role="slide-warning"]` visible elements is 0).
- Console clean — existing `consoleMessages` collection still passes.

### Console-clean assertion remains in place

The existing `consoleMessages` collection in the E2E suite still asserts zero browser console errors/warnings.

## Risks / unknowns

- **Slide count multiplication.** Typical worship song with 4-5 sections × 4-6 lines becomes ~12-15 slides + bookends. The operator UI already handles long slide lists.
- **Greedy wrapping is not optimal**. A 35-char line wraps to 24 + 9 instead of an optimal 17 + 17. Acceptable for worship lyrics — operators can manually reflow if needed.
- **`line_limit` change after paste**. If the operator changes `line_limit` after creating a presentation, slides won't auto-rewrap. Same behavior as today's manually-edited slides — out of scope.
- **The "square character" report**. Couldn't reproduce with the canonical paste, but defensive control-char stripping covers any source-side leakage (BOM, NBSP, raw control bytes).

## Out of scope

- Server `create_from_paste` API changes. The endpoint accepts the slide list and presentation name; both are sufficient.
- Auto-detection of paste source (ProPresenter vs spevnik vs Word). Filters are noop for unmatched lines.
- Removing the modal name input from the Blank or Import sub-screens.
- Re-wrapping existing presentations when `line_limit` changes.
