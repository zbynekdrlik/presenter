# Stage Song Number Display — Design Spec

**Issue:** #225 — On stage display also number of song for newlevel worship
**Date:** 2026-04-10

## Problem

The stage display shows lyrics, group labels, and a status bar (clock, LIVE indicator, connection status), but does not show the song's catalog number. Musicians need to see the song number from the ProPresenter library (e.g., "042" from "042 Amazing Grace") on the stage screen.

## Solution

Extract the 3-digit number prefix from the presentation name and display it as `#042` in the stage status bar, between the clock and the LIVE pill.

## Data Flow

1. **`StageDisplaySnapshot`** gains a new field: `song_number: Option<String>`
2. **`extract_song_number(name: &str) -> Option<String>`** — new function in `crates/presenter-server/src/state/stage.rs` that pulls the leading 3-digit prefix from the presentation name. Returns `None` if the name doesn't match the `NNN ` pattern.
3. **Server populates** `song_number` when building the stage snapshot, using `presentation_name` as input.
4. **WASM `StatusBar` component** reads `snapshot.song_number` and renders `#042` in the status bar when present. When `None`, nothing is rendered — no empty space.

## UI Placement

```
┌──────────────────────────────────────────────────┐
│  [current group pill]                            │
│                                                  │
│  Current slide lyrics                            │
│  line 2                                          │
│                                                  │
│  [next group pill]                               │
│                                                  │
│  Next slide lyrics (dimmed)                      │
│                                                  │
│  20:15:32    #042    LIVE    CONNECTED · 12 ms   │
└──────────────────────────────────────────────────┘
```

The song number appears between the clock and the LIVE pill in the existing status bar grid.

## Format

- Displayed as `#NNN` (e.g., `#042`, `#001`, `#115`)
- Uses a distinct color (blue, `#2196F3`) to differentiate from clock and connection text
- Same autofit sizing as other status bar elements

## Edge Cases

- **No number prefix:** `song_number` is `None`, nothing rendered. Status bar shows clock, LIVE, and connection only (current behavior).
- **No active presentation:** `presentation_name` is `None`, so `song_number` is `None`.
- **Non-3-digit prefixes** (e.g., `"1 Song"`, `"12 Song"`): Not matched. Only exactly 3 digits followed by a space qualify. This matches the existing `sanitize_song_title()` logic.

## Files Modified

| File | Change |
|------|--------|
| `crates/presenter-core/src/stage_display.rs` | Add `song_number: Option<String>` to `StageDisplaySnapshot` |
| `crates/presenter-server/src/state/stage.rs` | Add `extract_song_number()`, populate field in snapshot builder |
| `crates/presenter-ui/src/components/stage/status_bar.rs` | Render `#NNN` between clock and LIVE pill |
| `crates/presenter-server/src/state/broadcasting.rs` | Pass song_number through snapshot construction |
| `tests/e2e/stage-status-bar.spec.ts` | E2E test: song number visible on stage when presentation has number prefix |

## Testing

- **Unit test:** `extract_song_number` returns correct values for `"042 Song"`, `"001 Song"`, `"Song Without Number"`, `""`, `"12 Song"`.
- **E2E test:** Activate a presentation with a numbered name, verify `#042` appears in the stage status bar. Activate a presentation without a number prefix, verify no song number element is rendered.
