# Clipboard Import: Split Slides on Section Markers Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the operator's clipboard-paste presentation creator split a song into one slide per section (Verse N, Chorus, Bridge, …), filtering ProPresenter-style metadata markers (`Title:`, `Misc N`, `^B`).

**Architecture:** Replace the blank-line-only splitter `parse_song_text` with a line-walk parser that flushes on section markers AND blank lines, and filters metadata. Move it out of `test_helpers.rs` (where it inappropriately lived as production code) into a new `song_parser.rs` utility module.

**Tech Stack:** Rust + Leptos (WASM), Playwright/TypeScript.

**Spec:** `docs/superpowers/specs/2026-04-29-clipboard-import-section-split-design.md` (commit `c93d133`).

---

## Context

`parse_song_text` is used by:
1. The operator's "create presentation from paste" modal (`crates/presenter-ui/src/components/presentation_modal.rs:244`).
2. The JS test bridge `window.__presenterOperatorTestHelpers.parseSongText` (`crates/presenter-ui/src/utils/test_helpers.rs:328-335`), exercised by E2E test `tests/e2e/wasm-edge-cases.spec.ts:259` (`"parseSongText handles verse markers"`).

After this work the implementation moves but the JS-bridge name `parseSongText` stays the same, so the existing E2E test keeps working — it's actually a regression guard for the move.

`presenter-ui` is excluded from the root workspace and has its own `Cargo.lock`. Tests via `cargo test -p presenter-ui --target x86_64-unknown-linux-gnu`. Clippy via `cd crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown -- -D warnings -W clippy::all`. Local Rust builds are allowed on this dev machine.

`SlideInput` is defined in `crates/presenter-ui/src/api/presentations.rs` (already imported in `test_helpers.rs`).

---

## File Structure

| File | Change |
|------|--------|
| `Cargo.toml` (workspace `[workspace.package]`) | `0.4.45` → `0.4.46` |
| `crates/presenter-ui/Cargo.toml` | `0.1.14` → `0.1.15` |
| `crates/presenter-ui/src/utils/song_parser.rs` | **NEW.** Public `parse_song_text(text: &str) -> Vec<SlideInput>` with private helpers `is_group_line` and `is_metadata_line`. ~10 unit tests. |
| `crates/presenter-ui/src/utils/mod.rs` | Add `pub mod song_parser;` |
| `crates/presenter-ui/src/utils/test_helpers.rs` | Remove the local `parse_song_text` and `is_group_line` functions (lines 345-414). Update the `parseSongText` JS bridge to call `crate::utils::song_parser::parse_song_text` instead. |
| `crates/presenter-ui/src/components/presentation_modal.rs:244` | Change call from `crate::utils::test_helpers::parse_song_text` → `crate::utils::song_parser::parse_song_text` |

---

## Task 1: Bump version to 0.4.46

**Files:**
- Modify: `Cargo.toml` (workspace `[workspace.package]`)
- Modify: `crates/presenter-ui/Cargo.toml`

- [ ] **Step 1: Confirm versions**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
git fetch origin
grep '^version' Cargo.toml | head -1
grep '^version' crates/presenter-ui/Cargo.toml | head -1
```

Expected: workspace `0.4.45`, presenter-ui `0.1.14`.

- [ ] **Step 2: Bump workspace**

In `/home/newlevel/devel/presenter/presenter-dev2/Cargo.toml` under `[workspace.package]`, change `version = "0.4.45"` to `version = "0.4.46"`.

- [ ] **Step 3: Bump presenter-ui**

In `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui/Cargo.toml` under `[package]`, change `version = "0.1.14"` to `version = "0.1.15"`.

- [ ] **Step 4: Refresh Cargo.lock files**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
cargo check -p presenter-server 2>&1 | tail -3
cd crates/presenter-ui && cargo check --target wasm32-unknown-unknown 2>&1 | tail -3 && cd ../..
```

Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock crates/presenter-ui/Cargo.toml crates/presenter-ui/Cargo.lock
git commit -m "chore: bump version to 0.4.46 (#275)"
```

---

## Task 2: Create song_parser.rs with TDD

**Files:**
- Create: `crates/presenter-ui/src/utils/song_parser.rs`
- Modify: `crates/presenter-ui/src/utils/mod.rs`

- [ ] **Step 1: Add the module declaration**

Edit `crates/presenter-ui/src/utils/mod.rs`. Find the existing `pub mod` lines (use `cat /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui/src/utils/mod.rs` to see them) and add:

```rust
pub mod song_parser;
```

- [ ] **Step 2: Create the new file with failing tests + skeleton**

Create `crates/presenter-ui/src/utils/song_parser.rs` with the following content:

```rust
//! Song-text parser for the clipboard-paste "create presentation" flow.
//!
//! Splits a pasted song into one [`SlideInput`] per section. Triggers:
//! - Metadata lines (`Title:`, `Misc N`, `^[A-Z]`) → filter out.
//! - Group lines (`Verse N`, `Chorus`, `Bridge`, …) → flush current slide,
//!   start a new one with this line as the group name.
//! - Blank lines → flush current slide if non-empty.
//! - Anything else → append to the current slide's main text.
//!
//! See `docs/superpowers/specs/2026-04-29-clipboard-import-section-split-design.md`.

