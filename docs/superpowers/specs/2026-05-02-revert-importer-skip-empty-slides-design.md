# Revert .pro Importer Skip-Empty-Slides Rule — Design

**Date:** 2026-05-02
**Status:** Proposed
**Scope:** Backend (presenter-importer) — single file, single commit revert
**Issue:** [#284](https://github.com/zbynekdrlik/presenter/issues/284) — `.pro` library import drops intentional blank intro slides

## Goal

Restore intentional blank intro slides in `.pro` library imports by reverting commit `1b874be` (April 13, 2026 — "fix(importer): skip fully-empty slides with no group (#215)"). Songs imported from ProPresenter `.pro` bundles must keep EVERY slide from the source, including the deliberately-blank first slide that operators rely on for clean clears.

## What we're reverting and why

Commit `1b874be` modified `slide_content_from_proto` in `crates/presenter-importer/src/lib.rs` to skip slides where `buckets.is_empty() && group.is_none()` — i.e., slides with no text content AND no group label. The intent (per issue #215) was to strip ProPresenter placeholder-only junk slides from the imported list.

**Side effect not caught at the time:** intentional blank intro slides — the deliberately-empty slides operators put at the start of every song to allow a clean clear before launching lyrics — also have no text and no group label. The rule strips them. Old issue #27 already specified the desired behavior: *"Preserve the intentionally blank first slide that should exist on almost every presentation or song for clean clears."*

A re-import triggered by a recent deploy revealed the regression to the user (issue #284): "all songs are imported incorrectly… i want you to find it and revert it".

## Approach

Pure revert of `1b874be`. Three edits in `crates/presenter-importer/src/lib.rs`:

1. **`slide_content_from_proto` return type** — change `Result<Option<SlideContent>>` back to `Result<SlideContent>`.
2. **Body of `slide_content_from_proto`** — remove the `if buckets.is_empty() && group.is_none() { return Ok(None); }` early-return; restore `if buckets.is_empty() { buckets.push((TextRole::Main, String::new())); }` so empty slides become visible blank slides.
3. **Call site in `presentation_from_proto`** — restore `let content = slide_content_from_proto(base_slide, group)?;` (was `let Some(content) = …? else { continue; };`).

Plus restore the three unit-test variants `1b874be` had to introduce to handle the `Option`-returning signature back to their original `Result<SlideContent>` shape.

## Out of scope

- **`crates/presenter-ui/src/utils/song_parser.rs`** — the clipboard-paste parser is correct and stays untouched (per user instruction). Its `chunk_to_two_lines` / `wrap_with_empty_bookends` rules are independent of the `.pro` importer.
- **The clipboard flow in `presentation_modal.rs`** — untouched.
- **Issue #215's placeholder-junk problem** — returns as a known regression. If a smarter rule is wanted later (e.g. "skip slides whose only content is a single literal `.` character"), file a separate followup issue. The user has accepted this trade-off explicitly.
- **Issue #228 (seed-library race)** — separate issue. May be the reason a re-import is happening on every deploy; not fixed here.

## Files changed

| File | Change |
|------|--------|
| `Cargo.toml` (workspace `[workspace.package]`) | `0.4.49` → `0.4.50` |
| `crates/presenter-ui/Cargo.toml` | `0.1.18` → `0.1.19` |
| `crates/presenter-importer/src/lib.rs` | Revert commit `1b874be`'s changes (signature, body, call site, 3 tests) |

No changes to: server router, frontend WASM, CSS, JS, E2E tests.

## Testing

### Unit tests in `presenter-importer`

The three tests `1b874be` modified are reverted to their pre-`1b874be` form:

- The "blank slide kept" test calls `slide_content_from_proto(&slide, None)` and expects `Result<SlideContent>` (not `Result<Option<SlideContent>>`); the assertion that `content.main.value().is_empty()` stays.
- The "non-empty slide" test calls the same with text-bearing input and expects the result content directly.
- Any test that used `.expect("slide kept due to group")` or `.expect("non-empty slide")` is reverted to `.expect("content")`.

After revert, `cargo test -p presenter-importer` runs all importer tests including the original ones that asserted blank-slide preservation.

### Manual verification (post-deploy)

After CI deploys to dev, trigger an Import Data run on the dev environment for a known library (e.g. one that has a song with a blank intro slide — most worship songs from the production library qualify). Open the operator and confirm the song's first slide is the deliberately-blank one (no `Title:` / lyric content; just an empty slide for clean clears).

### Regression risk

ProPresenter placeholder-only junk slides (the issue-#215 problem) come back as the trade-off. Acceptable per user's explicit choice. If the operator finds the junk slides intolerable, a smarter rule lands in a followup PR.

## Risks / unknowns

- **Why a re-import happened on each deploy** — likely the seed-library race in issue #228. Out of scope for this PR; flag in completion report.
- **Production data** — the live production library was imported with `1b874be`'s rule and is currently MISSING the intentional blank slides. After this PR deploys + a re-import is triggered, those slides come back. The user will need to either trigger a re-import via Actions OR accept that future imports preserve blanks while past imports don't.
