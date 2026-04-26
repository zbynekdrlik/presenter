# Stage Display Song Name Boxes

**Date:** 2026-04-19
**Status:** Approved

## Goal

Add "Current Song" and "Next Song" name boxes to the worship-snv stage display layout so the worship band can see which song is playing and what's coming next.

## Data Sources

### Current Song

Always comes from Presenter's existing `song_name` field, which is derived from the active presentation's name via `sanitize_song_title()`. Already present in `StageDisplaySnapshot`. No new data fetching needed.

### Next Song

Two mutually exclusive sources depending on the control mode:

1. **AbleSet mode** (AbleSet enabled, tracking, and has an active song): Next song is the song immediately after the active song in AbleSet's `/api/setlist` response. AbleSet maps songs directly from the Presenter library (not from a playlist), so Presenter has no playlist configured in this mode.

2. **Playlist mode** (Presenter has an active playlist, AbleSet not in control): Next song is the next presentation after the current one in the Presenter playlist.

3. **Neither active**: Next song box is empty/hidden.

### AbleSet Setlist Data

The AbleSet bridge already polls `/api/setlist` every 250ms and tracks the active song. The response contains:
- `activeSongId` — the currently active song's ID
- `songs[]` — ordered array of all songs with `id`, `meta.name`, `meta.raw`, `internalMeta.order`

To get the next song, find the active song's index in the array and return `songs[index + 1]`. If the active song is the last one, there is no next song.

## Layout Changes (worship-snv only)

### Before (current layout)

```
|          [    CURRENT GROUP (centered)    ]          |
| CURRENT SLIDE (96% width, centered text)             |
|          [    NEXT GROUP (centered)      ]           |
| NEXT SLIDE (96% width, centered text)                |
| CLOCK | #NUM |        LIVE        | CONNECTION      |
```

### After (new layout)

```
| [CURRENT GROUP (left)] ......... [CURRENT SONG (right)] |
| CURRENT SLIDE (96% width, centered text)                 |
| [NEXT GROUP (left)]    ......... [NEXT SONG (right)]     |
| NEXT SLIDE (96% width, centered text)                    |
| CLOCK | #NUM |        LIVE        | CONNECTION          |
```

### CSS Positioning

| Element | Left | Top | Width | Height |
|---------|------|-----|-------|--------|
| `stage__current-group` | 2% | 1% | 35% | 5% |
| **`stage__current-song`** (new) | 63% | 1% | 35% | 5% |
| `stage__current-slide` | 2% | 7% | 96% | 48% |
| `stage__next-group` | 2% | 56% | 35% | 4% |
| **`stage__next-song`** (new) | 63% | 56% | 35% | 4% |
| `stage__next-slide` | 2% | 61% | 96% | 30% |

Group pills move from `left: 25%` (centered) to `left: 2%` (left-aligned). Width stays similar. Song boxes mirror on the right side.

### Styling

- Color: `#fbbf24` (amber/gold)
- Background: `rgba(251, 191, 36, 0.1)` (subtle amber tint)
- Font weight: 600
- Text transform: uppercase
- Letter spacing: 0.08em
- Auto-fit text scaling (same utility as group pills, max font ~200px)
- Align text right within the box

## Backend Changes

### StageDisplaySnapshot Extension

Add one new field to `StageDisplaySnapshot` in `presenter-core/src/stage_display.rs`:

```rust
pub next_song_name: Option<String>,
```

Serialized as `nextSongName` (camelCase). Skip when None.

### AbleSet Bridge Extension

Add a method to `AbleSetBridge` to get the next song name. This requires caching the full setlist (not just the active song) so we can look up what comes next. Currently `run_tracker` only stores the active song's `SongState`.

Changes to `ableset.rs`:
- Store `songs: Vec<(String, String)>` (id, name) in `AbleSetStatusInner` alongside `last_song`
- Update `run_tracker` to cache the full song list on each poll
- Add `pub async fn next_song_name(&self) -> Option<String>` that finds the active song index and returns `songs[index + 1].name`

### Stage Snapshot Builder

In `state/stage.rs`, the snapshot builder (`build_stage_display_snapshot` or equivalent) populates `next_song_name`:

1. Check if AbleSet is enabled and tracking → call `ableset_bridge.next_song_name()`
2. Else check if a playlist is active → find the next presentation after the current one in `playlist_entries`
3. Else → `None`

### WebSocket

No protocol changes. `LiveEvent::Stage { snapshot }` already carries the full `StageDisplaySnapshot`. The new `next_song_name` field is included automatically via serde serialization.

## Frontend Changes (presenter-ui)

### worship_snv.rs

Add two new elements to the component:

1. `stage__current-song` — reads `snapshot.song_name`
2. `stage__next-song` — reads `snapshot.next_song_name`

Both get `autofit_effect()` with max font ~200px (same as group pills).

When the value is `None` or empty, the box is present but empty (consistent with how group pills behave when there's no group).

### stage.css

Add CSS rules for `.stage__current-song` and `.stage__next-song` with the positioning and amber styling defined above. Update `.stage__current-group` and `.stage__next-group` positioning from centered to left-aligned.

Add the new classes to the debug border rule list.

## What Stays Unchanged

- All other layouts (worship-pp, timer, preach, bible, ndi-fullscreen)
- Status bar (clock, song number, live pill, connection)
- Resolume clip mapping (song_name/band_name flow to Arena)
- AbleSet settings UI and API endpoints
- AbleSet polling interval (250ms)
- Companion variables

## Testing

### Unit Tests

- `next_song_name()` returns correct next song from cached setlist
- `next_song_name()` returns `None` when active song is last in setlist
- `next_song_name()` returns `None` when no active song
- Stage snapshot builder populates `next_song_name` from AbleSet when tracking
- Stage snapshot builder populates `next_song_name` from playlist when no AbleSet

### E2E (Playwright)

- Stage display shows current song name in amber box
- Stage display shows next song name when available
- Song name boxes auto-fit text
- Boxes are positioned correctly (right side of stage)
