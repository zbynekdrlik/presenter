# Worship-PP Bigger Sidebar Fonts Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make worship-pp stage playlist sidebar text large enough that ~12 characters fit per row at 1080p projector — increase font from 2.6vh to 5vh, tighten sidebar and entry padding so the 22% sidebar's inner width is fully usable.

**Architecture:** Pure CSS iteration on top of PR #268. Touches one CSS file (4 rules in `crates/presenter-ui/styles/stage.css`) and tightens one E2E assertion. No WASM, server, or layout changes.

**Tech Stack:** CSS, Playwright/TypeScript.

**Spec:** `docs/superpowers/specs/2026-04-28-worship-pp-bigger-fonts-design.md` (commit `57ddbde`).

---

## Context

Stacks on top of PR #268 (still open, mergeable+clean). Dev is at 0.4.38; bump to 0.4.39 / presenter-ui 0.1.8 first (per CLAUDE.md "version-bumping" rule — bump before any code, so the CI version-check job doesn't fail).

`presenter-ui` is excluded from the root workspace and has its own `Cargo.lock`. Tests run via `cargo test -p presenter-ui --target x86_64-unknown-linux-gnu`. Clippy via `cd crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown -- -D warnings -W clippy::all`. Local Rust builds are allowed on this dev machine.

Working directory: `/home/newlevel/devel/presenter/presenter-dev2`.

The existing E2E `tests/e2e/stage-worship-pp.spec.ts` test "sidebar is narrower (~22%) and entries have projector-readable font" sets viewport to `1920×1080` and asserts `entryFontSizePx >= 24`. After the change, computed font is 5vh × 1080 = 54px, well above 40. We tighten the floor to 40 to lock in the larger size.

---

## File Structure

| File | Change |
|------|--------|
| `Cargo.toml` (workspace) | `0.4.38` → `0.4.39` |
| `crates/presenter-ui/Cargo.toml` | `0.1.7` → `0.1.8` |
| `crates/presenter-ui/styles/stage.css` | Replace 3 worship-pp rules (sidebar padding, entry font/padding, active padding-left) |
| `tests/e2e/stage-worship-pp.spec.ts` | Tighten font-size floor `>= 24` → `>= 40` |

---

## Task 1: Bump version to 0.4.39

**Files:**
- Modify: `Cargo.toml` (workspace `[workspace.package]`)
- Modify: `crates/presenter-ui/Cargo.toml`

- [ ] **Step 1: Confirm versions**

```bash
git fetch origin
grep '^version' Cargo.toml | head -1
grep '^version' crates/presenter-ui/Cargo.toml | head -1
gh release list --limit 1
```

Expected: workspace `0.4.38`, presenter-ui `0.1.7`, latest release `v0.4.26`.

- [ ] **Step 2: Bump workspace**

In `/home/newlevel/devel/presenter/presenter-dev2/Cargo.toml` under `[workspace.package]`, change `version = "0.4.38"` to `version = "0.4.39"`.

- [ ] **Step 3: Bump presenter-ui**

In `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui/Cargo.toml` under `[package]`, change `version = "0.1.7"` to `version = "0.1.8"`.

- [ ] **Step 4: Refresh both Cargo.lock files**

