# Worship-PP 10-Row Sidebar Fit Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bump the worship-pp sidebar entry font from `5vh` to `7.5vh` so ~10 entries fit per sidebar instead of ~14.

**Architecture:** Single-value CSS change + corresponding E2E floor tighten. Stacks on PR #268.

**Tech Stack:** CSS, Playwright/TypeScript.

**Spec:** `docs/superpowers/specs/2026-04-29-worship-pp-10-row-fit-design.md` (commit `66b3ac3`).

---

## Task 1: Bump version to 0.4.41

**Files:**
- Modify: `Cargo.toml`, `crates/presenter-ui/Cargo.toml`

- [ ] **Step 1: Confirm versions**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
git fetch origin
grep '^version' Cargo.toml | head -1
grep '^version' crates/presenter-ui/Cargo.toml | head -1
```

Expected: workspace `0.4.40`, presenter-ui `0.1.9`.

- [ ] **Step 2: Bump**

`Cargo.toml` `[workspace.package]`: `0.4.40` → `0.4.41`.
`crates/presenter-ui/Cargo.toml` `[package]`: `0.1.9` → `0.1.10`.

- [ ] **Step 3: Refresh Cargo.lock**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
cargo check -p presenter-server 2>&1 | tail -3
cd crates/presenter-ui && cargo check --target wasm32-unknown-unknown 2>&1 | tail -3 && cd ../..
```

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock crates/presenter-ui/Cargo.toml crates/presenter-ui/Cargo.lock
git commit -m "chore: bump version to 0.4.41 (#worship-pp-10-row-fit)"
```

---

## Task 2: CSS — bump font to 7.5vh

**File:** `crates/presenter-ui/styles/stage.css`

- [ ] **Step 1: Confirm location**

```bash
grep -n "stage-pp__playlist-entry\b\|font-size: 5vh" /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui/styles/stage.css | head
```

Expected: `font-size: 5vh;` inside the `.stage-pp__playlist-entry` rule.

- [ ] **Step 2: Replace**

In `crates/presenter-ui/styles/stage.css`, find the `.stage-pp__playlist-entry` rule (the one with `font-size: 5vh;`) and update the font-size and its inline comment. The OLD lines look like:

```css
    /* Sized so ~12 typical chars fit per row at 1080p:
       sidebar 22% × 1920 = 422px, less padding → ~390px content,
       at 5vh font (≈54px) char-width is ~30px, ~12 chars per row. */
    font-size: 5vh;
```

Replace with:

```css
    /* Sized so ~10 entries fit in the 92vh sidebar at 1080p:
       per-row ≈ 7.5vh × line-height 1.1 + 0.6vh padding ≈ 9.0vh.
       Trades fewer chars-per-row for visibly larger text. */
    font-size: 7.5vh;
```

- [ ] **Step 3: Verify nothing else needs updating**

```bash
grep -nE "5vh|font-size: 5" /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui/styles/stage.css
```

Expected: no matches that target stage-pp entries (other unrelated `5vh` rules in other components are fine — but inside `.stage-pp__playlist-entry` there should be NO `5vh` left).

- [ ] **Step 4: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
git add crates/presenter-ui/styles/stage.css
git commit -m "fix(stage): worship-pp sidebar font 5vh → 7.5vh (~10 entries fit) (#worship-pp-10-row-fit)

User wanted bigger song titles. At 5vh, 14 rows fit in the 92vh
sidebar; at 7.5vh, ~10 rows fit (per-row ≈ 9vh including
line-height and padding). Trades fewer chars-per-row for more
projector-readable text."
```

---

## Task 3: E2E — tighten font-size floor to 70

**File:** `tests/e2e/stage-worship-pp.spec.ts`

- [ ] **Step 1: Find the existing assertion**

```bash
grep -n "entryFontSizePx\|>= 40\|>= 70" /home/newlevel/devel/presenter/presenter-dev2/tests/e2e/stage-worship-pp.spec.ts
```

Expected: a line `expect(measurements.entryFontSizePx).toBeGreaterThanOrEqual(40);` plus the surrounding 3-line comment.

- [ ] **Step 2: Replace**