use crate::api::presentations::SlideInput;

/// Parse a pasted song into one [`SlideInput`] per section.
pub fn parse_song_text(text: &str) -> Vec<SlideInput> {
    let mut slides: Vec<SlideInput> = Vec::new();
    let mut current_group: Option<String> = None;
    let mut current_main: Vec<String> = Vec::new();

    for raw_line in text.lines() {
        let trimmed = raw_line.trim();

        if is_metadata_line(trimmed) {
            continue;
        }

        if is_group_line(trimmed) {
            flush_slide(&mut slides, &mut current_group, &mut current_main);
            current_group = Some(trimmed.to_string());
            continue;
        }

        if trimmed.is_empty() {
            // Only flush if we've accumulated content; otherwise ignore.
            if current_group.is_some() || !current_main.is_empty() {
                flush_slide(&mut slides, &mut current_group, &mut current_main);
            }
            continue;
        }

        current_main.push(raw_line.to_string());
    }

    flush_slide(&mut slides, &mut current_group, &mut current_main);
    slides
}

fn flush_slide(
    slides: &mut Vec<SlideInput>,
    group: &mut Option<String>,
    main_lines: &mut Vec<String>,
) {
    let main = main_lines.join("\n").trim_end().to_string();
    let took_group = group.take();
    main_lines.clear();
    if main.is_empty() && took_group.is_none() {
        return;
    }
    slides.push(SlideInput {
        main,
        translation: None,
        stage: None,
        group: took_group,
    });
}

/// Recognised section/group headings (the ProPresenter-style group names).
/// Mirrors the original list from `test_helpers.rs::is_group_line` and accepts
/// optional trailing digits / whitespace (e.g. `Verse 2`, `Chorus`).
fn is_group_line(line: &str) -> bool {
    let line_lower = line.trim().to_lowercase();
    let patterns = [
        "verse",
        "chorus",
        "bridge",
        "intro",
        "outro",
        "pre-chorus",
        "prechorus",
        "tag",
        "interlude",
        "refrain",
        "hook",
        "coda",
        "ending",
        "instrumental",
    ];
    for pattern in patterns {
        if let Some(rest) = line_lower.strip_prefix(pattern) {
            let rest = rest.trim();
            if rest.is_empty()
                || rest
                    .chars()
                    .all(|c| c.is_ascii_digit() || c.is_whitespace())
            {
                return true;
            }
        }
    }
    false
}