```bash
cargo check -p presenter-server 2>&1 | tail -3
cd crates/presenter-ui && cargo check --target wasm32-unknown-unknown 2>&1 | tail -3 && cd ../..
```

Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock crates/presenter-ui/Cargo.toml crates/presenter-ui/Cargo.lock
git commit -m "chore: bump version to 0.4.39 (#worship-pp-bigger-fonts)"
```

---

## Task 2: CSS — bigger font + tighter padding

**Files:**
- Modify: `crates/presenter-ui/styles/stage.css`

- [ ] **Step 1: Verify current line numbers**

```bash
grep -n "stage-pp__playlist-sidebar\|stage-pp__playlist-entry" crates/presenter-ui/styles/stage.css
```

Expected: `.stage-pp__playlist-sidebar`, `.stage-pp__playlist-entry`, `.stage-pp__playlist-entry--active` all present from PR #268.

- [ ] **Step 2: Replace `.stage-pp__playlist-sidebar`**

In `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui/styles/stage.css`, find the existing `.stage-pp__playlist-sidebar` rule (with `padding: 1% 1.5%`) and replace the whole rule with:

```css
.stage-pp__playlist-sidebar {
    position: absolute;
    right: 0;
    top: 0;
    width: 22%;
    height: 92%;
    overflow-y: auto;
    border-left: 1px solid rgba(255, 255, 255, 0.1);
    padding: 0.4% 0.5%;
    box-sizing: border-box;
}
```

(Only the `padding` line changes from `1% 1.5%` to `0.4% 0.5%`. Width, height, position, overflow, border, box-sizing stay.)

- [ ] **Step 3: Replace `.stage-pp__playlist-entry`**

Find the existing `.stage-pp__playlist-entry` rule (with `padding: 0.6vh 0.8rem` and `font-size: 2.6vh`) and replace it with:

```css
.stage-pp__playlist-entry {
    padding: 0.3vh 0.4rem;
    color: #94a3b8;
    /* Sized so ~12 typical chars fit per row at 1080p:
       sidebar 22% × 1920 = 422px, less padding → ~390px content,
       at 5vh font (≈54px) char-width is ~30px, ~12 chars per row. */
    font-size: 5vh;
    line-height: 1.1;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    border-radius: 4px;
    margin-bottom: 2px;
}
```

(Changes: `padding` value, `font-size` value, the inline comment is updated to reflect the new math.)

- [ ] **Step 4: Replace `.stage-pp__playlist-entry--active`**

Find the existing `.stage-pp__playlist-entry--active` rule (with `padding-left: calc(0.8rem - 4px)`) and replace it with:

```css
.stage-pp__playlist-entry--active {
    background: #38bdf8;
    color: #0f172a;
    font-weight: 700;
    border-left: 4px solid #0ea5e9;
    /* Compensate for the 4px border so text alignment with non-active rows
       stays consistent. Base padding-left is 0.4rem (≈6.4px); subtract the
       border width. */
    padding-left: calc(0.4rem - 4px);
}
```

(Only `padding-left` value changes from `calc(0.8rem - 4px)` to `calc(0.4rem - 4px)`, and the comment updates to match.)

- [ ] **Step 5: Confirm there are no other `2.6vh` or `0.6vh 0.8rem` references in the CSS file**

```bash
grep -nE "2\.6vh|0\.6vh 0\.8rem|0\.8rem - 4px|1% 1\.5%" crates/presenter-ui/styles/stage.css
```

Expected: no matches (all replaced).

- [ ] **Step 6: Commit**

```bash
git add crates/presenter-ui/styles/stage.css
git commit -m "fix(stage): worship-pp playlist text big enough for 12 chars per row (#worship-pp-bigger-fonts)

Sidebar padding 1% 1.5% → 0.4% 0.5%, entry padding 0.6vh 0.8rem
→ 0.3vh 0.4rem, entry font-size 2.6vh → 5vh. At 1080p projector,
the 22% sidebar's ~390px content area fits ~12 typical characters
at the new 54px font. Active rule's padding-left compensation
recalculated to match the new base padding."
```

---

## Task 3: E2E — tighten font-size floor

**Files:**
- Modify: `tests/e2e/stage-worship-pp.spec.ts`

- [ ] **Step 1: Find the existing assertion**

```bash
grep -n "entryFontSizePx\|>= 24" tests/e2e/stage-worship-pp.spec.ts
```

Expected: a line `expect(measurements.entryFontSizePx).toBeGreaterThanOrEqual(24);` inside the test "sidebar is narrower (~22%) and entries have projector-readable font".

- [ ] **Step 2: Tighten the floor from 24 to 40**

Replace the line:

```typescript
      expect(measurements.entryFontSizePx).toBeGreaterThanOrEqual(24);
```

with:

```typescript
      // Floor of 40px locks in the 5vh × 1080p = 54px font from
      // #worship-pp-bigger-fonts; any accidental drop back to 2.6vh
      // (≈28px) will fail the test.
      expect(measurements.entryFontSizePx).toBeGreaterThanOrEqual(40);
```

- [ ] **Step 3: Format**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
npx prettier --write tests/e2e/stage-worship-pp.spec.ts 2>&1 | tail -3
```

