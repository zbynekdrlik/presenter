# Worship-PP Active-Highlight Reactivity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the worship-pp stage playlist sidebar's active-row highlight follow the currently-presenting song instead of getting stuck on whatever song was active when the sidebar first rendered.

**Architecture:** Replace the static `class` binding inside the `<For>` children closure with a reactive closure that reads `is_active` from `ctx.snapshot` at evaluation time. Add an E2E regression test that triggers two consecutive presentations and asserts the highlight moves between sidebar rows.

**Tech Stack:** Leptos 0.7 (WASM), Playwright/TypeScript.

**Spec:** `docs/superpowers/specs/2026-04-28-worship-pp-active-reactivity-design.md` (commit `1a5c960`).

---

## Context

Stacks on PR #268 (still open, mergeable+clean awaiting user merge). Dev currently at 0.4.39; bump to 0.4.40 first per the version-bumping rule.

`presenter-ui` is excluded from the root workspace and has its own `Cargo.lock`. Tests via `cargo test -p presenter-ui --target x86_64-unknown-linux-gnu`. Clippy via `cd crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown -- -D warnings -W clippy::all`. Local Rust builds are allowed.

`ctx.snapshot` is a `RwSignal<Option<StageDisplaySnapshot>>` (Leptos `RwSignal` is `Copy`, so it can be captured into multiple closures freely). The fix uses `ctx.snapshot.with(|opt| ...)` to read entries reactively inside the `For` children closure.

---

## File Structure

| File | Change |
|------|--------|
| `Cargo.toml` (workspace `[workspace.package]`) | `0.4.39` → `0.4.40` |
| `crates/presenter-ui/Cargo.toml` | `0.1.8` → `0.1.9` |
| `crates/presenter-ui/src/components/stage/worship_pp.rs` | Replace the `<For children=...>` closure with the reactive-class version |
| `tests/e2e/stage-worship-pp.spec.ts` | Add the new "highlight moves on consecutive trigger" test |

---

## Task 1: Bump version to 0.4.40

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

Expected: workspace `0.4.39`, presenter-ui `0.1.8`.

- [ ] **Step 2: Bump workspace**

In `/home/newlevel/devel/presenter/presenter-dev2/Cargo.toml` under `[workspace.package]`, change `version = "0.4.39"` to `version = "0.4.40"`.

- [ ] **Step 3: Bump presenter-ui**

In `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui/Cargo.toml` under `[package]`, change `version = "0.1.8"` to `version = "0.1.9"`.

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
git commit -m "chore: bump version to 0.4.40 (#worship-pp-active-reactivity)"
```

---

## Task 2: Reactive active-class in worship_pp.rs

**Files:**
- Modify: `crates/presenter-ui/src/components/stage/worship_pp.rs:178-193`

- [ ] **Step 1: Confirm the `<For>` block location**

```bash
grep -n "stage-pp__playlist-sidebar\|For\|stage-pp__playlist-entry" /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui/src/components/stage/worship_pp.rs | head -10
```

Expected: lines around 178–193 contain `<For each=playlist_entries key=|entry| entry.name.clone() children=move |entry| { let class = if entry.is_active { ... } ... } />`.

- [ ] **Step 2: Replace the For block with the reactive version**

In `crates/presenter-ui/src/components/stage/worship_pp.rs`, find the `<For ...>` block inside `<div class="stage-pp__playlist-sidebar">` and replace the WHOLE block (lines 180–192 as shipped today). The existing block looks like:

```rust
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
```

Replace with:

```rust
                <For
                    each=playlist_entries
                    key=|entry| entry.name.clone()
                    children=move |entry| {
                        // Capture the entry's name once. The active-class
                        // must be REACTIVE — read from ctx.snapshot (a
                        // RwSignal) on every update so the highlight follows
                        // the currently-triggered song.
                        // Without this, Leptos's <For> reuses the row's DOM
                        // (same key = entry.name) and the captured entry's
                        // is_active stays at its first-render value forever.
                        let entry_name = entry.name.clone();
                        let snapshot = ctx.snapshot;
                        let is_active = move || {
                            snapshot.with(|opt| {
                                opt.as_ref()
                                    .and_then(|s| s.playlist_entries.as_ref())
                                    .map(|entries| {
                                        entries
                                            .iter()
                                            .any(|e| e.name == entry_name && e.is_active)
                                    })
                                    .unwrap_or(false)
                            })
                        };
                        let class = move || {
                            if is_active() {
                                "stage-pp__playlist-entry stage-pp__playlist-entry--active"
                            } else {
                                "stage-pp__playlist-entry"
                            }
                        };
                        let display_name = clean_song_name(&entry.name);
                        view! { <div class=class>{display_name}</div> }
                    }
                />
