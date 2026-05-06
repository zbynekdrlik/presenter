# Worship search returns whole songs, not slide passages

**Issue:** #283

**Date:** 2026-05-05

**Branch:** dev (workspace 0.4.64)

## Problem

Worship search currently returns three kinds of results — libraries, presentations (matched by name), and **individual slides** (matched by slide text). The slide results show as "passages" in the search UI: a single slide of a song, separated from its parent. The user wants the search to continue looking inside slide content (so songs with matching lyrics surface) but to return **the whole song** instead of the matching slide.

User wording: "worship song search should only search whole songs with the text in them not like now it searches some passages" — confirmed in brainstorming as: "i should search in slides, but i only want to load whole song not some passage."

## Goal

Search continues to query slide content. When a slide matches, the **parent presentation** is returned, not the slide. If the same presentation matches both by name (existing behavior) and by slide content, it appears once.

## Architecture

### `search_slides` in `crates/presenter-persistence/src/repository/search.rs`

The slide-text query stays. The result emission changes:

- Old: each matching slide → `SearchResult { kind: Slide, slide_id, presentation_id, ... }`.
- New: each matching slide → `SearchResult { kind: Presentation, presentation_id, ... }`, deduped by `presentation_id` against `ctx.seen_presentation_ids`.

The `match_field` retains its existing semantics (`MainText` / `TranslationText` / `StageText`). It now indicates which slide field caused the parent presentation to match.

### Frontend (`crates/presenter-ui/src/components/search.rs`)

The grouping logic at line 274 drops the `Slide` branch. The "Slides" UI section at line 345 is unreachable and gets deleted.

### Enum cleanup (`crates/presenter-core/src/search.rs`)

`SearchResultKind::Slide` is removed. Search results are ephemeral (in-memory Vec exchanged over the search endpoint), so there is no migration concern. Any code referencing the variant breaks at compile time.

The `SearchMatchField::MainText`/`TranslationText`/`StageText` variants stay — they remain useful diagnostic info on slide-matched presentation results.

## Out-of-scope

- The bible search flow (`search_bible_passages`) is independent and unchanged.
- The library-name match phase (`search_libraries`) is unchanged.
- The result `cap` (limit) handling is unchanged.

## Testing

### Server unit tests (`crates/presenter-server/src/router/tests.rs`)

1. **`search_slide_text_match_returns_parent_presentation`** — insert a presentation whose slides contain a unique marker `"test283-slide-marker"` (not in the presentation name or library name). Call `/search?query=test283-slide-marker`. Assert the response contains exactly one result, kind = Presentation, presentation_name matches.

2. **`search_dedupes_when_song_matches_both_name_and_slides`** — insert a presentation named `"Marker Song"` whose slides also contain "marker". Call `/search?query=marker`. Assert exactly one Presentation result (not two — the slide phase must skip presentations already in `seen_presentation_ids`).

3. **Existing `search_endpoint_returns_results` (router/tests.rs:555)** — must still pass. If it asserted `SearchResultKind::Slide` results, update to expect Presentation results.

### Playwright E2E (`tests/e2e/wasm-search.spec.ts`)

Add a test that:

1. Types a phrase known to exist in slide content but not in any presentation name (use a unique marker inserted via fixture or via existing data).
2. Asserts the result panel shows the presentation entry under "Presentations".
3. Asserts there is no "Slides" group section in the result panel.
4. Browser console clean.

### Manual verification on dev

After deploy, search for a phrase from the middle of a worship song's slide. Verify the search UI shows the song under "Presentations" (loadable as one click). No "Slides" group renders.

## File-level changes

| File | Change |
|---|---|
| `crates/presenter-persistence/src/repository/search.rs` | `search_slides` builds `SearchResult { kind: Presentation, ... }` per unique parent presentation, deduped against `ctx.seen_presentation_ids`. Drop slide-result construction. `slide_id` field stays `None`. |
| `crates/presenter-core/src/search.rs` | Remove `SearchResultKind::Slide` variant. |
| `crates/presenter-ui/src/components/search.rs` | Drop the `Slide` arm in result grouping (line 274). Delete the "Slides" `<section>` block at line 345. |
| `crates/presenter-server/src/router/tests.rs` | Add 2 new server unit tests. Update `search_endpoint_returns_results` if it asserts Slide results. |
| `tests/e2e/wasm-search.spec.ts` | Add E2E asserting slide-text-only search returns a presentation. |

## Acceptance

- All 2 new + existing server unit tests pass.
- New Playwright E2E asserts the new behavior.
- All workspace tests pass (`cargo test --workspace`, WASM clippy, native clippy).
- Mutation testing on the touched files passes.
- Browser console clean during E2E.
- Manual verification on dev: search for slide-text returns the parent song.

## Risk

- Removing the `Slide` variant is a workspace-wide breaking change. The compiler enforces consumer updates. Audit confirmed only the WASM frontend (line 274) and the search tests reference the variant.
- Dedupe via `seen_presentation_ids` is the existing mechanism; reusing it preserves insertion order — the name-match presentation appears first, then slide-match presentations after.

## Acceptance summary

| Behavior | Verified by |
|---|---|
| Slide-content search returns parent song | Unit test `search_slide_text_match_returns_parent_presentation` |
| Same song matched by name + slide returns once | Unit test `search_dedupes_when_song_matches_both_name_and_slides` |
| Slides UI group is removed | Playwright E2E + visual on dev |
| No regression for existing search behaviors | Existing `search_endpoint_returns_results` + workspace tests |
