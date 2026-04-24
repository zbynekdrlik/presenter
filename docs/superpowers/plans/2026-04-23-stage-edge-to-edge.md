# Stage Edge-to-Edge Layout Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove the 2% horizontal margin around worship-snv stage boxes so the current slide and next slide span the full viewport width and the top/next pills each take 50% width and meet in the middle.

**Architecture:** CSS-only change in `crates/presenter-ui/styles/stage.css`. Six selectors are adjusted to drop the 2% horizontal offset and expand pill widths from 35% to 50%. Vertical layout, status bar, and other layouts (worship-pp / bible / timer / preach / ndi-fullscreen) are untouched. One new Playwright E2E test asserts the six elements snap to screen edges. The `api` stage layout falls through to the `WorshipSnv` renderer (`crates/presenter-ui/src/pages/stage.rs:167-169`) and gets the same visual treatment automatically.

**Tech Stack:** CSS (embedded in WASM via Trunk), Playwright (TypeScript E2E), Rust workspace version bump.

**Spec:** `docs/superpowers/specs/2026-04-23-stage-edge-to-edge-design.md`

---

## Context

The stage display uses absolute positioning with percentage values. Every main box currently has `left: 2%` (or `right: 2%`) which produces a 2% dead strip on each edge. The header row has two 35%-wide pills with a 30% empty gap between them. The current slide (lyrics, the visually dominant box) therefore uses only 96% × 48% of the screen while the TV outputs black everywhere else.

**Key existing code:**

- `crates/presenter-ui/styles/stage.css` — stage CSS; six affected selectors at lines 21, 35, 47, 61, 75, 104.
- `crates/presenter-ui/src/components/stage/worship_snv.rs` — renders the boxes (no edits needed; CSS class selectors are stable).
- `crates/presenter-ui/src/pages/stage.rs:150-169` — dispatch on `layout_code`; `"api"` falls through to `WorshipSnv`.
- `tests/e2e/stage-layout.spec.ts` — existing Playwright suite with `openStageDisplay`, `triggerSlide`, and the "boxes maintain fixed positions" test that already uses `getBoundingClientRect()` on these selectors.
- `Cargo.toml:15` — workspace version. Currently `0.4.30`, matching `main` after the bible PR merge; must be bumped before any code change.

**How the build works:**

Trunk compiles `crates/presenter-ui` to WASM and copies `stage.css` into `crates/presenter-ui/dist/`. `cargo build -p presenter-server` embeds the dist directory via `include_bytes!`/similar. Pushing to `dev` runs the pipeline which builds on the GitHub-hosted runner and deploys to `http://10.77.8.134:8080`.

---

## File Structure

### Modified files

| File | Change |
|---|---|
| `Cargo.toml` | Bump workspace version `0.4.30` → `0.4.31` |
| `crates/presenter-ui/styles/stage.css` | Update six selectors (current-group, current-song, current-slide, next-group, next-song, next-slide) |
| `tests/e2e/stage-layout.spec.ts` | Append one new `test(...)` block asserting the six boxes snap to screen edges |

### Unchanged files

- `crates/presenter-ui/src/components/stage/worship_snv.rs` — CSS class names are stable.
- `crates/presenter-ui/src/pages/stage.rs` — `"api"` layout dispatch unchanged; it already reaches `WorshipSnv`.
- Other stage layout components (`worship_pp.rs`, `bible_layout.rs`, `timer_layout.rs`, `preach_layout.rs`, `ndi_fullscreen.rs`).
- `tests/e2e/stage-snapshot.spec.ts` — no committed pixel baseline to refresh (verified by search).

---

## Task 1: Bump Version

**Files:**
- Modify: `Cargo.toml:15`

- [ ] **Step 1: Update version**

In `Cargo.toml`, change line 15 from:

```toml
version = "0.4.30"
```

to:

```toml
version = "0.4.31"
```

- [ ] **Step 2: Regenerate lockfile**

Run: `cargo update -p presenter-server --offline 2>/dev/null; cargo check -p presenter-server --no-deps 2>&1 | tail -5`

Expected: `cargo check` completes and updates `Cargo.lock` with the new workspace version. If `--offline` fails because deps aren't cached, drop `--offline` and rerun.

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "chore: bump version to 0.4.31"
```

---

## Task 2: Write Failing E2E Test

**Files:**
- Modify: `tests/e2e/stage-layout.spec.ts` — append new test at end of file (after the "stage display has no console errors" test).

- [ ] **Step 1: Add the failing test**

Open `tests/e2e/stage-layout.spec.ts` and append the following test block at the end of the file (after line 398, after the existing `stage display has no console errors` test closes):

```typescript
// ─── Edge-to-edge layout (issue: maximize lyrics area) ───────────────────