```

Notes for the implementer:
- `ctx` is in scope inside the component function — see how the same component already uses `ctx.snapshot.get()` higher up.
- `RwSignal` in Leptos 0.7 is `Copy`, so `let snapshot = ctx.snapshot;` is a cheap copy and the resulting `snapshot` value can be moved into the inner `is_active` closure.
- `entry_name` is a `String` and is captured by the inner closure by move. Each call to `is_active()` borrows `entry_name` immutably to compare, then `class` calls `is_active()` from its own closure.

- [ ] **Step 3: Build the WASM crate**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui
cargo build --target wasm32-unknown-unknown 2>&1 | tail -5
cargo clippy --target wasm32-unknown-unknown -- -D warnings 2>&1 | tail -10
cd ../..
```

Expected: clean build, zero clippy warnings.

If clippy complains about a closure-moves-non-Copy issue on `entry_name`, the typical fix is to clone it inside the inner closure: replace `let entry_name = entry.name.clone();` with the same line, and inside `is_active`'s body change `entries.iter().any(|e| e.name == entry_name ...)` to `entries.iter().any(|e| e.name.as_str() == entry_name.as_str() ...)`. But the as-written code should compile cleanly because `entry_name` is captured by move into `is_active` (consumed once) and then the inner `entries.iter().any` captures `&entry_name` via the closure's environment. If a borrow-checker error pops up, paste it into the report and STOP — don't introduce an `unwrap`/`clone()` workaround without asking.

- [ ] **Step 4: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
cargo fmt --all
git add crates/presenter-ui/src/components/stage/worship_pp.rs
git commit -m "fix(stage): worship-pp active-highlight follows currently-presenting song (#worship-pp-active-reactivity)

