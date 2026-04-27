# Worship-PP — adopt worship-snv baseline + playlist tweaks Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bring `worship-snv`'s rendering improvements (slide-text wrapping, song-name boxes with autofit) into `worship-pp` while keeping `worship-pp`'s playlist sidebar; clean playlist entry names of their leading 3-digit ProPresenter prefix; force one row per entry with CSS ellipsis; derive the next-song box from the Presenter playlist instead of AbleSet's `next_song_name`.

**Architecture:** Frontend-only. Two files modified, one CSS rule extended, one helper added. No `presenter-server` work, no `presenter-core` contract change. `worship_snv` remains untouched.

**Tech Stack:** Rust + Leptos 0.7 (CSR/WASM), CSS.

**Spec:** `docs/superpowers/specs/2026-04-26-worship-pp-snv-baseline-design.md` (commit `2379413`)

---

## Context

Six WASM unit tests in `presenter-ui::utils::text` already exist for `break_if_long`. We add a sibling helper `clean_song_name` to the same module and add tests to the same `mod tests` block. The component file `worship_pp.rs` is rewritten — currently 147 lines, will become ~150 lines structurally identical to `worship_snv.rs` plus a sidebar block. The CSS change is three properties on one selector.

`presenter-ui` is excluded from the root workspace (separate `Cargo.lock`). Tests are run via `cargo test -p presenter-ui` from the workspace root, which delegates to native (non-WASM) target for the test binary — works because the test code doesn't pull WASM-only types.

---

## File Structure

| File | Status | Responsibility |
|---|---|---|
| `Cargo.toml` (workspace) | Modify | Bump `version = "0.4.36"`. |
| `crates/presenter-ui/src/utils/text.rs` | Modify | Add `pub fn clean_song_name(name: &str) -> String` and 6 unit tests inside the existing `mod tests` block. Existing `break_if_long` and its tests untouched. |
| `crates/presenter-ui/styles/stage.css` | Modify line 353 rule | Extend `.stage-pp__playlist-entry` with `white-space: nowrap; overflow: hidden; text-overflow: ellipsis;`. Existing properties and the `--active` modifier rule untouched. |
| `crates/presenter-ui/src/components/stage/worship_pp.rs` | Rewrite | Same shape as `worship_snv.rs` (refs, getters, autofit, six divs) plus the playlist sidebar with `clean_song_name` applied to each entry. Override `next_song_text` to walk `playlist_entries` (entry-after-active) instead of reading `s.next_song_name`. |

Nothing else touched.

---

## Task 1: Workspace prep — version bump 0.4.36

**Files:**
- Modify: `Cargo.toml:15`

- [ ] **Step 1: Bump workspace version**

In `Cargo.toml`, change line 15 from `version = "0.4.35"` to `version = "0.4.36"`.

- [ ] **Step 2: Verify workspace still builds**

```bash
cargo build -p presenter-server 2>&1 | tail -5
```

Expected: `Finished `dev` profile`.

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "chore: bump version to 0.4.36 (#worship-pp)"
```

---

## Task 2: Add `clean_song_name` helper with tests

**Files:**
- Modify: `crates/presenter-ui/src/utils/text.rs`

TDD order: write tests first, watch them fail, implement, watch them pass.

- [ ] **Step 1: Add the failing tests**

Open `crates/presenter-ui/src/utils/text.rs`. Inside the existing `#[cfg(test)] mod tests { ... }` block (the one containing `short_ascii_line_unchanged` etc.), append the following tests **before** the closing `}` of the module:

```rust
    #[test]
    fn clean_song_name_strips_3digit_prefix() {
        assert_eq!(clean_song_name("042 Amazing Grace"), "Amazing Grace");
    }

    #[test]
    fn clean_song_name_passes_through_non_prefixed() {
        assert_eq!(clean_song_name("Amazing Grace"), "Amazing Grace");
    }

    #[test]
    fn clean_song_name_rejects_two_digit_prefix() {
        assert_eq!(clean_song_name("12 Two Digit"), "12 Two Digit");
    }

    #[test]
    fn clean_song_name_rejects_four_digit_prefix() {
        assert_eq!(clean_song_name("1234 Four Digit"), "1234 Four Digit");
    }

    #[test]
    fn clean_song_name_handles_leading_whitespace() {
        assert_eq!(clean_song_name("  042 Padded"), "Padded");
    }

    #[test]
    fn clean_song_name_empty_input_unchanged() {
        assert_eq!(clean_song_name(""), "");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p presenter-ui --lib utils::text:: 2>&1 | tail -20
```