- [ ] **Step 4: Run the test locally to confirm it passes against the new CSS**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
npx playwright test tests/e2e/stage-worship-pp.spec.ts --grep "sidebar is narrower" 2>&1 | tail -20
```

Expected: PASS (computed font-size at 1920×1080 is 54px, ≥ 40).

If the test fails because the local Trunk dist hasn't rebuilt: run `scripts/build-ui.sh` (or `cd crates/presenter-ui && trunk build --release && cd ../..`) and re-run. The CI pipeline rebuilds dist on every push.

- [ ] **Step 5: Commit**

```bash
git add tests/e2e/stage-worship-pp.spec.ts
git commit -m "test(e2e): tighten worship-pp font-size floor to 40px (#worship-pp-bigger-fonts)

Locks in the new 5vh font (≈54px @ 1080p). Any accidental
regression to the previous 2.6vh (≈28px) fails the test."
```

---

## Task 4: Local checks + push + monitor pipeline CI

- [ ] **Step 1: Workspace fmt + clippy + test**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all 2>&1 | tail -5
cargo test --workspace 2>&1 | tail -10
```

Expected: every command exits 0, clippy zero warnings, all tests pass.

- [ ] **Step 2: presenter-ui specifically**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui
cargo clippy --target wasm32-unknown-unknown -- -D warnings 2>&1 | tail -5
cargo test --target x86_64-unknown-linux-gnu --lib 2>&1 | tail -5
cd ../..
```

Expected: clean.

- [ ] **Step 3: Push**

```bash
git push origin dev
```

- [ ] **Step 4: Find the new pipeline run**

```bash
gh run list --branch dev --limit 3 --json databaseId,name,event,headSha,status,conclusion --jq '.[] | select(.name == "Pipeline" and .event == "push")' | head -1
```

- [ ] **Step 5: Monitor pipeline (single background poll)**

Per `ci-monitoring.md` — single background command, no repeated polling:

```bash
sleep 1500 && gh run view <RUN_ID> --json status,conclusion,jobs --jq '{status, conclusion, jobs: [.jobs[] | {name, status, conclusion}]}'
```

When that returns, if everything is `success` we proceed. If anything failed, fetch failed log (`gh run view <RUN_ID> --log-failed`), fix the root cause in ONE commit, push, and monitor again.

Expected: ALL jobs green (Format, Clippy, Test, Build, Code Coverage, Mutation Testing, Playwright E2E 1/3+2/3+3/3, Merge E2E Reports, Deploy to Dev).

---

## Task 5: Verify on dev + update PR #268

- [ ] **Step 1: Verify dev healthz**

```bash
curl -s http://10.77.8.134:8080/healthz
```

Expected: `{"channel":"dev","status":"ok","version":"0.4.39"}`.

- [ ] **Step 2: Live verify the bigger font**

Use Playwright MCP:

1. `POST http://10.77.8.134:8080/stage/layout` with body `{"code":"worship-pp"}` to switch the stage layout.
2. Open `http://10.77.8.134:8080/stage` in Playwright. Wait for `body[data-layout-code="worship-pp"]` and `.stage-pp__playlist-entry`.
3. Read `getComputedStyle(entry).fontSize`. Compute `parseFloat(fontSize) / window.innerHeight`. Expected ratio: ≈ 0.05 (= 5vh).
4. Read sidebar `getBoundingClientRect()`. Confirm width ≈ 22% of viewport.
5. Browser console: 0 errors / warnings.

(If no playlist entry is rendered because no playlist is active, seed one via API: create a playlist with `showInDashboard: true`, add an entry referencing a real presentation, trigger the presentation onto stage. Cleanup at end.)

- [ ] **Step 3: Confirm PR #268 still mergeable & clean**

```bash
gh pr view 268 --json mergeable,mergeStateStatus,url
```

Expected: `MERGEABLE` + `CLEAN` (Mutation Testing on the latest pipeline run must have completed).

- [ ] **Step 4: Update the PR body to mention this iteration**

Use the REST API (REST avoids the GraphQL projects-classic deprecation error):