The For children closure captured entry.is_active at first render.
On subsequent updates Leptos reused the DOM (same key = entry.name)
and the stale class stuck. Make the class a reactive closure that
reads is_active from ctx.snapshot at evaluation time so the
highlight moves when the operator triggers a different song."
```

---

## Task 3: E2E test — highlight follows consecutive triggers

**Files:**
- Modify: `tests/e2e/stage-worship-pp.spec.ts`

- [ ] **Step 1: Append the new test inside the existing describe block**

Append this test at the END of the existing `test.describe("Stage worship-pp layout", () => { ... })` block in `tests/e2e/stage-worship-pp.spec.ts`, just before the closing `});`:

```typescript
  test("active highlight moves to the new song when the operator triggers a different presentation", async ({
    page,
  }) => {
    const consoleErrors: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        const t = msg.text();
        if (!t.includes("favicon") && !t.includes("crbug.com/981419")) {
          consoleErrors.push(`[${msg.type()}] ${t}`);
        }
      }
    });

    // Set worship-pp layout
    const layoutResp = await page.request.post(
      new URL("/stage/layout", baseURL).toString(),
      { data: { code: "worship-pp" } },
    );
    expect(layoutResp.ok()).toBeTruthy();

    // Find TWO presentations with at least one slide each.
    const libsResp = await page.request.get(
      new URL("/libraries", baseURL).toString(),
    );
    const libs = (await libsResp.json()) as Array<{
      id: string;
      presentations?: Array<{
        id: string;
        name?: string;
        slides?: Array<{ id: string }>;
      }>;
    }>;
    const allPres = libs
      .flatMap((lib) => lib.presentations ?? [])
      .filter((p) => (p.slides?.length ?? 0) > 0 && !!p.name);
    if (allPres.length < 2) {
      test.skip(true, "fixture has fewer than 2 presentations with slides");
      return;
    }
    const p1 = allPres[0];
    const p2 = allPres[1];
    if (
      !p1.slides ||
      !p2.slides ||
      !p1.slides[0] ||
      !p2.slides[0] ||
      !p1.name ||
      !p2.name
    ) {
      test.skip(true, "presentations missing required fields");
      return;
    }
    const p1SlideId = p1.slides[0].id;
    const p2SlideId = p2.slides[0].id;

    // Create a playlist with both presentations.
    const playlistResp = await page.request.post(
      new URL("/playlists", baseURL).toString(),
      {
        data: {
          name: `Highlight Move Test ${Date.now()}`,
          showInDashboard: true,
        },
      },
    );
    const playlist = (await playlistResp.json()) as { id: string };
    await page.request.put(
      new URL(`/playlists/${playlist.id}/entries`, baseURL).toString(),
      {
        data: {
          entries: [
            { type: "presentation", presentationId: p1.id },
            { type: "presentation", presentationId: p2.id },
          ],
        },
      },
    );

    // Trigger P1.
    const trig1 = await page.request.post(
      new URL("/stage/state", baseURL).toString(),
      {
        data: {
          presentationId: p1.id,
          currentSlideId: p1SlideId,
          playlistId: playlist.id,
        },
      },
    );
    expect(trig1.status()).toBe(204);

    // Open the stage page.
    await page.setViewportSize({ width: 1920, height: 1080 });
    await page.goto(new URL("/stage", baseURL).toString());
    await page.waitForFunction(
      () => document.body.dataset.wasmReady === "true",
      { timeout: 30_000 },
    );
    await page.waitForFunction(
      () => document.body.dataset.layoutCode === "worship-pp",
      { timeout: 30_000 },
    );

    // Wait for both rows to render.
    await page.waitForFunction(
      () => document.querySelectorAll(".stage-pp__playlist-entry").length >= 2,
      { timeout: 15_000 },
    );

    // Helper: read which row index has the active class. Returns -1 if none.
    const activeIndex = async (): Promise<number> =>
      page.evaluate(() => {
        const rows = Array.from(
          document.querySelectorAll(".stage-pp__playlist-entry"),
        );
        return rows.findIndex((r) =>
          r.classList.contains("stage-pp__playlist-entry--active"),
        );
      });

    // After triggering P1, P1's row (index 0) should be active.
    await expect.poll(activeIndex, { timeout: 10_000 }).toBe(0);

    // Now trigger P2. The highlight MUST move to row index 1.
    const trig2 = await page.request.post(
      new URL("/stage/state", baseURL).toString(),
      {
        data: {
          presentationId: p2.id,
          currentSlideId: p2SlideId,
          playlistId: playlist.id,
        },
      },
    );
    expect(trig2.status()).toBe(204);

    // Regression guard: the active class must now be on row 1, not row 0.
    await expect.poll(activeIndex, { timeout: 10_000 }).toBe(1);

    // And ensure row 0 is no longer active.
    const row0Active = await page.evaluate(() => {
      const rows = Array.from(
        document.querySelectorAll(".stage-pp__playlist-entry"),
      );
      return (
        rows[0]?.classList.contains("stage-pp__playlist-entry--active") ?? false
      );
    });
    expect(row0Active).toBe(false);

    // Cleanup
    await page.request.delete(
      new URL(`/playlists/${playlist.id}`, baseURL).toString(),
    );

    expect(consoleErrors).toEqual([]);
  });
```

- [ ] **Step 2: Format**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
npx prettier --write tests/e2e/stage-worship-pp.spec.ts 2>&1 | tail -3
```

- [ ] **Step 3: Run the new test locally**