Expected: 6 new tests fail with `cannot find function 'clean_song_name'`. The 9 existing `break_if_long` tests pass.

- [ ] **Step 3: Implement the helper**

In `crates/presenter-ui/src/utils/text.rs`, append the following BEFORE the `#[cfg(test)] mod tests` block:

```rust
/// Strip a leading 3-digit-then-space prefix from a ProPresenter song name.
/// Mirrors the server-side `sanitize_song_title`:
///
///   "042 Amazing Grace"  -> "Amazing Grace"
///   "  042 Padded"       -> "Padded"
///   "12 Two Digit"       -> "12 Two Digit"   (not exactly 3 digits → unchanged)
///   "Already Clean"      -> "Already Clean"
///
/// Used by the `worship-pp` playlist sidebar to keep operator-facing
/// numeric prefixes off the stage display.
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

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test -p presenter-ui --lib utils::text:: 2>&1 | tail -20
```

Expected: all `utils::text::tests::*` tests pass — 9 existing `break_if_long` tests + 6 new `clean_song_name` tests = 15 total.

- [ ] **Step 5: Commit**

```bash
git add crates/presenter-ui/src/utils/text.rs
git commit -m "feat(stage): add clean_song_name helper for playlist entries (#worship-pp)"
```

---

## Task 3: Extend `.stage-pp__playlist-entry` CSS for one-row ellipsis

**Files:**
- Modify: `crates/presenter-ui/styles/stage.css` (rule at line 353)

- [ ] **Step 1: Locate the rule**

```bash
sed -n '350,375p' crates/presenter-ui/styles/stage.css
```

You should see the `.stage-pp__playlist-entry { ... }` block at line 353 followed by `.stage-pp__playlist-entry--active { ... }` around line 364.

- [ ] **Step 2: Append the three properties to the existing rule**

Edit `crates/presenter-ui/styles/stage.css`. Find the `.stage-pp__playlist-entry { ... }` block. Add these three lines BEFORE the closing `}`, preserving any existing properties already in the block:

```css
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
```

Do NOT touch the `.stage-pp__playlist-entry--active` rule below it.

- [ ] **Step 3: Verify by re-printing the block**

```bash
sed -n '350,375p' crates/presenter-ui/styles/stage.css
```

The base rule should now contain the three new properties. The `--active` modifier rule should be unchanged.

- [ ] **Step 4: Commit**

```bash
git add crates/presenter-ui/styles/stage.css
git commit -m "style(stage): one-row ellipsis for worship-pp playlist entries (#worship-pp)"
```

---

## Task 4: Rewrite `worship_pp.rs` from `worship_snv.rs` baseline + playlist sidebar + next-song-from-playlist

**Files:**
- Modify: `crates/presenter-ui/src/components/stage/worship_pp.rs` (rewrite)

This single task replaces the entire file. The replacement keeps the same component name (`WorshipPp`), the same outer container (`<div class="stage-container" data-layout="worship-pp">`), and the same external interface (`(ws_state, latency_ms)`). What changes is the contents.

- [ ] **Step 1: Overwrite the file with the full replacement**

Replace the entire contents of `crates/presenter-ui/src/components/stage/worship_pp.rs` with:

```rust
use leptos::prelude::*;

use crate::state::stage::StageContext;
use crate::utils::autofit::autofit_effect;
use crate::utils::color::group_pill_style;
use crate::utils::text::{break_if_long, clean_song_name};
use crate::ws::stage::StageWsState;

const CURRENT_MAX_FONT: f64 = 800.0;
const NEXT_MAX_FONT: f64 = 500.0;
const CURRENT_GROUP_MAX_FONT: f64 = 200.0;
const NEXT_GROUP_MAX_FONT: f64 = 200.0;
const CURRENT_SONG_MAX_FONT: f64 = 200.0;
const NEXT_SONG_MAX_FONT: f64 = 200.0;
const STAGE_SLIDE_BREAK_THRESHOLD: usize = 26;

#[component]
pub fn WorshipPp(
    ws_state: ReadSignal<StageWsState>,
    latency_ms: ReadSignal<Option<f64>>,
) -> impl IntoView {
    let ctx = use_context::<StageContext>().expect("StageContext not provided");

    let current_text_ref = NodeRef::<leptos::html::Div>::new();
    let next_text_ref = NodeRef::<leptos::html::Div>::new();
    let current_group_ref = NodeRef::<leptos::html::Div>::new();
    let next_group_ref = NodeRef::<leptos::html::Div>::new();
    let current_song_ref = NodeRef::<leptos::html::Div>::new();
    let next_song_ref = NodeRef::<leptos::html::Div>::new();

    let current_text = move || {
        let raw = ctx
            .snapshot
            .get()
            .and_then(|s| {
                s.current.map(|slide| {
                    if !slide.stage.is_empty() {
                        slide.stage
                    } else {
                        slide.main
                    }
                })
            })
            .unwrap_or_default();
        break_if_long(raw, STAGE_SLIDE_BREAK_THRESHOLD)
    };

    let next_text = move || {
        let raw = ctx
            .snapshot
            .get()
            .and_then(|s| {
                s.next.map(|slide| {
                    if !slide.stage.is_empty() {
                        slide.stage
                    } else {
                        slide.main
                    }
                })
            })
            .unwrap_or_default();
        break_if_long(raw, STAGE_SLIDE_BREAK_THRESHOLD)
    };

    let current_group = move || {
        ctx.snapshot
            .get()
            .and_then(|s| s.current.and_then(|sl| sl.group))
    };
    let next_group = move || {
        ctx.snapshot
            .get()
            .and_then(|s| s.next.and_then(|sl| sl.group))
    };

    let current_group_style = move || {
        ctx.snapshot
            .get()
            .and_then(|s| s.current.and_then(|sl| sl.group_color))
            .map(|color| group_pill_style(&color))
            .unwrap_or_default()
    };

    let next_group_style = move || {
        ctx.snapshot
            .get()
            .and_then(|s| s.next.and_then(|sl| sl.group_color))
            .map(|color| group_pill_style(&color))
            .unwrap_or_default()
    };

    let current_group_text = move || current_group().unwrap_or_default();
    let next_group_text = move || next_group().unwrap_or_default();

    let current_song_text = move || {
        ctx.snapshot
            .get()
            .and_then(|s| s.song_name)
            .unwrap_or_default()
    };

    let playlist_entries = move || {
        ctx.snapshot
            .get()
            .and_then(|s| s.playlist_entries)
            .unwrap_or_default()
    };

    // worship-pp specific: derive next-song from the Presenter playlist's
    // entry-after-active, NOT from AbleSet's s.next_song_name. If no entry
    // is active, or the active one is last, returns "" (no next song).
    let next_song_text = move || {
        let entries = playlist_entries();
        let mut iter = entries.iter().skip_while(|e| !e.is_active);
        iter.next(); // consume the active entry itself
        iter.next()
            .map(|e| clean_song_name(&e.name))
            .unwrap_or_default()
    };

    autofit_effect(current_text_ref, CURRENT_MAX_FONT, current_text);
    autofit_effect(next_text_ref, NEXT_MAX_FONT, next_text);
    autofit_effect(
        current_group_ref,
        CURRENT_GROUP_MAX_FONT,
        current_group_text,
    );
    autofit_effect(next_group_ref, NEXT_GROUP_MAX_FONT, next_group_text);
    autofit_effect(current_song_ref, CURRENT_SONG_MAX_FONT, current_song_text);
    autofit_effect(next_song_ref, NEXT_SONG_MAX_FONT, next_song_text);

    view! {
        <div class="stage-container" data-layout="worship-pp">
            <div class="stage__current-group">
                <span class="stage__debug-label">"current-group"</span>
                <div node_ref=current_group_ref class="stage__group-pill" style=current_group_style>
                    {current_group_text}
                </div>
            </div>

            <div class="stage__current-song">
                <span class="stage__debug-label">"current-song"</span>
                <div node_ref=current_song_ref class="stage__song-name-text">
                    {current_song_text}
                </div>
            </div>

            <div class="stage__current-slide">
                <span class="stage__debug-label">"current-slide"</span>
                <div node_ref=current_text_ref class="stage__slide-text">
                    {current_text}
                </div>
            </div>

            <div class="stage__next-group">
                <span class="stage__debug-label">"next-group"</span>
                <div node_ref=next_group_ref class="stage__group-pill" style=next_group_style>
                    {next_group_text}
                </div>
            </div>

            <div class="stage__next-song">
                <span class="stage__debug-label">"next-song"</span>
                <div node_ref=next_song_ref class="stage__song-name-text">
                    {next_song_text}
                </div>
            </div>

            <div class="stage__next-slide">
                <span class="stage__debug-label">"next-slide"</span>
                <div node_ref=next_text_ref class="stage__slide-text">
                    {next_text}
                </div>
            </div>

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

- [ ] **Step 2: Build the presenter-ui crate**

```bash
cargo build -p presenter-ui 2>&1 | tail -10
```

Expected: `Finished `dev` profile`. If errors:
- `unresolved import 'crate::utils::text::clean_song_name'` — Task 2 didn't run; go back and verify Task 2 committed.
- `cannot find type 'StagePlaylistEntry'` — confirm `crate::state::stage::StageContext` exposes `playlist_entries` as `Option<Vec<StagePlaylistEntry>>` (it does — see `crates/presenter-core/src/stage_display.rs:65,119`). The pattern `entries.iter().skip_while(|e| !e.is_active)` works because `StagePlaylistEntry` has `is_active: bool` (line 69).

- [ ] **Step 3: Run presenter-ui tests**

```bash
cargo test -p presenter-ui 2>&1 | tail -10
```

Expected: all green, including the new `clean_song_name` tests from Task 2.

- [ ] **Step 4: Commit**

```bash
git add crates/presenter-ui/src/components/stage/worship_pp.rs
git commit -m "feat(stage): worship-pp adopts worship-snv baseline + playlist tweaks (#worship-pp)

