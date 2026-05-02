# Revert .pro Importer Skip-Empty-Slides Rule Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restore intentional blank intro slides in `.pro` library imports by reverting commit `1b874be` (April 13, 2026 — "skip fully-empty slides with no group (#215)").

**Architecture:** Pure single-file revert. The clipboard-paste parser (`crates/presenter-ui/src/utils/song_parser.rs`) is correct and stays untouched per user instruction.

**Tech Stack:** Rust (presenter-importer crate). Server-side change.

**Spec:** `docs/superpowers/specs/2026-05-02-revert-importer-skip-empty-slides-design.md` (commit `4b1f75b`).

---

## Context

PR #282 merged into main earlier today (commit `0313d14`). Main and dev are at v0.4.49. New work piles onto dev; the new PR auto-opens after the work pushes.

**Current code state in `crates/presenter-importer/src/lib.rs`** (verified via `Read`):

- Line 272-274 (call site in `presentation_from_proto`):
  ```rust
  let Some(content) = slide_content_from_proto(base_slide, group)? else {
      continue;
  };
  ```
- Line 290-293 (function signature):
  ```rust
  fn slide_content_from_proto(
      base_slide: &proto::Slide,
      group: Option<SlideGroup>,
  ) -> Result<Option<SlideContent>> {
  ```
- Line 313-317 (the skip rule):
  ```rust
  // Skip slides that have no text content AND no group assignment
  // (these are artifacts of ProPresenter slides with only placeholder elements).
  if buckets.is_empty() && group.is_none() {
      return Ok(None);
  }
  ```
- Line 330-335 (the wrapped return):
  ```rust
  Ok(Some(SlideContent::new(
      SlideText::new(main)?,
      SlideText::new(translation)?,
      SlideText::new(stage)?,
      group,
  )))
  ```

Three tests in `mod tests` (lines 544-628) need adjustment:
- `slide_content_treats_single_dot_as_blank` (~line 544) — uses `.expect("result").expect("slide kept due to group")`
- `slide_content_preserves_real_text_when_stage_placeholder_removed` (~line 572) — uses `.expect("result").expect("non-empty slide")`
- `slide_content_returns_none_when_no_elements_and_no_group` (~line 620) — asserts `result.is_none()`. **This test is the bug guard for `1b874be` and is the WRONG behavior under #284**. Will flip to assert the slide IS kept (regression test against re-introduction).

**Build / test commands:**
- Test: `cargo test -p presenter-importer 2>&1 | tail -20`
- Workspace fmt: `cargo fmt --all --check`
- Workspace clippy: `cargo clippy --workspace --all-targets -- -D warnings -W clippy::all`

**The clipboard parser stays UNTOUCHED.** Do not edit:
- `crates/presenter-ui/src/utils/song_parser.rs`
- `crates/presenter-ui/src/components/presentation_modal.rs`

---

## File Structure

| File | Change |
|------|--------|
| `Cargo.toml` (workspace `[workspace.package]`) | `0.4.49` → `0.4.50` |
| `crates/presenter-ui/Cargo.toml` | `0.1.18` → `0.1.19` |
| `crates/presenter-importer/src/lib.rs` | Revert `1b874be`'s changes (signature, body, call site, 2 test variants restored to original, 1 test flipped as regression guard) |

No other files. No CSS, no E2E, no JS, no UI components, no router.

---

## Task 1: Bump version 0.4.49 → 0.4.50

**Files:**
- Modify: `Cargo.toml`
- Modify: `crates/presenter-ui/Cargo.toml`

- [ ] **Step 1: Sync with remote**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
git fetch origin
git status -sb
```
Expected: clean working tree on `dev`.

- [ ] **Step 2: Bump workspace version**

In `/home/newlevel/devel/presenter/presenter-dev2/Cargo.toml` under `[workspace.package]`, change `version = "0.4.49"` to `version = "0.4.50"`.

- [ ] **Step 3: Bump presenter-ui version**

In `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui/Cargo.toml` under `[package]`, change `version = "0.1.18"` to `version = "0.1.19"`.

- [ ] **Step 4: Refresh both Cargo.lock files**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
cargo check -p presenter-server 2>&1 | tail -3
cd crates/presenter-ui && cargo check --target wasm32-unknown-unknown 2>&1 | tail -3 && cd ../..
```
Expected: both `Finished ...`.