/// ProPresenter-style metadata / control markers we want to silently drop:
/// - `Title:` prefix (case-insensitive)
/// - `Misc <digits>` (case-insensitive)
/// - `^X` where X is a single ASCII uppercase letter (control marker)
fn is_metadata_line(line: &str) -> bool {
    if line.is_empty() {
        return false;
    }
    let lower = line.to_lowercase();
    if lower.starts_with("title:") {
        return true;
    }
    // ^B / ^A / ^C etc. — exactly two chars: caret + uppercase ASCII letter.
    let bytes = line.as_bytes();
    if bytes.len() == 2 && bytes[0] == b'^' && bytes[1].is_ascii_uppercase() {
        return true;
    }
    // "Misc 1", "MISC 12", etc.
    if let Some(rest) = lower.strip_prefix("misc ") {
        if !rest.is_empty()
            && rest
                .chars()
                .all(|c| c.is_ascii_digit() || c.is_whitespace())
        {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_returns_no_slides() {
        assert!(parse_song_text("").is_empty());
        assert!(parse_song_text("   \n\n  ").is_empty());
    }

    #[test]
    fn user_pasted_song_splits_into_one_slide_per_section() {
        // Regression guard for issue #275.
        let input = "Title: 326 Všetko, čo v sebe držím \n\
            Misc 1\n\
            ^B\n\
            Verse 1\n\
            Všetko, čo v sebe držím: s túžbami, nádejami aj\n\
            s vysnívaným!\n\
            Ty ma chceš Pane... aj s tým všetkým, čo mám ja.\n\
            \n\
            Chorus\n\
            Mám Spasiteľa, žije vo mne, ó, ó,\n\
            chcem viac, chcem Teba spoznávať viac.\n\
            \n\
            Verse 2\n\
            Všetko, čo v sebe držím: s túžbami, nádejami aj\n\
            s vysnívaným!\n\
            \n\
            Interlude\n\
            \n\
            Outro\n\
            Nikto mi Ťa už nemôže vziať!\n\
            Misc 1\n\
            ^B\n";
        let slides = parse_song_text(input);
        let groups: Vec<Option<&str>> =
            slides.iter().map(|s| s.group.as_deref()).collect();
        assert_eq!(
            groups,
            vec![
                Some("Verse 1"),
                Some("Chorus"),
                Some("Verse 2"),
                Some("Interlude"),
                Some("Outro"),
            ],
            "expected one slide per section, no Title/Misc/^B leakage"
        );
        // No slide's main contains Title:, Misc, or ^B.
        for s in &slides {
            assert!(
                !s.main.to_lowercase().contains("title:")
                    && !s.main.to_lowercase().contains("misc ")
                    && !s.main.contains("^B"),
                "slide leaked metadata: {:?}",
                s
            );
        }
        // Outro slide should contain only the Nikto... lyric (Misc/^B trailing
        // metadata filtered).
        let outro = slides.iter().find(|s| s.group.as_deref() == Some("Outro")).unwrap();
        assert_eq!(outro.main, "Nikto mi Ťa už nemôže vziať!");
    }

    #[test]
    fn group_line_in_middle_of_paragraph_starts_new_slide() {
        // No blank line between Verse and Chorus.
        let input = "Verse 1\nlyric a\nlyric b\nChorus\nlyric c";
        let slides = parse_song_text(input);
        assert_eq!(slides.len(), 2);
        assert_eq!(slides[0].group.as_deref(), Some("Verse 1"));
        assert_eq!(slides[0].main, "lyric a\nlyric b");
        assert_eq!(slides[1].group.as_deref(), Some("Chorus"));
        assert_eq!(slides[1].main, "lyric c");
    }

    #[test]
    fn metadata_lines_are_filtered() {
        let input = "Title: My Song\n\nVerse 1\nlyric";
        let slides = parse_song_text(input);
        assert_eq!(slides.len(), 1);
        assert_eq!(slides[0].group.as_deref(), Some("Verse 1"));
        assert_eq!(slides[0].main, "lyric");
    }

    #[test]
    fn caret_uppercase_filtered_but_caret_alone_is_text() {
        let s1 = parse_song_text("Verse 1\n^B\nlyric");
        // ^B filtered, slide has just "lyric"
        assert_eq!(s1.len(), 1);
        assert_eq!(s1[0].main, "lyric");

        let s2 = parse_song_text("Verse 1\n^\nlyric");
        // Bare ^ is NOT filtered — treated as text.
        assert_eq!(s2.len(), 1);
        assert_eq!(s2[0].main, "^\nlyric");
    }

    #[test]
    fn misc_n_filtered_but_miscellaneous_is_text() {
        let s1 = parse_song_text("Verse 1\nMisc 1\nlyric");
        assert_eq!(s1.len(), 1);
        assert_eq!(s1[0].main, "lyric");

        let s2 = parse_song_text("Verse 1\nMiscellaneous things\nlyric");
        // "Miscellaneous" is NOT filtered — treated as text.
        assert_eq!(s2.len(), 1);
        assert_eq!(s2[0].main, "Miscellaneous things\nlyric");
    }

    #[test]
    fn empty_interlude_emits_slide_with_empty_main() {
        let input = "Interlude\n\nChorus\nlyric";
        let slides = parse_song_text(input);
        assert_eq!(slides.len(), 2);
        assert_eq!(slides[0].group.as_deref(), Some("Interlude"));
        assert_eq!(slides[0].main, "");
        assert_eq!(slides[1].group.as_deref(), Some("Chorus"));
        assert_eq!(slides[1].main, "lyric");
    }

    #[test]
    fn loose_paragraphs_without_groups_split_on_blank_lines() {
        let input = "Stanza 1\nline 2\n\nStanza 3\nline 4";
        let slides = parse_song_text(input);
        assert_eq!(slides.len(), 2);
        assert!(slides[0].group.is_none());
        assert_eq!(slides[0].main, "Stanza 1\nline 2");
        assert!(slides[1].group.is_none());
        assert_eq!(slides[1].main, "Stanza 3\nline 4");
    }

    #[test]
    fn group_line_is_case_insensitive() {
        let input = "chorus\nlower\n\nCHORUS\nupper\n\nVerse 2\nnumbered";
        let slides = parse_song_text(input);
        assert_eq!(slides.len(), 3);
        assert_eq!(slides[0].group.as_deref(), Some("chorus"));
        assert_eq!(slides[1].group.as_deref(), Some("CHORUS"));
        assert_eq!(slides[2].group.as_deref(), Some("Verse 2"));
    }

    #[test]
    fn lyric_indentation_preserved() {
        let input = "Verse 1\n  indented\n    more indent\nflush";
        let slides = parse_song_text(input);
        assert_eq!(slides.len(), 1);
        assert_eq!(slides[0].main, "  indented\n    more indent\nflush");
    }
}
```

- [ ] **Step 3: Run the new tests**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
cargo test -p presenter-ui --target x86_64-unknown-linux-gnu --lib song_parser 2>&1 | tail -20
```

Expected: all 10 tests pass.

If any test fails, fix the implementation. **Do NOT lower test expectations** — every assertion is a real requirement from the spec.

- [ ] **Step 4: Run clippy on the new module**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui
cargo clippy --target wasm32-unknown-unknown -- -D warnings 2>&1 | tail -10
cd ../..
```

Expected: zero warnings.

- [ ] **Step 5: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
cargo fmt --all
git add crates/presenter-ui/src/utils/mod.rs crates/presenter-ui/src/utils/song_parser.rs
git commit -m "feat(ui): add song_parser module with section-aware splitting (#275)

Line-walk parser with three triggers:
- Metadata lines (Title:/Misc N/^[A-Z]) -> filter
- Group lines (Verse/Chorus/Bridge/...) -> flush + start new slide
- Blank lines -> flush if non-empty

10 unit tests covering: user's exact pasted input from the issue
(regression guard), mid-paragraph group split, metadata filtering,
empty Interlude -> empty slide with group, loose paragraphs without
groups (preserves old behavior), case-insensitive group lines,
lyric indentation preservation."
```

---

## Task 3: Wire the new module into call sites

**Files:**
- Modify: `crates/presenter-ui/src/utils/test_helpers.rs`
- Modify: `crates/presenter-ui/src/components/presentation_modal.rs`

- [ ] **Step 1: Update presentation_modal.rs to use the new module**

In `crates/presenter-ui/src/components/presentation_modal.rs`, line 244, replace:

```rust
            let slides = crate::utils::test_helpers::parse_song_text(&text);
```

with:

```rust
            let slides = crate::utils::song_parser::parse_song_text(&text);
```

- [ ] **Step 2: Update test_helpers.rs to use the new module**

In `crates/presenter-ui/src/utils/test_helpers.rs`, the JS bridge at line 331 currently calls the local `parse_song_text` (defined later in the same file). Update it to call the new module:

Old (line 331):
```rust
            let slides = parse_song_text(&text);
```

New:
```rust
            let slides = crate::utils::song_parser::parse_song_text(&text);
```

- [ ] **Step 3: Remove the now-unused functions from test_helpers.rs**

Delete the entire `is_group_line` function (lines 345-378) and the entire `parse_song_text` function (lines 380-414) from `crates/presenter-ui/src/utils/test_helpers.rs`. Also remove any `use` of `SlideInput` if it's no longer referenced in this file (verify with `grep -n SlideInput crates/presenter-ui/src/utils/test_helpers.rs` after the deletion — if no matches remain except in the import, remove the import too).

- [ ] **Step 4: Build and clippy**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui
cargo build --target wasm32-unknown-unknown 2>&1 | tail -5
cargo clippy --target wasm32-unknown-unknown -- -D warnings 2>&1 | tail -10
cd ../..
```

Expected: clean build, zero warnings. If clippy complains about unused imports in `test_helpers.rs`, remove them.

- [ ] **Step 5: Run all presenter-ui tests**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
cargo test -p presenter-ui --target x86_64-unknown-linux-gnu 2>&1 | tail -10
```

Expected: all tests pass (the existing presenter-ui suite plus the 10 new song_parser tests).

- [ ] **Step 6: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
cargo fmt --all
git add crates/presenter-ui/src/utils/test_helpers.rs crates/presenter-ui/src/components/presentation_modal.rs
git commit -m "refactor(ui): wire song_parser into modal + test-helpers bridge (#275)

Move the parse_song_text + is_group_line implementations out of
test_helpers.rs (where they lived inappropriately as production
code) into the new song_parser module. The JS bridge
window.__presenterOperatorTestHelpers.parseSongText keeps the
same name and signature, so the existing
tests/e2e/wasm-edge-cases.spec.ts:259 'parseSongText handles
verse markers' E2E test continues to work unchanged."
```

---

## Task 4: Local checks + push + monitor pipeline CI

- [ ] **Step 1: Workspace fmt + clippy + tests**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all 2>&1 | tail -5
cargo test --workspace 2>&1 | tail -5
```

Expected: every command exits 0, clippy zero warnings, all tests pass.

- [ ] **Step 2: presenter-ui specifically**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui
cargo clippy --target wasm32-unknown-unknown -- -D warnings 2>&1 | tail -5
cargo test --target x86_64-unknown-linux-gnu 2>&1 | tail -5
cd ../..
```

Expected: clean.

- [ ] **Step 3: Push**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
git push origin dev
```

- [ ] **Step 4: Find new pipeline run**

```bash
gh run list --branch dev --limit 3 --json databaseId,name,event,headSha,status,conclusion --jq '.[] | select(.name == "Pipeline" and .event == "push")' | head -1
```

- [ ] **Step 5: Monitor (single background poll)**

Per `ci-monitoring.md` — single background command, no repeated polling:

```bash
sleep 1500 && gh run view <RUN_ID> --json status,conclusion,jobs --jq '{status, conclusion, jobs: [.jobs[] | {name, status, conclusion}]}'
```

When the poll returns: every job must have `conclusion: success`. If anything failed, fetch failed log (`gh run view <RUN_ID> --log-failed`), fix the root cause in ONE commit, push, and monitor again (one poll iteration only).

Expected: ALL jobs green incl. Mutation Testing, Playwright E2E 1/3+2/3+3/3, Deploy to Dev. The `wasm-edge-cases.spec.ts` E2E test "parseSongText handles verse markers" (which calls the `parseSongText` JS bridge) is the regression guard for the move — if it fails, the bridge isn't wired correctly.

---

## Task 5: Verify on dev + open PR + merge

- [ ] **Step 1: Verify dev healthz**

```bash
curl -s http://10.77.8.134:8080/healthz
```

Expected: `{"channel":"dev","status":"ok","version":"0.4.46"}`.

- [ ] **Step 2: Live verify the fix**

Use Playwright MCP to verify the user's exact reproduction:

1. Navigate to `http://10.77.8.134:8080/ui/operator`, wait for `body[data-wasm-ready="true"]`.
2. Run via `browser_evaluate`:

```js
const text = `Title: 326 Všetko, čo v sebe držím
Misc 1
^B
Verse 1
Všetko, čo v sebe držím: s túžbami, nádejami aj
s vysnívaným!
Ty ma chceš Pane... aj s tým všetkým, čo mám ja.

Chorus
Mám Spasiteľa, žije vo mne, ó, ó,
chcem viac, chcem Teba spoznávať viac.

Verse 2
Všetko, čo v sebe držím: s túžbami, nádejami aj
s vysnívaným!

Interlude

Outro
Nikto mi Ťa už nemôže vziať!
Misc 1
^B`;
const result = window.__presenterOperatorTestHelpers.parseSongText(text);
return {
  count: result.length,
  groups: result.map(s => s.group),
  hasMetadataLeak: result.some(s =>
    /title:|misc |\^[A-Z]/i.test(s.main)
  ),
};
```

Expected:
- `count`: 5
- `groups`: `["Verse 1", "Chorus", "Verse 2", "Interlude", "Outro"]`
- `hasMetadataLeak`: `false`

3. Browser console: zero errors / warnings.

- [ ] **Step 3: Open PR dev → main**

```bash
gh pr create --base main --head dev --title "fix(ui): clipboard import splits song into one slide per section (#275)" --body "$(cat <<'EOF'
## Summary
Fixes #275. The clipboard-paste presentation creator now splits a pasted song into one slide per section (Verse, Chorus, Bridge, Outro, Interlude, …) and filters ProPresenter metadata markers (`Title:`, `Misc N`, `^B`).

## Changes
- New `crates/presenter-ui/src/utils/song_parser.rs` line-walk parser
- Move `parse_song_text` + `is_group_line` out of `test_helpers.rs` (they were production code mis-located)
- Update the one production call site (`presentation_modal.rs:244`) and the JS test bridge to import from the new module
- 10 new unit tests including the user's exact pasted input as a regression guard

## Test plan
- [x] CI green (all 21 jobs incl. Mutation Testing, Playwright E2E 1/3+2/3+3/3, Deploy to Dev, Deploy Companion Plugin)
- [x] Existing `tests/e2e/wasm-edge-cases.spec.ts` "parseSongText handles verse markers" passes — proves the JS bridge move is correct
- [x] Live verification on dev: pasted user's example into `parseSongText` JS helper, got 5 slides with the expected group names, no metadata leakage
EOF
)"
```

- [ ] **Step 4: Wait for PR pipeline + verify CLEAN**

```bash
gh pr view <PR_NUMBER> --json mergeable,mergeStateStatus,url
```

Expected: `MERGEABLE` + `CLEAN`. If `UNSTABLE`, identify which check is pending (most likely Mutation Testing) and wait — do NOT propose a bypass.

- [ ] **Step 5: Send completion report and wait for explicit "merge it"**

Per `pr-merge-policy.md`, do NOT merge. Send the completion report (per `completion-report.md`) with the PR URL and wait.

---

## Task 6: Post-merge production verify (gated on user merge instruction)

- [ ] **Step 1: Watch the post-merge main pipeline**

```bash
gh run list --branch main --limit 3 --json databaseId,name,event,status,conclusion --jq '.[] | select(.event == "push")' | head -1
sleep 1500 && gh run view <RUN_ID> --json status,conclusion,jobs ...
```

Wait for ALL jobs green incl. `Deploy to Production`.

- [ ] **Step 2: Verify production**

```bash
curl -s http://10.77.9.205/healthz
```

Expected: `{"channel":"release","status":"ok","version":"0.4.46"}`.

Repeat the live `parseSongText` Playwright check from Task 5 step 2 against `http://10.77.9.205/ui/operator`. Same pasted text, same expected output.

- [ ] **Step 3: Bump dev version (post-release lifecycle)**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
git fetch origin
git checkout dev
git pull origin main 2>&1 | tail -3
sed -i 's/^version = "0.4.46"$/version = "0.4.47"/' Cargo.toml
sed -i 's/^version = "0.1.15"$/version = "0.1.16"/' crates/presenter-ui/Cargo.toml
cargo check -p presenter-server 2>&1 | tail -3
cd crates/presenter-ui && cargo check --target wasm32-unknown-unknown 2>&1 | tail -3 && cd ../..
git add Cargo.toml Cargo.lock crates/presenter-ui/Cargo.toml crates/presenter-ui/Cargo.lock
git commit -m "chore: bump version to 0.4.47 (post-release)"
git push origin dev
```

- [ ] **Step 4: Final completion report**

Short report per `completion-report.md` with prod-verified URLs. No follow-up question.

---

## Verification Summary

| Check | How |
|-------|-----|
| User's exact pasted input parses correctly | `song_parser::tests::user_pasted_song_splits_into_one_slide_per_section` |
| Metadata lines filtered | `song_parser::tests::metadata_lines_are_filtered`, `caret_uppercase_filtered_but_caret_alone_is_text`, `misc_n_filtered_but_miscellaneous_is_text` |
| Section markers split mid-paragraph | `song_parser::tests::group_line_in_middle_of_paragraph_starts_new_slide` |
| Loose paragraphs still split on blank lines | `song_parser::tests::loose_paragraphs_without_groups_split_on_blank_lines` |
| Empty Interlude emits empty-main slide with group | `song_parser::tests::empty_interlude_emits_slide_with_empty_main` |
| JS bridge `parseSongText` still works | `tests/e2e/wasm-edge-cases.spec.ts:259` (existing test, unchanged) |
| Live dev verification | Playwright MCP probe with user's exact input |
| Browser console clean | Live dev verification asserts 0 errors / 0 warnings |