- Brings break_if_long for slide text + song-name boxes with autofit
  from worship_snv.rs into worship_pp.rs.
- Keeps the .stage-pp__playlist-sidebar with the <For> over playlist_entries.
- Each rendered entry runs through clean_song_name to strip the
  ProPresenter 3-digit prefix.
- Overrides next_song_text to walk playlist_entries for the entry
  after the active one, ignoring AbleSet's s.next_song_name.
- worship-snv unchanged."
```

---

## Task 5: Local fmt + clippy + workspace tests

**Files:** None (verification step).

- [ ] **Step 1: Format**

```bash
cargo fmt --all
cd crates/presenter-ui && cargo fmt --all && cd ../..
```

The presenter-ui crate has its own Cargo.lock and is excluded from the root workspace; running fmt twice covers both.

- [ ] **Step 2: Clippy zero-warnings on the workspace**

```bash
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all 2>&1 | tail -10
```

- [ ] **Step 3: Clippy zero-warnings on presenter-ui**

```bash
cd crates/presenter-ui && cargo clippy --all-targets -- -D warnings -W clippy::all 2>&1 | tail -15 && cd ../..
```

Expected: clean. Common things:
- `clippy::needless_borrow` if you wrote `&entry.name` where `entry.name` would auto-deref — adjust per the lint.
- `clippy::needless_collect` shouldn't appear; the iterator chain is short.

- [ ] **Step 4: Workspace tests**

```bash
cargo test --workspace 2>&1 | tail -10
```

- [ ] **Step 5: presenter-ui tests**

```bash
cargo test -p presenter-ui 2>&1 | tail -10
```

Expected: all green, including 6 new `clean_song_name` tests.

- [ ] **Step 6: If any of Steps 2-5 produced fixes, commit**

```bash
git add -A
git commit -m "chore: fmt + clippy fixes for worship-pp (#worship-pp)"
```

If no diff after fmt/clippy, skip the commit.

---

## Task 6: Push to dev + monitor CI

**Files:** None.

- [ ] **Step 1: Sync with main, then push**

```bash
git fetch origin
git merge origin/main --no-edit 2>&1 | tail -3
git push origin dev 2>&1 | tail -3
```

- [ ] **Step 2: Identify the new pipeline run**

```bash
sleep 12
gh run list --branch dev --limit 3 --json databaseId,name,status,event --jq '.[] | "\(.databaseId)\t\(.name)\t\(.status)\t\(.event)"'
```

Capture the `databaseId` of the newest `Pipeline` row triggered by `push`.

- [ ] **Step 3: Monitor with single-sleep pattern in background**

```bash
RUN_ID=<paste databaseId>
sleep 1500 && gh run view $RUN_ID --json status,conclusion,jobs --jq '{status,conclusion,jobs:[.jobs[]|{name,conclusion,status}]}'
```

Run as `run_in_background: true`. After it returns, if `Mutation Testing` or any job is still `in_progress`, schedule another `sleep 600 && gh run view $RUN_ID ...` background command. **Do NOT poll repeatedly. Do NOT use `gh run watch`.**

If any job fails: `gh run view $RUN_ID --log-failed | tail -100`, fix in ONE commit, push, monitor again.

- [ ] **Step 4: Confirm Deploy to Dev succeeded**

```bash
gh run view $RUN_ID --json jobs --jq '.jobs[] | select(.name=="Deploy to Dev") | .conclusion'
```

Expected: `success`.

---

## Task 7: Verify on dev

**Files:** None (live check against `http://10.77.8.134:8080`).