In `tests/e2e/stage-worship-pp.spec.ts`, replace the existing comment + assertion (the comment block currently mentions `5vh × 1080p = 54px` and the assertion uses 40):

```typescript
      // Floor of 40px locks in the 5vh × 1080p = 54px font from
      // #worship-pp-bigger-fonts; any accidental drop back to 2.6vh
      // (≈28px) will fail the test.
      expect(measurements.entryFontSizePx).toBeGreaterThanOrEqual(40);
```

with:

```typescript
      // Floor of 70px locks in the 7.5vh × 1080p = 81px font from
      // #worship-pp-10-row-fit; any accidental drop back to 5vh
      // (≈54px) or smaller will fail the test.
      expect(measurements.entryFontSizePx).toBeGreaterThanOrEqual(70);
```

- [ ] **Step 3: Format**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
npx prettier --write tests/e2e/stage-worship-pp.spec.ts 2>&1 | tail -3
```

- [ ] **Step 4: Commit**

```bash
git add tests/e2e/stage-worship-pp.spec.ts
git commit -m "test(e2e): tighten worship-pp font-size floor to 70px (#worship-pp-10-row-fit)

Locks in the new 7.5vh font (≈81px @ 1080p). Any accidental
regression to 5vh (≈54px) or smaller fails the test."
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

- [ ] **Step 2: presenter-ui specifically**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui
cargo clippy --target wasm32-unknown-unknown -- -D warnings 2>&1 | tail -5
cd ../..
```

- [ ] **Step 3: Push**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
git push origin dev
```

- [ ] **Step 4: Monitor pipeline**

```bash
gh run list --branch dev --limit 3 --json databaseId,name,event,headSha,status --jq '.[] | select(.name == "Pipeline" and .event == "push")' | head -1
sleep 1500 && gh run view <RUN_ID> --json status,conclusion,jobs --jq '{status, conclusion, jobs: [.jobs[] | {name, status, conclusion}]}'
```

Expected: ALL jobs green incl. Mutation Testing + Deploy to Dev.

---

## Task 5: Verify on dev + update PR #268

- [ ] **Step 1: Verify dev healthz**

```bash
curl -s http://10.77.8.134:8080/healthz
```

Expected: `{"channel":"dev","status":"ok","version":"0.4.41"}`.

- [ ] **Step 2: Live verify**

Use Playwright MCP:
1. setViewport 1920×1080.
2. Set worship-pp layout via `POST /stage/layout`.
3. Open `/stage`, wait for `body[data-layout-code="worship-pp"]`.
4. Read `getComputedStyle(.stage-pp__playlist-entry).fontSize` — expect `81px` (= 7.5vh × 1080).
5. Browser console: zero errors / warnings.

- [ ] **Step 3: Update PR #268 body**

Add a "10-row sidebar fit (round 5)" subsection to the PR body via REST API. Replace the existing PR title/body via `gh api -X PATCH repos/zbynekdrlik/presenter/pulls/268 -f title=... -F body=@/tmp/pr-body.md`. The new round bullet:

> ### 10-row sidebar fit (round 5)
> - Entry font 5vh → 7.5vh; ~10 entries now fit in the 92vh sidebar (down from ~14). Bigger projector-readable text.
> - E2E floor tightened from ≥40px to ≥70px to lock in the new 81px @ 1080p font.

- [ ] **Step 4: Wait for explicit "merge it"**

Per `pr-merge-policy.md`. Send completion report; DO NOT merge.

---

## Task 6: Post-merge production verify (gated on user merge)

- [ ] **Step 1: Watch main pipeline**

```bash
gh run list --branch main --limit 3 --json databaseId,name,event,status,conclusion --jq '.[] | select(.event == "push")' | head -1
sleep 1500 && gh run view <RUN_ID> ...
```

- [ ] **Step 2: Verify prod**

```bash
curl -s http://10.77.9.205/healthz
```

Expected: `{"channel":"release","status":"ok","version":"0.4.41"}`. Repeat the live Playwright check from Task 5 step 2 against `http://10.77.9.205/...`.

- [ ] **Step 3: Final completion report**

Short report per `completion-report.md` with prod-verified URLs.
