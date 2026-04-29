# Worship-PP 10-Row Sidebar Fit — Design

**Date:** 2026-04-29
**Status:** Proposed
**Scope:** CSS only

## Goal

Bump the worship-pp playlist sidebar entry font from `5vh` to `7.5vh` so that exactly ~10 rows fit in the 92vh sidebar instead of the current ~14. The user wants larger song titles for projector readability; 10-row capacity is enough for typical service lengths.

## Non-goals

- No change to sidebar width (22%), slides-area width (78%), padding, or active-highlight color/border.
- No layout / WASM / server changes.

## Approach

Single-value CSS change in `crates/presenter-ui/styles/stage.css`. Math at 1080p:

| | Current (5vh) | Target (7.5vh) |
|---|---|---|
| font-size | 54px | 81px |
| Text box height (line-height 1.1) | 59.4px | 89.1px |
| Padding (0.3vh × 2) | 6.5px | 6.5px |
| Margin-bottom | 2px | 2px |
| **Per-row height** | **67.9px** | **97.6px** |
| Rows in 986px usable sidebar | ~14.5 | ~10.1 |

E2E `tests/e2e/stage-worship-pp.spec.ts` already asserts `entryFontSizePx >= 40`. Tighten to `>= 70` so the new 81px font is locked in (any accidental regression to ≤ 70px would fail the test).

### CSS change

In `.stage-pp__playlist-entry`, replace `font-size: 5vh;` with `font-size: 7.5vh;` and update the inline comment to reflect the new ~10-row target.

### E2E change

In `tests/e2e/stage-worship-pp.spec.ts`, replace `expect(measurements.entryFontSizePx).toBeGreaterThanOrEqual(40);` with `expect(measurements.entryFontSizePx).toBeGreaterThanOrEqual(70);` and refresh the comment.

## Risks

- Slovak diacritics in song titles already truncate at the previous 5vh font; at 7.5vh, fewer characters fit per row (~8 typical instead of ~12). Existing `text-overflow: ellipsis` handles this. The user has been iterating on the size/fit trade-off and is now asking explicitly for fewer-rows-bigger-text — so this is the desired trade-off.
- Per-row height includes the active row's 4px left border. The active row's `padding-left: calc(0.4rem - 4px)` compensation is unchanged — text alignment with non-active rows stays consistent.

## Out of scope

- Dynamic font sizing based on entry count.
- Multi-line entry support.
- Adjusting active-highlight border thickness.