- [ ] **Step 1: Confirm dev is on 0.4.36**

```bash
curl -s http://10.77.8.134:8080/healthz; echo
```

Expected: `{"channel":"dev","status":"ok","version":"0.4.36"}`.

- [ ] **Step 2: Switch dev stage to worship-pp**

```bash
curl -s -X POST http://10.77.8.134:8080/stage/layout -H 'content-type: application/json' -d '{"code":"worship-pp"}'
```

Expected: response includes `"code":"worship-pp"`.

- [ ] **Step 3: Open dev stage in a browser and verify visually**

Navigate to `http://10.77.8.134:8080/stage` in a desktop browser. Visual checks:

1. Layout has the same structure as worship-snv (current-group / current-song / current-slide / next-group / next-song / next-slide) PLUS the `.stage-pp__playlist-sidebar` on the right.
2. If the playlist has any songs whose names start with a 3-digit number + space, those numbers do NOT appear in the sidebar (e.g. "042 Amazing Grace" renders as "Amazing Grace").
3. If any song name is long enough to overflow its container, the overflowing text is replaced by `…` rather than wrapping to a second row.
4. The active song row has the `--active` styling (typically a different background or color per the existing CSS).
5. The next-song box (`.stage__next-song`) shows the cleaned name of the song after the active one (or empty if active is last).

- [ ] **Step 4: Browser console check**

In DevTools console: zero errors, zero warnings. (Codebase rule from `browser-console-zero-errors.md`.)

- [ ] **Step 5: Mark task done — no commit, observation only.**

---

## Task 8: Open PR dev → main + monitor PR CI

**Files:** None (PR creation).

- [ ] **Step 1: Verify state**

```bash
git fetch origin
git log origin/main..origin/dev --oneline
gh pr list --base main --head dev --state open
```

- [ ] **Step 2: Create PR**