test("stage worship-snv boxes snap to viewport edges", async ({ context }) => {
  const stagePage = await openStageDisplay(context);

  // Trigger a slide so boxes render with content (some layouts short-circuit
  // when empty). The selectors we assert on exist regardless of content.
  await triggerSlide(context, 0, 1);
  await stagePage.waitForTimeout(2_000);

  const geom = await stagePage.evaluate(() => {
    const vw = window.innerWidth;
    const read = (sel: string) => {
      const el = document.querySelector(sel);
      if (!el) return null;
      const r = el.getBoundingClientRect();
      return {
        left: Math.round(r.left),
        right: Math.round(vw - r.right),
        width: Math.round(r.width),
      };
    };
    return {
      vw,
      currentSlide: read(".stage__current-slide"),
      nextSlide: read(".stage__next-slide"),
      currentGroup: read(".stage__current-group"),
      currentSong: read(".stage__current-song"),
      nextGroup: read(".stage__next-group"),
      nextSong: read(".stage__next-song"),
    };
  });

  const TOL = 2; // ±2px tolerance for sub-pixel rounding

  // Full-width slides: left edge at 0, right edge at viewport width
  expect(geom.currentSlide).not.toBeNull();
  expect(geom.currentSlide!.left).toBeLessThanOrEqual(TOL);
  expect(geom.currentSlide!.right).toBeLessThanOrEqual(TOL);
  expect(Math.abs(geom.currentSlide!.width - geom.vw)).toBeLessThanOrEqual(TOL);

  expect(geom.nextSlide).not.toBeNull();
  expect(geom.nextSlide!.left).toBeLessThanOrEqual(TOL);
  expect(geom.nextSlide!.right).toBeLessThanOrEqual(TOL);
  expect(Math.abs(geom.nextSlide!.width - geom.vw)).toBeLessThanOrEqual(TOL);

  // Left pills: flush left, 50% width
  const halfVw = geom.vw / 2;
  expect(geom.currentGroup).not.toBeNull();
  expect(geom.currentGroup!.left).toBeLessThanOrEqual(TOL);
  expect(Math.abs(geom.currentGroup!.width - halfVw)).toBeLessThanOrEqual(TOL);

  expect(geom.nextGroup).not.toBeNull();
  expect(geom.nextGroup!.left).toBeLessThanOrEqual(TOL);
  expect(Math.abs(geom.nextGroup!.width - halfVw)).toBeLessThanOrEqual(TOL);

  // Right pills: flush right, 50% width
  expect(geom.currentSong).not.toBeNull();
  expect(geom.currentSong!.right).toBeLessThanOrEqual(TOL);
  expect(Math.abs(geom.currentSong!.width - halfVw)).toBeLessThanOrEqual(TOL);

  expect(geom.nextSong).not.toBeNull();
  expect(geom.nextSong!.right).toBeLessThanOrEqual(TOL);
  expect(Math.abs(geom.nextSong!.width - halfVw)).toBeLessThanOrEqual(TOL);

  await stagePage.close();
});
```

- [ ] **Step 2: Run it to confirm it FAILS**

Run: `npx playwright test tests/e2e/stage-layout.spec.ts -g "boxes snap to viewport edges"`

Expected: test FAILS. The current slide width will be ~96% of viewport (assertion expects 100%), and the pill widths will be ~35% (assertion expects 50%). This proves the test measures the right thing.

Record the actual failing numbers from the Playwright report for sanity — they should be `width ≈ viewport * 0.96` for slides and `width ≈ viewport * 0.35` for pills.

- [ ] **Step 3: Commit the failing test**

```bash
git add tests/e2e/stage-layout.spec.ts
git commit -m "test(stage): add failing edge-to-edge layout assertions"
```

---

## Task 3: Update CSS to Pass the Test

**Files:**
- Modify: `crates/presenter-ui/styles/stage.css` — selectors at lines 21, 35, 47, 61, 75, 104.

- [ ] **Step 1: Update `.stage__current-group` (line 21)**

Find the block starting at line 21 and change `left: 2%` to `left: 0` and `width: 35%` to `width: 50%`:

```css
.stage__current-group {
    position: absolute;
    left: 0;
    top: 1%;
    width: 50%;
    height: 5%;
    display: flex;
    align-items: stretch;
    justify-content: center;
    overflow: hidden;
    padding: 0;
    margin: 0;
}
```

- [ ] **Step 2: Update `.stage__current-slide` (line 35)**

Change `left: 2%` to `left: 0` and `width: 96%` to `width: 100%`:

```css
.stage__current-slide {
    position: absolute;
    left: 0;
    top: 7%;
    width: 100%;
    height: 48%;
    display: flex;
    align-items: flex-start;
    justify-content: center;
    overflow: hidden;
}
```

- [ ] **Step 3: Update `.stage__next-group` (line 47)**

Change `left: 2%` to `left: 0` and `width: 35%` to `width: 50%`:

```css
.stage__next-group {
    position: absolute;
    left: 0;
    top: 56%;
    width: 50%;
    height: 4%;
    display: flex;
    align-items: stretch;
    justify-content: center;
    overflow: hidden;
    padding: 0;
    margin: 0;
}
```

- [ ] **Step 4: Update `.stage__current-song` (line 61)**

Change `right: 2%` to `right: 0` and `width: 35%` to `width: 50%`:

```css
.stage__current-song {
    position: absolute;
    right: 0;
    top: 1%;
    width: 50%;
    height: 5%;
    display: flex;
    align-items: stretch;
    justify-content: center;
    overflow: hidden;
    padding: 0;
    margin: 0;
}
```

- [ ] **Step 5: Update `.stage__next-song` (line 75)**

Change `right: 2%` to `right: 0` and `width: 35%` to `width: 50%`:

```css
.stage__next-song {
    position: absolute;
    right: 0;
    top: 56%;
    width: 50%;
    height: 4%;
    display: flex;
    align-items: stretch;
    justify-content: center;
    overflow: hidden;
    padding: 0;
    margin: 0;
}
```

- [ ] **Step 6: Update `.stage__next-slide` (line 104)**

Change `left: 2%` to `left: 0` and `width: 96%` to `width: 100%`:

```css
.stage__next-slide {
    position: absolute;
    left: 0;
    top: 61%;
    width: 100%;
    height: 30%;
    display: flex;
    align-items: flex-start;
    justify-content: center;
    overflow: hidden;
}
```

- [ ] **Step 7: Rebuild WASM bundle**

Run: `cd crates/presenter-ui && trunk build --release 2>&1 | tail -10; cd ../..`

Expected: build succeeds; `crates/presenter-ui/dist/stage-*.css` contains the new values. No Rust errors (CSS-only change).

- [ ] **Step 8: Run the E2E test to confirm it PASSES**

Run: `npx playwright test tests/e2e/stage-layout.spec.ts -g "boxes snap to viewport edges"`

Expected: test passes. All assertions within ±2px tolerance.

- [ ] **Step 9: Run the full stage-layout suite to catch regressions**

Run: `npx playwright test tests/e2e/stage-layout.spec.ts`

Expected: all tests pass. The "boxes maintain fixed positions regardless of content" test (line 292) only checks `top` and `height`, both unchanged by this PR. The "stage display has no console errors" test (line 373) only asserts clean console.

If any existing test fails, inspect the output: if it asserts specific pixel widths (unlikely, but verify), update the assertion to the new expected value. Do NOT weaken assertions to hide the failure.

- [ ] **Step 10: Commit the CSS change**

```bash
git add crates/presenter-ui/styles/stage.css
git commit -m "fix(stage): remove horizontal margins so worship-snv boxes fill viewport

