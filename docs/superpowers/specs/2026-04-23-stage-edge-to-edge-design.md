# Stage Edge-to-Edge Layout Design

**Status:** Approved
**Date:** 2026-04-23
**Author:** Claude (brainstorming skill)
**Scope:** `worship-snv` stage layout (and `api` layout, which shares the same renderer)

## Problem

The stage display currently has ~2% left/right margins around every main box, and a ~30% empty gap between the two pills in the header and next-header rows. Because the TV is black and pixels outside the boxes carry no information, this wastes screen real estate that should be used to make lyrics larger and more legible for band singers on stage.

The current slide (the main lyric area) is the most affected: a 4% total horizontal margin reduces the text width noticeably. The group pill and song-name pill each occupy only 35% width, with a 30% dead zone between them.

## Goals

- **Current slide and next slide:** span the full viewport width with zero left/right margin.
- **Header row (group pill + song name):** each takes exactly 50% width and meets in the middle (no center gap).
- **Next row (next group pill + next song name):** same treatment as header row.
- **Vertical layout:** unchanged. The small vertical gaps between rows are kept for visual separation, and the existing `top` / `height` values are preserved.
- **Status bar:** unchanged. The clock, song number, live-pill, and connection boxes keep their current widths and positions.

## Non-goals

- Other layouts (`worship-pp`, `bible`, `timer`, `preach`, `ndi-fullscreen`) are not changed.
- No HTML / component changes in `crates/presenter-ui/src/components/stage/worship_snv.rs`. CSS-only change.
- No reallocation of vertical space between rows. Current slide height stays at 48%, next slide at 30%.
- No changes to autofit logic, text styling (colour, weight, letter-spacing), or pill background colours.

## Design

All changes are in `crates/presenter-ui/styles/stage.css`. The six targeted selectors each drop their 2% horizontal margin and, for the pills, expand to 50% width.

| Selector | `left` | `right` | `width` | `top` | `height` |
|---|---|---|---|---|---|
| `.stage__current-group` | `0` (was `2%`) | — | `50%` (was `35%`) | `1%` (unchanged) | `5%` (unchanged) |
| `.stage__current-song` | — | `0` (was `2%`) | `50%` (was `35%`) | `1%` (unchanged) | `5%` (unchanged) |
| `.stage__current-slide` | `0` (was `2%`) | — | `100%` (was `96%`) | `7%` (unchanged) | `48%` (unchanged) |
| `.stage__next-group` | `0` (was `2%`) | — | `50%` (was `35%`) | `56%` (unchanged) | `4%` (unchanged) |
| `.stage__next-song` | — | `0` (was `2%`) | `50%` (was `35%`) | `56%` (unchanged) | `4%` (unchanged) |
| `.stage__next-slide` | `0` (was `2%`) | — | `100%` (was `96%`) | `61%` (unchanged) | `30%` (unchanged) |

**Visual effect:**

- Current slide gains 4% horizontal space (~77px at 1920-wide). The autofit logic already present in `.stage__slide-text` automatically scales lyrics to fill the wider box — no code change needed.
- Group pill and song-name pill each gain 15% horizontal space and now meet in the middle. The autofit-aware text inside the pills will scale up proportionally.
- Debug-frame 1px borders (`.stage__current-group`, `.stage__current-song`, etc.) will touch in the middle where the two pills meet — visually this reads as a single intentional dividing line, not as a layout bug.

## Affected files

| File | Change |
|---|---|
| `crates/presenter-ui/styles/stage.css` | Update the six selectors above |
| `tests/e2e/stage-layout.spec.ts` | Add assertions that the six elements snap to screen edges |
| `tests/e2e/stage-snapshot.spec.ts` | Regenerate baseline screenshot if present; otherwise no change |
| `Cargo.toml` | Version bump before first commit |

## Testing

### E2E (Playwright)

Add a new test `stage edge-to-edge layout` in `tests/e2e/stage-layout.spec.ts`:

1. Navigate to `/stage` on the dev instance.
2. Wait for stage to reach connected state (existing helper).
3. Read `getBoundingClientRect()` on the six elements and assert:
   - `.stage__current-slide`: `x === 0` and `width === viewport.width`
   - `.stage__next-slide`: `x === 0` and `width === viewport.width`
   - `.stage__current-group`: `x === 0` and `width === viewport.width / 2` (±1px)
   - `.stage__current-song`: `right === viewport.width` and `width === viewport.width / 2` (±1px)
   - `.stage__next-group`: same as current-group
   - `.stage__next-song`: same as current-song
4. Assert the browser console has no errors or warnings (existing project-wide rule).

Because `px` values are fractional at non-round viewports, allow ±1px tolerance on the half-width assertions.

### Manual / visual verification

After CI deploys to dev:

1. Open `http://10.77.8.134:8080/stage` in Playwright at 1920×1080.
2. Verify visually that there is no black margin around the current slide or next slide, and that the header and next-header pills meet in the middle.
3. Confirm the status bar at the bottom looks identical to before (unchanged).

## Risks and edge cases

- **Autofit regression.** The existing text autofit logic is robust to width changes — it measures the container and shrinks text to fit. But once width increases, the text will be larger; if the current autofit has an upper bound clamp, we may not see the full gain. If that happens, verify and raise the clamp in a follow-up. (No change proposed for this spec.)
- **Pills overlapping in the middle.** Because both pills are absolutely positioned with `left: 0; width: 50%` and `right: 0; width: 50%`, they are mathematically adjacent (no overlap). The 1px debug borders will touch and render as a single 2px line where they meet — intended.
- **Snapshot test baseline.** `stage-snapshot.spec.ts` (if it has a committed pixel baseline) will need its baseline refreshed in the same PR, because the layout actually changed. If there is no committed baseline, no action is needed.
- **Other layouts not affected.** The changed selectors are all specific to the `worship-snv` layout class. No side effects on bible / timer / preach / worship-pp / ndi.

## Rollout

- Single PR from `dev` → `main`.
- Version bump first commit (per airuleset `version-bumping` rule).
- No migration, no deploy-script change, no external API change. CSS is part of the embedded WASM bundle — the deploy workflow picks it up automatically.

## Open questions

None. All scope decisions confirmed by the user during brainstorming:

- All six listed boxes go edge-to-edge; status bar is left untouched.
- Header and next-header pills each take 50% width and meet in the middle.
- Vertical layout is preserved — tiny gaps between rows are kept for visual separation.
- Scope is `worship-snv` and the `api` layout (which renders through the same component).
