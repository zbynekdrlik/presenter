# Clipboard Import: Split Slides by Section Markers — Design

**Date:** 2026-04-29
**Status:** Proposed
**Scope:** Frontend (presenter-ui WASM), one new module + one tiny call-site update + tests

**Issue:** [#275 — clipboard import puts a lot of lines into one slide when it should be two](https://github.com/zbynekdrlik/presenter/issues/275)

## Goal

Make the operator's "create presentation from paste" feature split a pasted song into one slide per section (`Verse N`, `Chorus`, `Bridge`, `Outro`, `Interlude`, etc.), instead of only splitting on blank lines. Filter out ProPresenter-style metadata markers (`Title:`, `Misc N`, `^B`) so they don't pollute the slide content.

## Bug

`parse_song_text` (currently in `crates/presenter-ui/src/utils/test_helpers.rs:380-414` despite being production code) splits the input by `\n\n` and only treats the FIRST line of each blank-line-separated block as a possible group name.

The user's example input begins:
```
Title: 326 Všetko, čo v sebe držím
Misc 1
^B
Verse 1
[verse text…]

Chorus
[chorus text…]
```

The whole `Title: … Verse 1 [verse text]` block is one paragraph (no internal blank line), so the parser produces ONE slide whose `main` is the entire literal block including the `Title:`, `Misc 1`, `^B`, and the section heading `Verse 1`. The user expects each section heading to start a new slide.

## Approach

Rewrite as a line-walk parser. For each line in the input:

1. **Metadata line** → filter (skip silently). Recognized:
   - `Title:` prefix (case-insensitive)
   - `Misc <digits>` (case-insensitive, e.g. `Misc 1`, `MISC 12`)
   - `^<UPPER>` ProPresenter control marker (e.g. `^B`, `^A`, `^C`) — exactly one caret + one uppercase ASCII letter
2. **Group line** (the existing `is_group_line` list: Verse, Chorus, Bridge, Intro, Outro, Pre-chorus / Prechorus, Tag, Interlude, Refrain, Hook, Coda, Ending, Instrumental, with optional trailing number) → **flush the current slide** then start a new slide with this line as the group name.
3. **Blank line** → if the current slide has any content (group or main lines), **flush it**. Otherwise ignore.
4. **Anything else** → append the original line (preserving its leading whitespace) to the current slide's `main` lines.

After the loop, flush any remaining slide.

**Flush rule:** join collected `main` lines with `\n`, trim trailing whitespace. Skip the slide entirely only if both `group` is `None` AND the trimmed `main` is empty (defends against producing empty slides from leading filter-only lines or stacked blank lines). A slide with `group = Some(...)` and empty main IS kept (e.g., `Interlude` heading with no lyrics produces a "title-only" slide — matches ProPresenter behavior).

### Walk-through on the user's input

| Line | Trigger | Effect |
|------|---------|--------|
| `Title: 326 …` | metadata | skip |
| `Misc 1` | metadata | skip |
| `^B` | metadata | skip |
| `Verse 1` | group | flush (empty, skip) → start slide{group="Verse 1"} |
| `Všetko…` | text | append |
| `…porovnávať!` | text | append |
| `Ty ma chceš…` | text | append |
| (blank) | blank | flush slide{group="Verse 1", main="Všetko…\n…\nTy ma chceš…"} |
| `Chorus` | group | start slide{group="Chorus"} |
| `Mám Spasiteľa…` | text | append |
| … | … | … |
| `Outro` | group | flush prev, start slide{group="Outro"} |
| `Nikto mi Ťa…` | text | append |
| `Misc 1` | metadata | skip |
| `^B` | metadata | skip |
| EOF | flush | flush slide{group="Outro", main="Nikto…\nNikto…"} |

Result: 8 slides (Verse 1, Chorus, Verse 2, Chorus, Chorus, Interlude, Chorus, Chorus, Outro — matching the user's input structure).

## File structure

| File | Change |
|------|--------|
| `crates/presenter-ui/src/utils/song_parser.rs` | **NEW.** Public `parse_song_text(text: &str) -> Vec<SlideInput>` plus private helpers `is_group_line`, `is_metadata_line`. Includes `SlideInput` re-export or struct definition (whichever matches the existing pattern). |
| `crates/presenter-ui/src/utils/mod.rs` | Add `pub mod song_parser;` |
| `crates/presenter-ui/src/utils/test_helpers.rs` | Remove `parse_song_text` and `is_group_line`. Keep the rest. |
| `crates/presenter-ui/src/components/presentation_modal.rs` | Update line 244: `crate::utils::test_helpers::parse_song_text` → `crate::utils::song_parser::parse_song_text` |

`SlideInput` is currently defined in `crates/presenter-ui/src/api/presentations.rs`. The new `song_parser.rs` will import it from there (same as `test_helpers.rs` does today).

## Testing

Unit tests in `song_parser.rs` covering:

1. **User's exact pasted input** → produces 9 slides with the right groups and lyrics, no `Title:` / `Misc 1` / `^B` text in any slide. (Regression guard for the bug.)
2. **Mid-paragraph group split** — `Verse 1\nlyric\nChorus\nlyric` (no blank line between Verse and Chorus) → 2 slides.
3. **Metadata filtering** — `Title: foo\n\nVerse 1\nlyric` → 1 slide, no `Title:`.
4. **`^B` and `^A` control codes filtered**, but a literal `^` line (no letter) NOT filtered (just text).
5. **`Misc N` filtered**, but a line that starts with `Miscellaneous` (longer word) NOT filtered.
6. **Empty Interlude** — `Interlude\n\nChorus\nlyric` → 2 slides; first one has `group="Interlude", main=""`.
7. **Loose paragraphs without groups** — `Stanza 1\nline 2\n\nStanza 3\nline 4` → 2 slides, no group, preserves old behavior.
8. **Whitespace handling** — leading/trailing blanks, leading indentation on lyric lines (preserved within main).
9. **Empty input / whitespace-only input** → empty `Vec`.
10. **Group line case-insensitivity** — `chorus`, `CHORUS`, `Chorus 2` all detected.

## Risks / unknowns

- The existing `is_group_line` list might be incomplete for edge cases (e.g., songs with a `Bridge` numbered as `Bridge 2` — already handled by the trailing-digit-or-whitespace check in the current code). If the user reports another song that misparses, expand the list in a follow-up.
- `^B` / `^A` filtering uses the strict pattern `^[A-Z]` (caret + one ASCII uppercase). Lines like `^B12` (caret + uppercase + digits) are NOT filtered — they fall through as text. If real-world ProPresenter exports include such variants, we add them later.
- `parse_song_text` is currently re-exported via `__presenterOperatorTestHelpers` for E2E test access (test_helpers.rs:340-342 area). If any E2E test uses `window.__presenterOperatorTestHelpers.parse_song_text`, the move could break it — verify with grep before committing.

## Out of scope

- Multi-line group headers (e.g., `Verse 1 (Bridge variant)` — treated as a regular line if not caught by `is_group_line`).
- Auto-detection of "this is a song" vs "this is a sermon" — always uses the song-style parser. Other parsers stay unwritten unless asked.
- Improving the textarea UI in `presentation_modal.rs` (placeholder text, formatting hints). The bug is purely in the parser.
- Changing the server-side `create_from_paste` API.
