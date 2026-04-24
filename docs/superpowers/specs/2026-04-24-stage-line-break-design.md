# Stage Single-Line Auto-Break Design

## Goal

When a worship slide contains only one line of text and that line exceeds 26 characters, break it into two lines on the stage display so the autofit engine can render the text at a larger font size. Preserve musical phrasing by moving as few words as possible to the new second line (tail-break, not mid-break).

Also fix a related bug in the operator UI where the "line-too-long" warning counts bytes instead of characters, causing Slovak diacritic text (which uses 2-byte UTF-8 for `á č š ž` etc.) to trigger the warning prematurely.

## Motivation

The stage display renders lyrics for band singers on a black TV screen. Maximum legibility = maximum font size. The current rule is `white-space: pre` which preserves existing newlines but does not wrap. A 30-character single-line verse forces autofit to shrink the font so the whole line fits horizontally, wasting most of the vertical space. Splitting that verse into two lines lets autofit grow the font roughly 1.5–2×.

The operator UI independently flags lines "too long" with a red warning at 32 characters, so editors can proactively shorten lines. However the check uses `str::len()` (bytes). Slovak worship lyrics contain many diacritics; `"Nad všetkých"` is 12 visible chars but 14 bytes. The threshold fires at 28–30 visible chars instead of the intended 32. This creates false positives and confusion for the user.

## Scope

**In scope:**
- Auto-break rule in the `WorshipSnv` stage component (covers both `worship-snv` and `api` layouts — `api` falls through to `WorshipSnv` in `stage.rs:167-169`).
- Byte-count bug fix in `slide_list_utils.rs` (`format_multiline` and `field_has_warning`).

**Out of scope:**
- Other layouts (`worship-pp`, `bible`, `preach`, `timer`, `ndi-fullscreen`). `worship-pp` could adopt the rule later if needed.
- Changing the operator UI threshold value (stays at 32). Only the counting unit changes (bytes → chars).
- Multi-line slides (no change — already split by the song editor).
- Slides that have the longest line >26 chars but also contain `\n` — respect the author's existing line breaks.

## Algorithm: latest-space-under-threshold

Pseudo-code for `break_if_long(text: &str, threshold: usize = 26) -> Cow<str>`:

```
if text contains '\n':
    return text unchanged          # author already split it
if text.chars().count() <= threshold:
    return text unchanged          # short enough
find the rightmost space index `i` such that
    text[..i].chars().count() <= threshold
if no such space exists:
    return text unchanged          # cannot split nicely; let autofit shrink
return text[..i] + '\n' + text[i+1..]
```

**Key properties:**
- Character-based, not byte-based. Diacritics count as one unit.
- Tail-break bias: picks the rightmost valid space, so line 1 is as long as possible and line 2 contains only the last word or two.
- Idempotent on already-split input (contains `\n` → returned unchanged).
- Safe on pathological inputs: single very long word, empty string, only whitespace.

**Worked examples** (threshold = 26 chars):

| Input | Output (visual) | Reason |
|---|---|---|
| `Nad všetkých vyvyšený bude Pán` (30) | `Nad všetkých vyvyšený bude` / `Pán` | Rightmost space before char 26 is after "bude". Moves 1 word. |
| `Pán je môj pastier som v bezpečí` (32) | `Pán je môj pastier som v` / `bezpečí` | Rightmost space fitting ≤ 26. Moves 1 word. |
| `Ježiš je Pán` (12) | `Ježiš je Pán` | ≤ 26 threshold. No change. |
| `Halleluja\nAmen` | `Halleluja\nAmen` | Already multi-line. No change. |
| `Supercalifragilisticexpialidocious` (34, no spaces) | unchanged | No valid split point. Autofit handles it. |
| `Some Verse Here` (15) | `Some Verse Here` | ≤ 26 threshold. No change. |

## Integration point

In `crates/presenter-ui/src/components/stage/worship_snv.rs`, the `current_text` and `next_text` closures return `String`. Wrap the output through `break_if_long`:

```rust
let current_text = move || break_if_long(&raw_current_text());
let next_text = move || break_if_long(&raw_next_text());
```

The autofit effect re-runs whenever the signal produces a new value; no further wiring is needed. CSS stays `white-space: pre` — we inject the newline, the CSS renders it.

## Operator UI byte→char fix

In `crates/presenter-ui/src/components/slide_list_utils.rs`:

```rust
// Before
if limit > 0 && line.len() as u32 > limit { ... }

// After
if limit > 0 && line.chars().count() as u32 > limit { ... }
```

Applies to both `format_multiline` (line 8) and `field_has_warning` (line 26). The CSS threshold (`--operator-line-limit-ch: 32` in `operator.css:37`) stays unchanged.

## Testing

### Unit tests (Rust, non-WASM)

New file or existing test module:
- `break_if_long` — threshold boundary (26, 27 chars), single long word (no spaces), already multi-line input, Slovak diacritics (`chars().count()` vs `len()`), empty string.
- `field_has_warning` — 25-char diacritic string at 26-byte length must NOT warn at limit=32; 33-char ASCII must warn.
- `format_multiline` — diacritic string under limit does NOT get wrapped in `operator__slide-overflow`.

### E2E tests (Playwright)

- **Stage split test** — trigger a single-line slide with 28 Slovak characters; assert the rendered `.stage__slide-text` element contains `\n` (via `textContent`) or has two visual lines (`clientHeight` / `scrollHeight` comparison).
- **Stage no-split test** — trigger a slide ≤ 26 chars; assert no newline was injected.
- **Operator UI warning test** — render a slide with a 28-char Slovak line containing `á č š ž`; assert the line does NOT get the `operator__slide-overflow` class. Render a 33-char ASCII line; assert it DOES get the class.

## Risk & Rollback

- Risk: algorithm inserts a `\n` where the user didn't want one. Mitigation: only triggers on single-line slides >26 chars; author can always add their own line break in the song source to override.
- Risk: byte→char change in operator UI causes a slide that was previously flagged to no longer be flagged (or vice versa). This is the intended fix, but worth calling out in the PR description.
- Rollback: revert two commits (auto-break + char-count fix). No database or persisted-data changes.

## Non-goals

- No new threshold for operator UI; no new config key.
- No change to `worship-pp` layout.
- No change to autofit itself.
- No change to `white-space` CSS rule.
