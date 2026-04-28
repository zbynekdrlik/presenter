# Worship-PP Bigger Sidebar Fonts — Design

**Date:** 2026-04-28
**Status:** Proposed
**Scope:** CSS only

## Goal

Make the worship-pp stage playlist sidebar text large enough that ~12 characters fit per row at projector distance. The 22%-wide sidebar is the right width — but the current 2.6vh font (≈28px @ 1080p) is too small, and the sidebar/entry padding eats too much width. Increase the font, tighten the padding.

## Non-goals

- No change to sidebar width (22% is correct).
- No change to slides-area width (78% is correct).
- No change to active-highlight color or accent bar (only the padding-left compensation adjusts to match the new entry padding).
- No layout / WASM / server changes.

## Approach

CSS-only updates to four rules in `crates/presenter-ui/styles/stage.css`. Width math at 1080p:

| | px @ 1080p (1920w × 1080h) |
|---|---|
| Viewport width | 1920 |
| Sidebar width (22%) | 422 |
| Sidebar padding both sides (now `0.4% 0.5%` → 19.2px L+R) | minus 19 → 403 inner |
| Entry padding both sides (now `0.4rem` → 12.8px L+R) | minus 13 → 390 chars-area |
| Per char target (12 chars per row) | ~32px |
| Font-size that yields ~32px char-width (typical ratio 0.55–0.6) | ~54px ≈ **5vh** |

Row height with `5vh` font + `0.3vh` padding × 2 + line-height 1.1 ≈ `5.5vh + 0.6vh = 6.1vh` per row → 12 rows ≈ **73vh** in a 92vh sidebar. Plenty of headroom; >12 entries still scroll via existing `overflow-y: auto`.

### Concrete CSS changes

`crates/presenter-ui/styles/stage.css`:

`.stage-pp__playlist-sidebar` — tighten padding:

```css
.stage-pp__playlist-sidebar {
    position: absolute;
    right: 0;
    top: 0;
    width: 22%;
    height: 92%;
    overflow-y: auto;
    border-left: 1px solid rgba(255, 255, 255, 0.1);
    padding: 0.4% 0.5%;     /* was 1% 1.5% */
    box-sizing: border-box;
}
```

`.stage-pp__playlist-entry` — bigger font, tighter padding:

```css
.stage-pp__playlist-entry {
    padding: 0.3vh 0.4rem;          /* was 0.6vh 0.8rem */
    color: #94a3b8;
    /* Sized so ~12 typical chars fit per row at 1080p:
       sidebar 22% × 1920 = 422px, less padding → ~390px content,
       at 5vh font (≈54px) char-width is ~30px, ~12 chars per row. */
    font-size: 5vh;                  /* was 2.6vh */
    line-height: 1.1;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    border-radius: 4px;
    margin-bottom: 2px;
}
```

`.stage-pp__playlist-entry--active` — recompensate padding-left for new base:

```css
.stage-pp__playlist-entry--active {
    background: #38bdf8;
    color: #0f172a;
    font-weight: 700;
    border-left: 4px solid #0ea5e9;
    /* Base padding-left is 0.4rem (≈6.4px); subtract the 4px border so
       text alignment with non-active rows stays consistent. */
    padding-left: calc(0.4rem - 4px);
}
```

`.stage-pp__slides-area` — unchanged (already 78%).

## Testing

Update one assertion in `tests/e2e/stage-worship-pp.spec.ts` — the existing test "sidebar is narrower (~22%) and entries have projector-readable font" asserts `entryFontSizePx >= 24`. Tighten to `>= 40` since the new 5vh on the test's `1920×1080` viewport = 54px, and we want the regression guard to catch any accidental drop back to a small font.

No new E2E spec needed.

## Risks

- Slovak diacritics widen text slightly. With the 1080p target ~30px char-width and a 5vh font, very wide characters (`Ž`, `Š`, `Č`) may push to ~10 chars instead of 12 in the worst case. Existing `text-overflow: ellipsis` truncates cleanly. The user's "12 chars" was approximate — the design hits 11–12 typical, 9–10 in worst-case Slavic strings. Acceptable.
- 5vh on a non-16:9 stage display (e.g. tall portrait) would render smaller in absolute pixels. The presenter targets 1080p projector, so this is fine.
- Tight 0.4% sidebar padding leaves only ~7px of breathing room from the screen edge at 1080p. Text won't TOUCH the edge but is close. The user explicitly chose this option ("A — small visible breathing room").

## Out of scope

- Adapting font-size dynamically based on the longest entry's character count (would require JS measurement; not worth the complexity for this iteration).
- Scaling the active accent bar with font-size.
- Multi-line entry support (current `white-space: nowrap` truncates; user accepted ellipsis).