```bash
cat > /tmp/pr-body.md <<'EOF'
## Summary

Worship-pp baseline + drag-drop infrastructure + three layout regression fixes + follow-up polish + bigger sidebar fonts.

### Stage worship-pp baseline
- Adopts worship-snv improvements (autofit text, song-name boxes, six-region layout)
- Keeps playlist sidebar; derives next_song_text from Presenter playlist instead of AbleSet

### Drag-drop infrastructure
- Added missing `GET /playlists/{id}` route (operator drop handler fetched playlist before appending and got 405)
- Fixed WASM `PlaylistEntryPayload` field renames (presentationId/entryId — server was 422-ing the snake_case payload)

### Layout regressions (round 1)
- Wrapped six worship-pp regions in `.stage-pp__slides-area` to stop sidebar overlap
- Replaced 15%-opacity tint with solid sky-blue + 4px accent bar (visible from projector)
- Server enriches playlist response with `presentation_name`; operator reads it directly; deleted obsolete `rebuild_playlist_presentations_with_signal` helper

### Follow-up polish (round 2)
- Search-result drag onto playlist row now accepted: `set_drop_effect("copy")` in playlist dragover (Chromium silently rejected drops without matching dropEffect when source set effectAllowed="copy")
- Library-kind search results no longer marked draggable (they have no `presentation_id` — drops were silent no-ops)
- Worship-pp sidebar shrunk 30% → 22% (slides 70% → 78%); entry font 0.9vw → 2.6vh

### Bigger sidebar fonts (round 3)
- Sidebar padding 1%/1.5% → 0.4%/0.5%; entry padding 0.6vh/0.8rem → 0.3vh/0.4rem; entry font-size 2.6vh → 5vh
- ~12 typical characters fit per row at 1080p projector; 12 entries still fit in the 92vh sidebar

## Test plan
- [x] CI pipeline green (all 21 jobs incl. Mutation Testing)
- [x] 3 core/serde unit tests for `presentation_name` round-trip
- [x] 2 server integration tests for response enrichment
- [x] Playwright E2E: stage-worship-pp.spec.ts (overlap, highlight, sidebar width, font ≥40px)
- [x] Playwright E2E: wasm-playlist-operations.spec.ts (drag from presentation, drag from search, name visible after drag)
- [x] Playwright E2E: wasm-drag-drop.spec.ts (search result draggable for presentation-kind, NOT for library-kind)
- [x] Live Playwright verification on dev (10.77.8.134:8080):
  - drag from search adds entry, drag from presentation list adds entry
  - sidebar at 22%, slides at 78%, no overlap
  - entry font-size ≈ 5vh; ~12 chars fit per row at 1080p
  - browser console clean
EOF
gh api -X PATCH repos/zbynekdrlik/presenter/pulls/268 \
  -f title="feat(stage): worship-pp baseline, drag-drop infra, layout regressions, follow-ups, bigger fonts" \
  -F body=@/tmp/pr-body.md \
  --jq '{number, title, state, mergeable_state}'
```

- [ ] **Step 5: Send completion report and wait for explicit "merge it"**

Per `pr-merge-policy.md` — do NOT merge. Send the completion report (per `completion-report.md`) listing the URL set for both dev and prod, audits clean, what shipped. End with `❓ Question: Merge to main now?`.

---

## Task 6: Post-merge production verification (after user merges)

- [ ] **Step 1: Watch the post-merge main pipeline**

```bash
gh run list --branch main --limit 3 --json databaseId,name,event,status,conclusion --jq '.[] | select(.event == "push")' | head -1
```

Background-poll: `sleep 1500 && gh run view <RUN_ID> ...`. Wait for ALL jobs green incl. `Deploy to Production`.

- [ ] **Step 2: Verify production**

```bash
curl -s http://10.77.9.205/healthz
```

Expected: `{"channel":"release","status":"ok","version":"0.4.39"}`.

Re-do the live Playwright check from Task 5 step 2 against `http://10.77.9.205/...`. Capture: sidebar at 22%, font ≈ 5vh, drag-from-search adds entry.

- [ ] **Step 3: Final completion report**

Short report per `completion-report.md` with prod-deploy-verified URLs. No follow-up question.

---

## Verification Summary

| Check | How |
|-------|-----|
| Font ~12 chars/row at 1080p | E2E `entryFontSizePx >= 40` (54px clears it); manual visual on dev/prod stage |
| Tighter sidebar padding | E2E `sidebarRatio` still ~22% (unchanged); inner content area larger so text reaches further |
| 12 entries still fit | Math in spec; existing `overflow-y: auto` handles >12 |
| Active highlight intact | PR #268 E2E tests for `--active` background still pass |
| No regression elsewhere | Existing E2E suite (overlap, drag-from-presentation, drag-from-search, sidebar-width) all still pass |
| Browser console clean | Every E2E asserts `consoleErrors === []` |