- [ ] **Step 5: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
git add Cargo.toml Cargo.lock crates/presenter-ui/Cargo.toml crates/presenter-ui/Cargo.lock
git commit -m "chore: bump version to 0.4.50 (#284)"
```

---

## Task 2: Revert importer skip rule + flip regression test

**Files:**
- Modify: `crates/presenter-importer/src/lib.rs`

- [ ] **Step 1: Revert `slide_content_from_proto` signature**

In `crates/presenter-importer/src/lib.rs`, find the function signature around line 290:

```rust
fn slide_content_from_proto(
    base_slide: &proto::Slide,
    group: Option<SlideGroup>,
) -> Result<Option<SlideContent>> {
```

Change to:

```rust
fn slide_content_from_proto(
    base_slide: &proto::Slide,
    group: Option<SlideGroup>,
) -> Result<SlideContent> {
```

- [ ] **Step 2: Remove the skip rule, restore the empty-bucket fallback**

In the same function, find lines 313-317:

```rust
    // Skip slides that have no text content AND no group assignment
    // (these are artifacts of ProPresenter slides with only placeholder elements).
    if buckets.is_empty() && group.is_none() {
        return Ok(None);
    }
```

Replace with:

```rust
    if buckets.is_empty() {
        buckets.push((TextRole::Main, String::new()));
    }
```

- [ ] **Step 3: Unwrap the return value from `Ok(Some(...))` to `Ok(...)`**

In the same function, find lines 330-335:

```rust
    Ok(Some(SlideContent::new(
        SlideText::new(main)?,
        SlideText::new(translation)?,
        SlideText::new(stage)?,
        group,
    )))
```

Change to:

```rust
    Ok(SlideContent::new(
        SlideText::new(main)?,
        SlideText::new(translation)?,
        SlideText::new(stage)?,
        group,
    ))
```

- [ ] **Step 4: Restore the call site in `presentation_from_proto`**

Find the call site around line 272:

```rust
            let Some(content) = slide_content_from_proto(base_slide, group)? else {
                continue;
            };
```

Replace with:

```rust
            let content = slide_content_from_proto(base_slide, group)?;
```

- [ ] **Step 5: Restore test `slide_content_treats_single_dot_as_blank`**

Find around line 565-568:

```rust
        let group = Some(SlideGroup::new("Verse 1".to_string()));
        let content = super::slide_content_from_proto(&slide, group)
            .expect("result")
            .expect("slide kept due to group");
        assert!(content.main.value().is_empty(), "main text should be blank");
```

Replace with:

```rust
        let content = super::slide_content_from_proto(&slide, None).expect("content");
        assert!(content.main.value().is_empty(), "main text should be blank");
```

(removes the `group` binding since the function no longer needs it as a "keep-the-slide" trigger).

Also remove the `let group = Some(...)` line above it (the binding becomes unused).

- [ ] **Step 6: Restore test `slide_content_preserves_real_text_when_stage_placeholder_removed`**

Find around line 610-612:

```rust
        let content = super::slide_content_from_proto(&slide, None)
            .expect("result")
            .expect("non-empty slide");
```

Replace with:

```rust
        let content = super::slide_content_from_proto(&slide, None).expect("content");
```

- [ ] **Step 7: Flip `slide_content_returns_none_when_no_elements_and_no_group` into a regression test**

Find around line 620-628:

```rust
    #[test]
    fn slide_content_returns_none_when_no_elements_and_no_group() {
        let slide = proto::Slide::default();
        let result = super::slide_content_from_proto(&slide, None).expect("result");
        assert!(
            result.is_none(),
            "empty slide with no group should be skipped"
        );
    }
```

Replace with:

```rust
    #[test]
    fn slide_content_keeps_empty_slide_with_no_group() {
        // Regression guard for #284: an intentionally-blank slide (no
        // text, no group) is the operator's "clean clear" intro and
        // MUST NOT be skipped during .pro import. See old issue #27.
        let slide = proto::Slide::default();
        let content = super::slide_content_from_proto(&slide, None).expect("content");
        assert!(
            content.main.value().is_empty(),
            "blank intro slide should have empty main"
        );
        assert!(
            content.translation.value().is_empty(),
            "blank intro slide should have empty translation"
        );
        assert!(
            content.stage.value().is_empty(),
            "blank intro slide should have empty stage"
        );
        assert!(
            content.group.is_none(),
            "blank intro slide preserves the absent group"
        );
    }
```

- [ ] **Step 8: Run tests**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
cargo test -p presenter-importer 2>&1 | tail -25
```

Expected: all importer tests pass, including the renamed `slide_content_keeps_empty_slide_with_no_group`.

If a different test fails (e.g., one that depended on the skip-empty-slide behavior elsewhere), read the failure carefully — the revert may need to extend to that test. The diff in `git show 1b874be -- crates/presenter-importer/src/lib.rs` shows the EXACT 3 test changes; if a 4th appears, it was added after `1b874be` and may need separate handling.

- [ ] **Step 9: Workspace fmt + clippy**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
cargo fmt --all --check && echo "FMT OK"
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all 2>&1 | tail -10
```

Expected: fmt clean, clippy clean.

- [ ] **Step 10: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
git add crates/presenter-importer/src/lib.rs
git commit -m "fix(importer): revert skip-empty-slides rule, restore intentional blanks (#284)

Reverts 1b874be (April 13, 2026 — 'skip fully-empty slides with no
group (#215)') in crates/presenter-importer/src/lib.rs. The rule
introduced there also stripped the deliberately-blank intro slides
operators rely on for clean clears (per old issue #27 — 'preserve
the intentionally blank first slide').

User reported in #284 that all .pro library imports were producing
songs with missing intro slides. This commit restores blank slides
from the source bundle.

The third test added in 1b874be is FLIPPED into a regression guard:
slide_content_keeps_empty_slide_with_no_group asserts that an
intentionally-blank slide is kept, preventing future re-introduction
of the bug.

Trade-off: ProPresenter placeholder-only junk slides (the issue #215
problem) come back. Acceptable per user — followup with a smarter
distinguisher if the junk becomes intolerable.

The clipboard-paste parser (song_parser.rs) is untouched per user
instruction; it correctly handles its own empty-section logic via
chunk_to_two_lines + wrap_with_empty_bookends and is unrelated to
this importer fix."
```

---

## Task 3: Push + monitor + verify dev + open PR

This task is controller-handled (NOT a subagent).

- [ ] **Step 1: Final local sanity sweep**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
cargo fmt --all --check && echo "WORKSPACE FMT OK"
cargo test -p presenter-importer 2>&1 | tail -5
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all 2>&1 | tail -3
```

Expected: all three commands succeed.

- [ ] **Step 2: Push**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
git push origin dev
```

- [ ] **Step 3: Identify and monitor the new pipeline**

```bash
gh run list --branch dev --limit 2 --json databaseId,name,status,headSha
```

Note the `databaseId` of the new `Pipeline` run for the latest `headSha`. Monitor with the standard pattern (no `gh run watch`, no polling loops):

```bash
sleep 600 && gh run view <DATABASE_ID> --json status,conclusion,jobs --jq '{status, conclusion, pending: [.jobs[] | select(.status!="completed") | .name], failed: [.jobs[] | select(.conclusion=="failure") | .name]}'
```

Run as a background task. Re-poll every ~10-15 minutes until terminal.

- [ ] **Step 4: Verify all jobs SUCCESS**

```bash
gh run view <DATABASE_ID> --json conclusion,jobs --jq '{conclusion, allSuccess: ([.jobs[] | .conclusion=="success"] | all)}'
```

Expected: `{"conclusion":"success","allSuccess":true}`. If any job failed: `gh run view <DATABASE_ID> --log-failed`, fix in ONE commit, push, monitor again.

- [ ] **Step 5: Verify dev shows v0.4.50**

```bash
curl -s http://10.77.8.134:8080/healthz
```

Expected: `{"channel":"dev","status":"ok","version":"0.4.50"}`.

- [ ] **Step 6: Functional verify — re-import a library and check blank intro slide is back**

Trigger an Import Data run on dev (Actions → "Import Data" → run with environment=dev, library=any one with a known blank intro). After the import completes, query the API to confirm the first slide is blank:

```bash
# Pick a library, list its presentations, then pick one and read its slides
curl -s http://10.77.8.134:8080/api/libraries | jq '.[0:3]'
# After picking a presentation id from the operator UI:
# curl -s http://10.77.8.134:8080/api/presentations/<UUID> | jq '.slides[0]'
```

For the manual sanity check, navigate to `http://10.77.8.134:8080/ui/operator`, open any worship presentation, confirm slide 1 is the intentionally-blank intro. If you can't easily identify a song known to have one, ask the user to point at one in the dev operator.

- [ ] **Step 7: Open the PR**

```bash
gh pr list --base main --head dev --json number --jq 'length'
```

If 0:

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
gh pr create --base main --head dev --title "fix(importer): revert skip-empty-slides rule, restore intentional blanks (#284)" --body "$(cat <<'EOF'
## Summary

Reverts commit \`1b874be\` (April 13, 2026 — 'fix(importer): skip fully-empty slides with no group (#215)'). That rule stripped the deliberately-blank intro slides operators rely on for clean clears (per old issue #27).

User reported in #284 that all .pro library imports were producing songs with missing intro slides after a recent re-import. This PR restores every slide from the ProPresenter bundle, including blanks.

## What changed

- \`crates/presenter-importer/src/lib.rs\`: revert the skip rule. \`slide_content_from_proto\` returns \`Result<SlideContent>\` again (not \`Result<Option<SlideContent>>\`), and empty buckets get a single empty Main bucket so blank slides survive.
- The third test added in \`1b874be\` (originally asserting \"empty slide returns None\") is FLIPPED into a regression guard \`slide_content_keeps_empty_slide_with_no_group\` that asserts the slide IS kept.

The clipboard-paste parser (\`song_parser.rs\`) is untouched per user instruction.

## Trade-off

ProPresenter placeholder-only junk slides (the issue #215 problem) come back. Acceptable per user. If the junk becomes intolerable, a smarter distinguisher can land in a followup PR.

## Test plan

- [x] cargo test -p presenter-importer green (all importer tests including the new regression guard)
- [x] cargo fmt --check + cargo clippy --workspace --all-targets -- -D warnings clean
- [x] After deploy, re-import a library and verify a known song has its blank intro slide restored

## Followup (out of scope)

Issue #228 (seed-library race that may be triggering re-imports on every deploy) — the cause of why the user noticed this regression now, after weeks. Worth fixing separately.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

Confirm mergeable + clean:

```bash
gh api repos/zbynekdrlik/presenter/pulls/<NUMBER> --jq '{mergeable, mergeable_state, head_sha: .head.sha}'
```

Expected: `{"mergeable": true, "mergeable_state": "clean", ...}`.

- [ ] **Step 8: Run /plan-check + /review pre-completion gate**

Per `~/devel/airuleset/modules/core/completion-report.md`. Fix any 🔴 / 🟡 / 🔵 inside the diff before sending the completion report.

- [ ] **Step 9: Send completion report**

Use the EXACT template from `~/devel/airuleset/modules/core/completion-report.md` (audits at top, Goal/What changed/URLs/PR at bottom; both 🌐 lines for Dev + Prod).

DO NOT merge. The user merges via explicit "merge it" instruction per `pr-merge-policy.md`. Production will get the fix only after merge + automatic deploy on push to main.

---

## Verification Summary

| Check | How to verify |
|-------|---------------|
| Importer keeps blank intro slides | New regression test `slide_content_keeps_empty_slide_with_no_group` passes |
| All other importer tests still pass | `cargo test -p presenter-importer` green |
| Workspace clippy clean | `cargo clippy --workspace --all-targets -- -D warnings` exits 0 |
| Dev shows v0.4.50 | `/healthz` reports 0.4.50; operator UI DOM shows v0.4.50 (dev) |
| Re-imported song has blank intro | Open the song on dev operator after a fresh Import Data run; first slide is empty |
| Clipboard parser untouched | `git diff origin/main..origin/dev -- crates/presenter-ui/src/utils/song_parser.rs` shows no changes |
| All CI jobs green | Pipeline run reports `allSuccess: true` |
