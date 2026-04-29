# Worship-PP layout — adopt worship-snv baseline + playlist tweaks — Design

**Issue:** new (no GitHub issue filed; user-driven via brainstorming).

## Problem

The `worship-snv` stage layout has accumulated improvements that `worship-pp` is missing:

1. `break_if_long` wrapping for slide text (avoids awkward single-line cramming).
2. Song-name boxes (`.stage__current-song` / `.stage__next-song`) with their own autofit pass.
3. The constants and refs that drive those.

`worship-pp` still has the older, plainer rendering and is functionally unchanged for months. We want it visually equivalent to `worship-snv` — except `worship-pp` keeps a feature `worship-snv` doesn't have: a playlist sidebar showing the full setlist with the active song highlighted.

The sidebar itself has two operator pain points:

1. ProPresenter song names carry a leading 3-digit number (e.g. `042 Amazing Grace`) used by the operator to find the song in ProPresenter. On stage display, that prefix is noise.
2. Long song names wrap onto a second row, breaking the rhythm of the list. Each entry should be one row.

And one source-of-truth issue specific to `worship-pp`:

3. The next-song box currently reads `s.next_song_name`, which the server populates from AbleSet (the external setlist coordinator). For `worship-pp`, the user wants next-song from the **Presenter playlist**'s entry-after-active — what the operator actually sees in the Presenter UI — not from AbleSet's view.

`worship-snv` does not have a playlist sidebar and continues to use `s.next_song_name` from AbleSet. Out of scope here.

## Approach

Frontend-only. Two files modified, one new CSS rule, one new helper. Zero contract changes; no server-side work; `worship-snv` unaffected.

1. **`worship_pp.rs`** is rewritten from `worship_snv.rs` as the structural base — picks up `break_if_long`, song-name boxes, autofit refs, the constants. We then add back the `.stage-pp__playlist-sidebar` block and the `playlist_entries` getter that `worship_snv` doesn't have.

2. **`utils/text.rs`** gains a `clean_song_name(&str) -> String` helper that mirrors the server-side `sanitize_song_title` (strips a leading 3-digit-then-space prefix). The frontend can't call the server-side function directly because they're in different crates and `sanitize_song_title` is `pub(crate)`. Mirroring is a 10-line copy with the same behaviour.

3. **`worship_pp.rs::next_song_text`** is overridden vs the snv copy: instead of reading `ctx.snapshot.get().and_then(|s| s.next_song_name)`, it walks `playlist_entries`, skips entries until the `is_active` one, takes the entry after that, and runs its name through `clean_song_name`. If no active entry or active is last, returns empty (no next song shown).

4. **CSS** for `.stage-pp__playlist-entry` adds `white-space: nowrap; overflow: hidden; text-overflow: ellipsis;` so each entry stays one row and overflows with `…`.

## File-level scope

| File | Change |
|---|---|
| `crates/presenter-ui/src/components/stage/worship_pp.rs` | **Rewrite** — same shape as `worship_snv.rs` plus the playlist sidebar, with `next_song_text` and the playlist `<For>` modified per spec. Component name stays `WorshipPp`. |
| `crates/presenter-ui/src/utils/text.rs` | **Add** `pub fn clean_song_name(name: &str) -> String` plus 4 unit tests. Existing `break_if_long` is unchanged. |
| `crates/presenter-ui/styles/stage.css` (rule at line 353, `.stage-pp__playlist-entry`) | **Extend** the existing rule with three properties for one-row-with-ellipsis. Keep existing properties. |

Nothing else touched. No `presenter-core` change. No `presenter-server` change. No new dep.

## Component sketch — `worship_pp.rs`

Pseudocode of the divergent parts vs `worship_snv.rs`:

```rust
use crate::utils::text::{break_if_long, clean_song_name};
// ... rest of imports same as worship_snv.rs ...

#[component]
pub fn WorshipPp(...) -> impl IntoView {
    // ... all the same setup as worship_snv.rs:
    //     - context, refs (current_text_ref, next_text_ref, current_group_ref, next_group_ref,
    //                      current_song_ref, next_song_ref)
    //     - current_text / next_text getters with break_if_long
    //     - current_group / next_group / *_style / *_text getters
    //     - current_song_text getter (from ctx.snapshot.song_name)
    //     - autofit_effect calls for all six refs

    // worship-pp ONLY: bring back playlist_entries
    let playlist_entries = move || {
        ctx.snapshot
            .get()
            .and_then(|s| s.playlist_entries)
            .unwrap_or_default()
    };

    // worship-pp ONLY: next-song from playlist, not AbleSet
    let next_song_text = move || {
        let entries = playlist_entries();
        let mut iter = entries.iter().skip_while(|e| !e.is_active);
        iter.next();           // consume the active entry itself
        iter.next()            // entry after active
            .map(|e| clean_song_name(&e.name))
            .unwrap_or_default()
    };

    autofit_effect(next_song_ref, NEXT_SONG_MAX_FONT, next_song_text);

    view! {
        <div class="stage-container" data-layout="worship-pp">
            // ... same six divs as worship_snv (current-group, current-song, current-slide,
            //      next-group, next-song, next-slide) ...

            <div class="stage-pp__playlist-sidebar">
                <span class="stage__debug-label">"playlist-sidebar"</span>
                <For
                    each=playlist_entries
                    key=|entry| entry.name.clone()
                    children=move |entry| {
                        let class = if entry.is_active {
                            "stage-pp__playlist-entry stage-pp__playlist-entry--active"
                        } else {
                            "stage-pp__playlist-entry"
                        };
                        let display_name = clean_song_name(&entry.name);
                        view! { <div class=class>{display_name}</div> }
                    }
                />
            </div>

            <super::status_bar::StatusBar ws_state=ws_state latency_ms=latency_ms />
        </div>
    }
}
```