```bash
gh pr create --base main --head dev --title "feat(stage): worship-pp adopts worship-snv baseline + playlist tweaks" --body "$(cat <<'EOF'
## Summary
- Brings \`break_if_long\` slide-text wrapping and the song-name boxes (current-song / next-song) with their own autofit pass from worship_snv.rs into worship_pp.rs.
- Keeps worship-pp's playlist sidebar.
- Strips the leading 3-digit ProPresenter prefix from sidebar entry names via a new \`clean_song_name\` helper in \`presenter-ui::utils::text\`.
- Forces each sidebar entry to one row with CSS \`white-space: nowrap; overflow: hidden; text-overflow: ellipsis;\`.
- Overrides next-song-box content to derive from the Presenter playlist's entry-after-active, ignoring AbleSet's \`s.next_song_name\` for worship-pp.
- Frontend-only. No \`presenter-server\` or \`presenter-core\` change. \`worship_snv\` unaffected.

## Spec & plan
- Spec: \`docs/superpowers/specs/2026-04-26-worship-pp-snv-baseline-design.md\`
- Plan: \`docs/superpowers/plans/2026-04-26-worship-pp-snv-baseline.md\`

## Test plan
- [x] Six unit tests for \`clean_song_name\` covering 3-digit prefix strip, pass-through for non-prefixed / 2-digit / 4-digit, leading-whitespace handling, and empty input.
- [x] All existing \`break_if_long\` tests continue to pass.
- [x] \`cargo clippy --workspace --all-targets -D warnings\` clean.
- [x] \`cargo clippy -p presenter-ui --all-targets -D warnings\` clean.
- [x] Dev deploy verified: stage layout switched to \`worship-pp\` renders correctly on http://10.77.8.134:8080/stage with sidebar entries cleaned and one-row-with-ellipsis.
- [ ] Production verification post-merge (operator visual on the actual stage TVs).

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 3: Monitor PR CI**

```bash
sleep 10
gh pr checks <new-PR-number> 2>&1 | head -30
```

Use the `sleep N && gh pr view <N> --json mergeable,mergeStateStatus` background pattern from Task 6 to wait until terminal. ALL required checks must be green.

- [ ] **Step 4: Verify PR is mergeable + clean**

```bash
gh pr view <PR-number> --json number,mergeable,mergeStateStatus,url --jq '{number, mergeable, mergeStateStatus, url}'
```

Required: `mergeable: "MERGEABLE"`, `mergeStateStatus: "CLEAN"`. If `UNSTABLE`, investigate the failing check via `gh pr checks` and `gh run view --log-failed`. Fix the gate root-cause; per `pr-merge-policy.md` and `autonomous-quality-discipline.md`, do NOT propose admin-merge or "merge despite".

- [ ] **Step 5: Provide PR URL to user, wait for explicit merge instruction.**

Per `pr-merge-policy.md`: never merge without the user saying "merge it" or equivalent. Output the full clickable URL.

---

## Task 9: Post-merge production verification

**Files:** None (live check against `http://10.77.9.205`).

Triggered after the user says "merge it" and the merge to main runs the Deploy workflow.

- [ ] **Step 1: Confirm main deploy succeeded**

```bash
gh run list --branch main --limit 3
```

Find the newest `Deploy` run for the merge commit. Wait for it to reach `conclusion=success` using the same monitor pattern as Task 6.

- [ ] **Step 2: Confirm production version**

```bash
curl -s http://10.77.9.205/healthz; echo
```

Expected: `version: 0.4.36`, `channel: release`.

- [ ] **Step 3: Switch production stage to worship-pp and visually inspect**

```bash
curl -s -X POST http://10.77.9.205/stage/layout -H 'content-type: application/json' -d '{"code":"worship-pp"}'
```

Open `http://10.77.9.205/stage` in a desktop browser. Repeat the five visual checks from Task 7 Step 3 against the production deployment.

- [ ] **Step 4: Ask user for visual confirmation on the real stage TVs**

The operator confirms on the church's actual stage displays that:
- Playlist names are cleaned (no `042 ` prefix).
- Each entry stays on one row.
- The next-song box matches the Presenter playlist's next entry, not whatever AbleSet thinks is next.
- worship-snv (if used elsewhere) still behaves identically to before this PR.

Record the operator's verdict.

- [ ] **Step 5: Mark task done — no spec Findings update needed (this is a small UI tweak, not a multi-iteration design with empirical baselines).**

---

## Verification Summary

| Check | Where verified |
|---|---|
| `clean_song_name` strips 3-digit prefix | `utils::text::tests::clean_song_name_strips_3digit_prefix` (Task 2) |
| `clean_song_name` passes through non-prefixed names | `utils::text::tests::clean_song_name_passes_through_non_prefixed` (Task 2) |
| `clean_song_name` rejects 2-digit and 4-digit prefixes | Two dedicated tests (Task 2) |
| `clean_song_name` handles leading whitespace | `utils::text::tests::clean_song_name_handles_leading_whitespace` (Task 2) |
| `clean_song_name` handles empty input | `utils::text::tests::clean_song_name_empty_input_unchanged` (Task 2) |
| `worship_pp.rs` compiles after rewrite | `cargo build -p presenter-ui` (Task 4 Step 2) |
| Sidebar names render cleaned | Visual check (Task 7 Step 3.2) |
| Sidebar entries one row with ellipsis | Visual check (Task 7 Step 3.3) |
| Next-song from playlist (not AbleSet) | Visual check (Task 7 Step 3.5) + operator confirm (Task 9 Step 4) |
| `worship_snv` unaffected | No change to `worship_snv.rs`; existing E2E `tests/e2e/stage-api-ndi.spec.ts` and any worship-snv-touching specs continue to pass via Task 6's CI |