The test must pass against the new fix from Task 2. The local Trunk dist must reflect Task 2's change — if you ran `cargo build --target wasm32-unknown-unknown` in Task 2 step 3 that's enough for `cargo build --release -p presenter-server` to bundle, but the Playwright tests typically use a release build. Run:

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
# Rebuild the WASM dist to pick up Task 2's change.
scripts/build-ui.sh 2>&1 | tail -5 || (cd crates/presenter-ui && trunk build --release && cd ../..)
# Run the new test only.
npx playwright test tests/e2e/stage-worship-pp.spec.ts --grep "active highlight moves" 2>&1 | tail -25
```

Expected: PASS. Both `expect.poll(activeIndex)` calls succeed (first to 0, then to 1) and the row 0 check shows `false` after the second trigger.

If the test fails BEFORE Task 2's fix lands (e.g. you skipped Task 2), the second `expect.poll` will time out because the highlight stays on row 0 — that's the regression we're fixing.

- [ ] **Step 4: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
git add tests/e2e/stage-worship-pp.spec.ts
git commit -m "test(e2e): assert worship-pp active highlight moves on consecutive triggers (#worship-pp-active-reactivity)

The previous static test only verified the active class was applied
when ONE presentation was triggered. This new test seeds a playlist
with TWO presentations, triggers each in turn, and asserts the
.stage-pp__playlist-entry--active class moves between rows. This
is the reactivity guard the previous test missed."
```

---

## Task 4: Local checks + push + monitor pipeline CI

- [ ] **Step 1: Workspace fmt + clippy + tests**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all 2>&1 | tail -10
cargo test --workspace 2>&1 | tail -10
```

Expected: every command exits 0, clippy zero warnings, all tests pass.

- [ ] **Step 2: presenter-ui specifically**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui
cargo clippy --target wasm32-unknown-unknown -- -D warnings 2>&1 | tail -10
cargo test --target x86_64-unknown-linux-gnu --lib 2>&1 | tail -5
cd ../..
```

Expected: clean.

- [ ] **Step 3: Push**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
git push origin dev
```

- [ ] **Step 4: Find the new pipeline run**

```bash
gh run list --branch dev --limit 3 --json databaseId,name,event,headSha,status,conclusion --jq '.[] | select(.name == "Pipeline" and .event == "push")' | head -1
```

Capture the `databaseId` — call it `RUN_ID` below.

- [ ] **Step 5: Monitor pipeline (single background poll)**

Per `ci-monitoring.md` — single background command, no repeated polling:

```bash
sleep 1500 && gh run view <RUN_ID> --json status,conclusion,jobs --jq '{status, conclusion, jobs: [.jobs[] | {name, status, conclusion}]}'
```

When the poll returns: every job must have `conclusion: success`. If anything failed, fetch failed log (`gh run view <RUN_ID> --log-failed`), fix the root cause in ONE commit, push, and monitor again (one poll iteration only).

Expected: ALL 21 jobs green (Format, Clippy, Test, Build, Code Coverage, Mutation Testing, Playwright E2E 1/3+2/3+3/3, Merge E2E Reports, Deploy to Dev, Deploy Companion Plugin).

---

## Task 5: Verify on dev + update PR #268

- [ ] **Step 1: Verify dev healthz**

```bash
curl -s http://10.77.8.134:8080/healthz
```

Expected: `{"channel":"dev","status":"ok","version":"0.4.40"}`.

- [ ] **Step 2: Live verify the fix**

Use Playwright MCP. The exact verification is the SAME flow as the new E2E test:
1. `setViewportSize` 1920×1080.
2. POST `/stage/layout` body `{"code":"worship-pp"}`.
3. Pick two presentations with slides via GET `/libraries`.
4. Create a playlist via API; PUT entries with both presentations.
5. POST `/stage/state` with P1.
6. Open `/stage`, wait for `body[data-layout-code="worship-pp"]` and at least 2 `.stage-pp__playlist-entry` rows.
7. Read `findIndex(r => r.classList.contains("stage-pp__playlist-entry--active"))` → expect `0`.
8. POST `/stage/state` with P2.
9. Re-read the index → expect `1`.
10. Browser console: 0 errors / 0 warnings.
11. Cleanup: DELETE the test playlist, POST `/stage/clear`.

- [ ] **Step 3: Update PR #268 body**

PR #268 is the umbrella PR for all worship-pp work. Add a "Reactivity fix" section to the body via REST API (avoid the GraphQL projects-classic deprecation error):

```bash
cat > /tmp/pr-body.md <<'EOF'
## Summary