Current slide and next slide now span the full viewport width; header
and next-header pills each take 50% width and meet in the middle.
Vertical layout and status bar unchanged. Also applies to the api
stage layout (shares WorshipSnv renderer)."
```

---

## Task 4: Local Verification

**Files:** none modified in this task.

- [ ] **Step 1: Build the release binary**

Run: `cargo build --release -p presenter-server 2>&1 | tail -5`

Expected: build succeeds. The WASM `dist/` (built in Task 3 Step 7) is embedded into the binary.

- [ ] **Step 2: Start local dev server at an unused port**

Run in the background (bash tool `run_in_background: true`):

```bash
PRESENTER_PORT=18090 PRESENTER_DB_URL=sqlite:///tmp/presenter-stage-e2e.db \
  ./target/release/presenter-server
```

Expected: binary starts, logs `Server listening on 0.0.0.0:18090`.

- [ ] **Step 3: Switch stage to worship-snv and open in Playwright**

Run (foreground):

```bash
curl -s -X POST http://127.0.0.1:18090/stage/layout \
  -H 'Content-Type: application/json' \
  -d '{"code":"worship-snv"}'
```

Then use the Playwright MCP tool or `npx playwright ...` to navigate to `http://10.77.8.134:18090/stage` at 1920×1080 and take a screenshot. Compare against `stage-current-gaps.png` (captured during brainstorming).

