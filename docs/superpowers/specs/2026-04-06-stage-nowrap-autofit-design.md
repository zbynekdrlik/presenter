# Stage SNV: Prevent Line Wrapping in Slide Text

**Date:** 2026-04-06
**Status:** Approved

## Problem

On the Stage SNV display, slide text with long lines wraps within the container, creating more visual lines than intended. For example, a slide with 2 logical lines (one `\n`) renders as 3 visual lines because the first line is too wide at the auto-fitted font size and word-wraps.

**Root cause:** CSS `white-space: pre-wrap` on `.stage__slide-text` preserves newlines but also allows word-wrapping. The autofit algorithm shrinks font size until vertical overflow stops, but a wrapped line consumes extra vertical space, resulting in smaller font AND more lines than necessary.

## Solution

Change `white-space: pre-wrap` to `white-space: pre` on `.stage__slide-text`.

**Effect:**
- `\n` line breaks are preserved (each logical line stays on its own visual line)
- No word-wrapping occurs — each line is a single visual line
- The existing autofit binary search already checks `scroll_width > client_width` (in `is_overflowing()`), so it will shrink font size until the widest line fits the container width

## Files Changed

- `crates/presenter-ui/styles/stage.css` line 163: `white-space: pre-wrap` → `white-space: pre`

## Verification

1. Current slide "obri padnú, hradby predo mnou / sa zrútia" should render as exactly 2 lines
2. Next slide "V takú istú moc verím / čo kameň z hrobu odvalí" should remain 2 lines
3. Font size auto-adjusts so the widest line fills the container width
4. No visual regression on other stage layouts