Worship-pp baseline + drag-drop infrastructure + three layout regression fixes + follow-up polish + bigger sidebar fonts + active-highlight reactivity fix.

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
- Search-result drag onto playlist row now accepted: `set_drop_effect("copy")` in playlist dragover
- Library-kind search results no longer marked draggable
- Worship-pp sidebar 30% → 22%, slides 70% → 78%; entry font 0.9vw → 2.6vh

### Bigger sidebar fonts (round 3)
- Sidebar padding 1%/1.5% → 0.4%/0.5%; entry padding 0.6vh/0.8rem → 0.3vh/0.4rem; entry font-size 2.6vh → 5vh
- ~12 typical characters fit per row at 1080p projector

### Active-highlight reactivity (round 4)
- Sidebar's `--active` class was stuck on the first triggered song because the For children closure captured `entry.is_active` once at insert. Made `class` a reactive closure that reads `is_active` from `ctx.snapshot` at evaluation time. Highlight now follows the operator's song changes.
- New E2E asserts the `--active` class moves between rows on consecutive triggers (the reactivity guard the previous static test missed).

## Test plan
- [x] CI pipeline green (all 21 jobs incl. Mutation Testing)
- [x] Playwright E2E: stage-worship-pp.spec.ts (overlap, highlight static, sidebar width, font ≥40px, highlight MOVES on consecutive trigger)
- [x] Playwright E2E: wasm-playlist-operations.spec.ts (drag from presentation, drag from search, name visible after drag)
- [x] Playwright E2E: wasm-drag-drop.spec.ts (search result draggable for presentation-kind, NOT for library-kind)
- [x] Live Playwright verification on dev (10.77.8.134:8080) at 1920×1080:
  - sidebar at 22%, font 54px (5vh)
  - drag from search adds entry; drag from presentation list adds entry
  - active highlight moves between rows on consecutive triggers
  - browser console clean
EOF
gh api -X PATCH repos/zbynekdrlik/presenter/pulls/268 \
  -f title="feat(stage): worship-pp baseline, drag-drop infra, layout regressions, follow-ups, bigger fonts, active-highlight reactivity" \
  -F body=@/tmp/pr-body.md \
  --jq '{number, title, state, mergeable_state}'
```

- [ ] **Step 4: Confirm PR is mergeable + clean**

```bash
gh pr view 268 --json mergeable,mergeStateStatus,url,state
```

Expected: `MERGEABLE` + `CLEAN`. If `UNSTABLE`, identify which check is pending (most likely Mutation Testing) and wait — do NOT propose any kind of bypass.

- [ ] **Step 5: Send completion report and wait for explicit "merge it"**

Per `pr-merge-policy.md` — do NOT merge. Send completion report (per `completion-report.md`) with dev + prod URLs, audits clean, what shipped. End with `❓ Question: Merge to main now?`.

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

Expected: `{"channel":"release","status":"ok","version":"0.4.40"}`.

Repeat the live Playwright verification from Task 5 step 2 against `http://10.77.9.205/...`. Capture: highlight moves between rows on consecutive triggers.

- [ ] **Step 3: Final completion report**

Short report per `completion-report.md` with prod-deploy-verified URLs. No follow-up question.

---

## Verification Summary

| Check | How |
|-------|-----|
| Active class follows song changes | New E2E asserts `findIndex` of `--active` row goes 0 → 1 on consecutive triggers |
| Old behavior is now caught | If reactivity regresses, the second `expect.poll` times out |
| No flicker, no DOM rebuild | Same `For` key, only the class binding becomes reactive |
| Existing tests still pass | The static "active row has high-contrast background" test continues to pass |
| Browser console clean | Every E2E asserts `consoleErrors === []` |