Expected visual difference:

- Current slide area (top ~55%) — no black strip on left or right; extends from pixel 0 to pixel 1920.
- Header pills (VSETCI, song name) — meet in the middle; each occupies half the width.
- Next-row pills — same behaviour.
- Status bar at bottom — unchanged from the previous screenshot.
- Small vertical strips between rows — unchanged (kept intentionally).

- [ ] **Step 4: Stop local server**

Kill the background server process started in Step 2.

---

## Task 5: Format, Clippy, Push, and Monitor CI

**Files:** none modified; this task verifies and ships.

- [ ] **Step 1: Fmt check**

Run: `cargo fmt --all --check`

Expected: zero output, exit code 0. If it fails, run `cargo fmt --all` and amend via a new commit (not `--amend`).

- [ ] **Step 2: Clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings -W clippy::all 2>&1 | tail -20`

Expected: no warnings. (This PR does not change Rust code, so clippy should be untouched — but run it as the safety net.)

- [ ] **Step 3: Push**

```bash
git push origin dev
```

Expected: push succeeds, pipeline.yml triggers on `dev`.

- [ ] **Step 4: Monitor CI**

Run: `gh run list --branch dev --limit 1`

Get the latest run id, then poll with a single background wait (per ci-monitoring rule):

```bash
gh run view <run-id> --json status,conclusion,jobs
```

Wait until `status == "completed"` and `conclusion == "success"` on every job, including `deploy-dev`. If any job fails, `gh run view <run-id> --log-failed`, fix root cause in ONE commit, push once, monitor again.

- [ ] **Step 5: Post-deploy verification on dev**

After `deploy-dev` succeeds, open `http://10.77.8.134:8080/stage` in Playwright (1920×1080). Confirm visually the same four points from Task 4 Step 3. Also check the browser console is clean (no errors or warnings beyond `ResizeObserver loop` which is the existing allow-listed filter).

Take a screenshot and save to `stage-edge-to-edge-verified.png` for the completion report.

- [ ] **Step 6: Open PR from dev to main**

```bash
gh pr create --base main --head dev --title "Stage worship-snv: remove horizontal margins so boxes fill viewport" \
  --body "$(cat <<'EOF'
## Summary
- Current slide and next slide now span the full viewport width (was 96%).
- Header and next-header pills each take 50% width and meet in the middle (were 35% with 30% center gap).
- Vertical layout, status bar, and all other stage layouts unchanged.
- Also applies to the `api` stage layout which renders through the same `WorshipSnv` component.

## Test plan
- [x] Playwright E2E: new `stage worship-snv boxes snap to viewport edges` test asserts all six boxes snap to edges within 2px.
- [x] Existing stage-layout suite still green (no assertions on the changed `left`/`width` values).
- [x] Visual verification on dev at 1920×1080 against `stage-current-gaps.png` baseline.
- [x] Browser console clean.

Spec: `docs/superpowers/specs/2026-04-23-stage-edge-to-edge-design.md`
EOF
)"
```

Expected: PR created, URL printed. Verify mergeable state:

```bash
gh api repos/zbynekdrlik/presenter/pulls/$(gh pr view --json number --jq .number) --jq '{mergeable, mergeable_state}'
```

Expected: `mergeable: true`, `mergeable_state: "clean"` once PR-CI completes green. Do NOT merge — wait for user's explicit instruction per `pr-merge-policy`.

---

## Verification Summary

| Check | How to verify |
|---|---|
| Current slide spans full viewport | `.stage__current-slide` `left === 0`, `width === viewport.width` ± 2px (E2E test) |
| Next slide spans full viewport | `.stage__next-slide` `left === 0`, `width === viewport.width` ± 2px (E2E test) |
| Header pills each 50%, meet in middle | `.stage__current-group` flush-left, `.stage__current-song` flush-right, each width === viewport.width / 2 ± 2px (E2E test) |
| Next pills each 50%, meet in middle | Same assertions on `.stage__next-group` / `.stage__next-song` (E2E test) |
| Vertical layout unchanged | Existing "boxes maintain fixed positions" test already covers `top` + `height` stability |
| Status bar unchanged | No CSS change on `.stage__clock`, `.stage__song-number`, `.stage__live-pill`, `.stage__connection` |
| Other layouts unchanged | No CSS change on worship-pp, bible, timer, preach, ndi-fullscreen selectors |
| Console clean | Existing "stage display has no console errors" test |
| Version bumped | `Cargo.toml` reports 0.4.31; CI `version-check` job passes |
| Dev deploy verified | Playwright screenshot on `10.77.8.134:8080` matches expected visual change |