## `clean_song_name` helper

```rust
/// Strip a leading 3-digit-then-space prefix from a ProPresenter song name.
/// Mirrors the server-side `sanitize_song_title`:
///
///   "042 Amazing Grace"  -> "Amazing Grace"
///   "  042 Padded"       -> "Padded"
///   "12 Two Digit"       -> "12 Two Digit"   (not exactly 3 digits → unchanged)
///   "Already Clean"      -> "Already Clean"
///
/// Used by the worship-pp playlist sidebar to keep operator-facing numeric
/// prefixes off the stage display.
pub fn clean_song_name(name: &str) -> String {
    let trimmed = name.trim_start();
    let bytes = trimmed.as_bytes();
    if bytes.len() >= 4
        && bytes[0].is_ascii_digit()
        && bytes[1].is_ascii_digit()
        && bytes[2].is_ascii_digit()
        && bytes[3].is_ascii_whitespace()
    {
        trimmed[4..].trim_start().to_string()
    } else {
        trimmed.to_string()
    }
}
```

Test cases (unit):
- `clean_song_name("042 Amazing Grace")` → `"Amazing Grace"`
- `clean_song_name("Amazing Grace")` → `"Amazing Grace"`
- `clean_song_name("12 Two Digit")` → `"12 Two Digit"` (rejects, not exactly 3)
- `clean_song_name("1234 Four Digit")` → `"1234 Four Digit"` (rejects, more than 3)
- `clean_song_name("  042 Padded")` → `"Padded"`
- `clean_song_name("")` → `""`

## CSS for playlist entries

Add to the `.stage-pp__playlist-entry` rule (do not replace the existing properties — extend them):

```css
.stage-pp__playlist-entry {
    /* existing properties unchanged */
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
```

The existing `.stage-pp__playlist-entry` rule lives in `crates/presenter-ui/styles/stage.css` at line 353 (verified). Append the three new properties; do not touch the `--active` modifier rule below it.

## Behavior matrix

| Scenario | What renders in next-song box |
|---|---|
| Playlist has 5 songs, song #2 is active | Cleaned name of song #3 |
| Active song is the last entry | Empty (no next song) |
| No entry is active (e.g. setlist not started) | Empty |
| `playlist_entries` is None / empty | Empty |
| AbleSet has different idea of "next" | Ignored — Presenter playlist wins |

## Tests

**Unit (frontend, in `presenter-ui` crate):**
- `clean_song_name` — six cases above.

**Existing tests** that must keep passing:
- `tests/e2e/stage-worship-pp.spec.ts` (or whichever Playwright file covers `/stage` with `worship-pp` layout, if one exists). The component still renders for `data-layout="worship-pp"`.

**No new E2E** required for this PR — the changes are visual layout adjustments and a name-cleaning helper that's covered by unit tests. If we discover during dev that the next-song-from-playlist logic interacts with the AbleSet/Presenter data flow in a way that benefits from end-to-end coverage, we'll add a Playwright test then.

## Risks

- **`break_if_long` threshold of 26 chars** is tuned for `worship-snv`. `worship-pp` has the same slide-text width because the layout positions (`width:66%; left:2%;`) match. Acceptable.
- **`clean_song_name` and `sanitize_song_title` will drift** if either is changed without the other. We accept this risk for now (last-iteration MJPEG-style YAGNI; both are 10-line static functions, easy to audit). If drift becomes a real issue, lift to `presenter-core` later.
- **AbleSet integration in `worship-pp`** is effectively dead-code for the next-song display once this lands — `worship-pp` no longer reads `s.next_song_name`. The field still feeds `worship-snv`, so the server still computes it. That's fine.

## Out of scope

- Changes to `worship_snv` (untouched).
- Server-side song-name normalization (`sanitize_song_title` stays where it is).
- New snapshot fields like `next_song_from_playlist`.
- Refactoring shared layout fragments out of `worship_pp.rs` / `worship_snv.rs` — they will share ~120 lines of structure but extracting now is premature.
- Visual tuning of the playlist sidebar beyond one-row + ellipsis (font sizes, colors, spacing).

## Decision log

- **Frontend-only over backend stripping (Approach A vs C):** keeps the diff small (two files), no contract change, `worship-snv` provably unaffected. The duplication of `sanitize_song_title` ↔ `clean_song_name` is a 10-line copy that's trivially auditable.
- **Ellipsis over autofit-shrink for entries:** standard CSS, predictable visual, no font jitter as the active row shifts. Autofit per-entry would also work but introduces inconsistent text sizes across the list.
- **Next-song from playlist, not AbleSet:** explicit user requirement; matches what the operator sees in the Presenter UI rather than the external coordinator.
- **No new Playwright E2E:** the change is a layout/helper refactor; unit coverage of `clean_song_name` plus existing E2E for the `/stage` page is sufficient.
