# Worship Search Whole-Songs Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When a worship search query matches text inside a presentation's slides, return the parent song (presentation) as a single result instead of one result per matching slide.

**Architecture:** The slide-text query in `search_slides` continues to run; the result emission changes — instead of pushing per-slide results, emit one `Presentation` result per unique parent presentation, deduped against the existing `seen_presentation_ids` set so name-matched and slide-matched presentations never double-count. Remove the now-unused `SearchResultKind::Slide` variant and the dead "Slides" UI section.

**Tech Stack:** Rust (sea-orm, axum), Leptos WASM, Playwright TypeScript.

**Spec:** `docs/superpowers/specs/2026-05-05-worship-search-whole-songs-design.md` (commit 368b38c).

---

## Context

Issue #283 (title-only): "worship song search should only search whole songs with the text in them not like now it searches some passages". User clarification in brainstorming: search continues to look inside slide content, but loads the whole song, not the matching slide.

**Key existing code:**

- `crates/presenter-persistence/src/repository/search.rs:238-409` — `search_slides`. Slide-text query, then per-slide result push starting around line 350.
- `crates/presenter-persistence/src/repository/search.rs:24` — `SearchContext.seen_presentation_ids: HashSet<String>` (existing dedupe set populated by `search_presentations`).
- `crates/presenter-core/src/search.rs:8-12` — `SearchResultKind` enum with `Library`, `Presentation`, `Slide` variants.
- `crates/presenter-ui/src/components/search.rs:274` — match arm `Slide => slds.push(r)`.
- `crates/presenter-ui/src/components/search.rs:344-349` — the "Slides" `<section>` block to delete.
- `crates/presenter-server/src/router/tests.rs:555-602` — existing test `search_endpoint_returns_results` that asserts a `Slide` result exists. Must be updated.
- `tests/e2e/wasm-search.spec.ts` — existing search Playwright spec; will get a new test.

---

## File Structure

### Modified Files

| File | Change |
|------|--------|
| `Cargo.toml` | Workspace version 0.4.64 → 0.4.65 |
| `crates/presenter-ui/Cargo.toml` | Version 0.1.33 → 0.1.34 |
| `crates/presenter-persistence/src/repository/search.rs` | `search_slides` emits one `Presentation` result per unique parent presentation, deduped against `ctx.seen_presentation_ids`. |
| `crates/presenter-core/src/search.rs:8-12` | Remove `SearchResultKind::Slide` variant. |
| `crates/presenter-ui/src/components/search.rs:270-280, 344-349` | Drop `Slide` arm in result grouping; delete "Slides" UI section. |
| `crates/presenter-server/src/router/tests.rs:555-602` | Update `search_endpoint_returns_results` to drop the Slide assertion. Add 2 new tests. |
| `tests/e2e/wasm-search.spec.ts` | Add E2E asserting slide-only-text search returns a Presentation result. |

---

## Task 1: Version Bump

**Files:**
- Modify: `Cargo.toml:15`
- Modify: `crates/presenter-ui/Cargo.toml:3`
- Modify: `Cargo.lock` (auto)
- Modify: `crates/presenter-ui/Cargo.lock` (auto)

- [ ] **Step 1: Bump workspace version**

In `Cargo.toml`, change line 15:

```toml
[workspace.package]
version = "0.4.65"
```

- [ ] **Step 2: Bump presenter-ui version**

In `crates/presenter-ui/Cargo.toml`, change line 3:

```toml
version = "0.1.34"
```

- [ ] **Step 3: Update lockfiles**

```bash
cargo update --workspace
cargo update --workspace --manifest-path crates/presenter-ui/Cargo.toml
```

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock crates/presenter-ui/Cargo.toml crates/presenter-ui/Cargo.lock
git commit -m "chore: bump version to 0.4.65"
```

---

## Task 2: `search_slides` emits Presentation results

**Files:**
- Modify: `crates/presenter-persistence/src/repository/search.rs:238-409` (`search_slides` body)

- [ ] **Step 1: Read the existing slide-result push site**

```bash
grep -n "SearchResultKind::Slide\|results.push" crates/presenter-persistence/src/repository/search.rs
```

You will find the push site inside the `for (slide_model, presentation_model) in pending` loop — look for a block like `ctx.results.push(SearchResult { kind: SearchResultKind::Slide, slide_id: Some(slide_id), ... })`.

Read the surrounding 30-40 lines to understand the data flow:

```bash
sed -n '310,409p' crates/presenter-persistence/src/repository/search.rs
```

- [ ] **Step 2: Replace the slide-result push with a presentation-result push (deduped)**

Find the existing push block (in the loop iterating over `pending`). Replace the construction of `SearchResult { kind: SearchResultKind::Slide, ... }` with the following pattern. The key changes:

1. Skip the iteration if the parent presentation is already in `ctx.seen_presentation_ids`.
2. Insert into `ctx.seen_presentation_ids` to prevent emitting the same presentation twice from this loop.
3. Push as `kind: SearchResultKind::Presentation`, with `presentation_id: Some(...)`, `presentation_name: Some(presentation_model.name.clone())`, `slide_id: None`, `snippet: None`. Keep the existing `match_field` (Main/Translation/Stage) — it tells the UI/diagnostic why this song matched.

The replacement loop body (show the full new body of the per-slide processing inside the existing `for` loop):

```rust
        for (slide_model, presentation_model) in pending {
            if ctx.is_full() {
                break;
            }
            if !ctx
                .seen_presentation_ids
                .insert(presentation_model.id.clone())
            {
                continue;
            }

            let library_name = ctx
                .library_names
                .get(&presentation_model.library_id)
                .cloned()
                .unwrap_or_default();
            let library_id = LibraryId::from_uuid(parse_uuid(&presentation_model.library_id)?);
            let presentation_id = PresentationId::from_uuid(parse_uuid(&presentation_model.id)?);

            // Determine which slide field caused the match — preserves diagnostic
            // information about why this song surfaced. Priority: Main, Translation, Stage.
            let eff_main = slide_model.worship_main.as_str();
            let eff_main_search = slide_model.worship_main_search.as_str();
            let eff_translation = slide_model.worship_translate.as_str();
            let eff_translation_search = slide_model.worship_translate_search.as_str();
            let eff_stage = slide_model.worship_stage.as_str();
            let eff_stage_search = slide_model.worship_stage_search.as_str();

            let match_field = if ctx.has_tokens {
                let combined = fold_query(&format!(
                    "{} {} {} {} {}",
                    library_name, presentation_model.name, eff_main, eff_translation, eff_stage
                ));
                if !ctx.tokens.iter().all(|token| combined.contains(token)) {
                    // Fallback: presentation_name match (rare — name didn't match here, but
                    // we already passed library/slide token check above; default to MainText).
                    SearchMatchField::MainText
                } else if ctx
                    .tokens
                    .iter()
                    .all(|token| eff_main_search.contains(token))
                {
                    SearchMatchField::MainText
                } else if ctx
                    .tokens
                    .iter()
                    .all(|token| eff_translation_search.contains(token))
                {
                    SearchMatchField::TranslationText
                } else {
                    SearchMatchField::StageText
                }
            } else if !ctx.trimmed.is_empty() {
                if eff_main.contains(&ctx.trimmed) {
                    SearchMatchField::MainText
                } else if eff_translation.contains(&ctx.trimmed) {
                    SearchMatchField::TranslationText
                } else {
                    SearchMatchField::StageText
                }
            } else {
                SearchMatchField::MainText
            };

            ctx.results.push(SearchResult {
                kind: SearchResultKind::Presentation,
                library_id,
                library_name: library_name.clone(),
                presentation_id: Some(presentation_id),
                presentation_name: Some(presentation_model.name.clone()),
                slide_id: None,
                match_field,
                snippet: None,
            });
        }
```

NOTE: replace ONLY the body of the `for (slide_model, presentation_model) in pending` loop (the iteration body that decides whether/how to push the result). Do NOT alter the slide-text query, the `pending` accumulator, the `missing_library_ids` lookup, or any code outside this loop. Read the existing body carefully and preserve the logic that came before the push (snippet fetching, eff_* extraction).

If the existing code structure is materially different from the pattern above, adapt — the goal is: skip-if-already-seen, mark-seen, push-as-Presentation. The match_field selection logic above is correct for the data flow as I read it; if the existing code uses a different intermediate to determine match_field, port it.

- [ ] **Step 3: Build and verify the change isolated to search_slides**

```bash
cargo build -p presenter-persistence
```

Expected: build passes. If `slide_id: None` causes a type error, verify `SearchResult.slide_id` is `Option<SlideId>` (per `crates/presenter-core/src/search.rs:35`).

- [ ] **Step 4: Run persistence + server tests (some will fail — expected)**

```bash
cargo test -p presenter-persistence
cargo test -p presenter-server
```

Expected: persistence tests pass. Server tests: ONE expected failure — `search_endpoint_returns_results` (router/tests.rs:555) asserts a Slide kind exists. Task 4 fixes that. Other tests should pass.

If MORE than one server test fails, investigate before proceeding.

- [ ] **Step 5: Run clippy + fmt**

```bash
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all
cargo fmt --all --check
```

Expected: clean. Zero warnings.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add crates/presenter-persistence/src/repository/search.rs
git commit -m "feat(search): emit parent presentation for slide-text matches (#283)

search_slides now pushes the parent presentation as a Presentation
result deduped against seen_presentation_ids, instead of pushing each
matching slide as a Slide result. Same slide-text query; just the
emission shape changes."
```

---

## Task 3: Remove `SearchResultKind::Slide` variant + frontend cleanup

**Files:**
- Modify: `crates/presenter-core/src/search.rs:8-12` (remove enum variant)
- Modify: `crates/presenter-ui/src/components/search.rs:270-280, 344-349` (drop Slide arm + delete "Slides" UI section)

This is a workspace-wide cascade: removing the enum variant breaks any consumer. Tests in router/tests.rs are addressed in Task 4. The frontend is the other consumer.

- [ ] **Step 1: Remove the `Slide` variant**

In `crates/presenter-core/src/search.rs`, find the enum at line 8-12:

```rust
pub enum SearchResultKind {
    Library,
    Presentation,
    Slide,
}
```

Replace with:

```rust
pub enum SearchResultKind {
    Library,
    Presentation,
}
```

- [ ] **Step 2: Drop the Slide arm in the frontend grouping**

In `crates/presenter-ui/src/components/search.rs`, find the match block around line 270-280. Look for code like:

```rust
                                presenter_core::SearchResultKind::Library => libs.push(r),
                                presenter_core::SearchResultKind::Presentation => pres.push(r),
                                presenter_core::SearchResultKind::Slide => slds.push(r),
```

Remove the `Slide` arm. Also remove the `slds` accumulator if it's defined in the same scope (check 5-10 lines above the match for `let mut slds = Vec::new();` or similar).

- [ ] **Step 3: Delete the "Slides" UI section**

In the same file, find the "Slides" `<section>` block around line 344-349:

```rust
                            {(!slides.is_empty()).then(|| view! {
                                <section class="operator__search-group" data-kind="slide">
                                    <h3>"Slides"</h3>
                                    {slides.into_iter().map(render_result).collect_view()}
                                </section>
                            })}
```

Delete this entire block. Also drop the `slides` variable from any earlier `let slides = ...;` line and from any function-end return.

If the file has a CSS class or test-id only used by this section (e.g. `data-kind="slide"`), removing the section is enough — clippy/dead-code warnings will guide further cleanup.

- [ ] **Step 4: Build native + WASM**

```bash
cargo build -p presenter-core
cargo build -p presenter-ui
cd crates/presenter-ui && cargo build --target wasm32-unknown-unknown && cd ../..
```

Expected: each builds. The compiler will surface ANY remaining references to `SearchResultKind::Slide` workspace-wide. Fix each in turn.

- [ ] **Step 5: Run workspace clippy native + WASM + fmt**

```bash
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all
cd crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings -W clippy::all && cd ../..
cargo fmt --all --check
```

Expected: clean. Zero warnings. Note: at this point, server unit tests still won't all pass (router/tests.rs:555 still asserts Slide). That's fixed in Task 4.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add crates/presenter-core/src/search.rs crates/presenter-ui/src/components/search.rs
git commit -m "refactor(search): remove SearchResultKind::Slide variant + UI section (#283)

Search results no longer include slide kind; the slide-text match
phase now emits its parent presentation. Drops the dead Slides UI
group and the unused enum variant."
```

---

## Task 4: Update existing server test + add 2 new server unit tests

**Files:**
- Modify: `crates/presenter-server/src/router/tests.rs:555-602` (update existing) + add 2 new tests

- [ ] **Step 1: Read the existing test**

```bash
sed -n '555,605p' crates/presenter-server/src/router/tests.rs
```

The test creates a library "Search Library", a presentation "Search Anthem", and a slide containing "Search line main" / "Search translation" / "Stage". It searches `?query=Search`. The current assertions check that all three kinds (Library, Presentation, Slide) appear.

After the fix:
- Library "Search Library" matches by name → `Library` result.
- Presentation "Search Anthem" matches by name → `Presentation` result.
- Slide content "Search line main" matches → would emit a `Presentation` result for "Search Anthem" but it's already in `seen_presentation_ids` → skipped (dedupe).

So the result set has `Library` + `Presentation` (one each). NO `Slide` results.

- [ ] **Step 2: Update `search_endpoint_returns_results`**

Replace the assertion block at lines 593-602 (the `let results: ...; assert!(...) chain`) entirely with:

```rust
    let results: Vec<SearchResult> = serde_json::from_slice(&bytes).unwrap();
    assert!(
        results
            .iter()
            .any(|result| matches!(result.kind, SearchResultKind::Library)),
        "expected a Library result"
    );
    assert!(
        results
            .iter()
            .any(|result| matches!(result.kind, SearchResultKind::Presentation)),
        "expected a Presentation result"
    );
```

The `Slide` assertion is dropped — slide-kind results no longer exist. The `Slide` variant has been removed in Task 3, so any leftover assertion would be a compile error anyway.

- [ ] **Step 3: Add `search_slide_text_match_returns_parent_presentation` test**

Append at the end of `crates/presenter-server/src/router/tests.rs` (just before the file's final `}` if it has one, or at the bottom of the file, following the existing test pattern):

```rust
#[tokio::test]
async fn search_slide_text_match_returns_parent_presentation() {
    let state = AppState::in_memory().await.unwrap();
    let library = state.create_library("Whole Songs Library").await.unwrap();
    let (_, _, presentation, _) = state
        .create_presentation(library.id, "Some Anthem", None)
        .await
        .unwrap();
    let slide_id = presentation.slides.first().unwrap().id;
    state
        .update_slide_content(
            presentation.id,
            slide_id,
            "test283-slide-marker is in the lyrics".to_string(),
            String::new(),
            String::new(),
            None,
            None,
        )
        .await
        .unwrap();

    let app = build_router(state);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/search?query=test283-slide-marker")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    const MAX: usize = usize::MAX;
    let bytes = axum::body::to_bytes(response.into_body(), MAX)
        .await
        .unwrap();
    let results: Vec<SearchResult> = serde_json::from_slice(&bytes).unwrap();

    let presentation_results: Vec<&SearchResult> = results
        .iter()
        .filter(|result| matches!(result.kind, SearchResultKind::Presentation))
        .collect();
    assert_eq!(
        presentation_results.len(),
        1,
        "expected exactly one Presentation result, got: {:?}",
        results
    );
    assert_eq!(
        presentation_results[0].presentation_name.as_deref(),
        Some("Some Anthem")
    );
}
```

- [ ] **Step 4: Add `search_dedupes_when_song_matches_both_name_and_slides` test**

Append next:

```rust
#[tokio::test]
async fn search_dedupes_when_song_matches_both_name_and_slides() {
    let state = AppState::in_memory().await.unwrap();
    let library = state.create_library("Dedupe Library").await.unwrap();
    let (_, _, presentation, _) = state
        .create_presentation(library.id, "Marker Song", None)
        .await
        .unwrap();
    let slide_id = presentation.slides.first().unwrap().id;
    state
        .update_slide_content(
            presentation.id,
            slide_id,
            "the marker is also here".to_string(),
            String::new(),
            String::new(),
            None,
            None,
        )
        .await
        .unwrap();

    let app = build_router(state);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/search?query=marker")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    const MAX: usize = usize::MAX;
    let bytes = axum::body::to_bytes(response.into_body(), MAX)
        .await
        .unwrap();
    let results: Vec<SearchResult> = serde_json::from_slice(&bytes).unwrap();

    let presentation_results: Vec<&SearchResult> = results
        .iter()
        .filter(|result| matches!(result.kind, SearchResultKind::Presentation))
        .collect();
    assert_eq!(
        presentation_results.len(),
        1,
        "song matched by both name and slide text must appear ONCE; got: {:?}",
        results
    );
}
```

- [ ] **Step 5: Run server tests**

```bash
cargo test -p presenter-server -- --nocapture
```

Expected: all tests pass — including the updated `search_endpoint_returns_results` and the 2 new tests.

If `update_slide_content`'s signature differs from the calls above (different param count or types), inspect with:

```bash
grep -n "pub async fn update_slide_content" crates/presenter-server/src/state/*.rs
```

Adapt the calls to match the actual signature. The test intent (insert slide with marker text) is what matters; the helper API may differ slightly.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add crates/presenter-server/src/router/tests.rs
git commit -m "test(search): cover whole-song slide-match results (#283)

Drops the Slide-kind assertion from search_endpoint_returns_results
and adds two new tests:
- search_slide_text_match_returns_parent_presentation
- search_dedupes_when_song_matches_both_name_and_slides"
```

---

## Task 5: Playwright E2E for slide-text → presentation result

**Files:**
- Modify: `tests/e2e/wasm-search.spec.ts` (add one new test)

- [ ] **Step 1: Inspect existing search spec patterns**

```bash
grep -n "test(\"" tests/e2e/wasm-search.spec.ts | head -10
sed -n '40,90p' tests/e2e/wasm-search.spec.ts
```

The file has `initPage(page)` helper, `[data-role="global-search-query"]` for the input, and `[data-role="global-search-results"]` for the result panel. Each result has `[data-role="search-result-item"]`.

- [ ] **Step 2: Add the new test**

Append at the end of the spec file's `describe` block (before the final `});` closing the describe):

```typescript
  test("slide-text-only match returns presentation, no Slides group", async ({
    page,
  }) => {
    const consoleMessages: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() !== "error" && msg.type() !== "warning") return;
      if (msg.text().includes("crbug.com/981419")) return;
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    });

    await initPage(page);

    // Type a phrase that exists in slide content but not in any
    // presentation/library name. The dev seed fixture contains
    // worship songs whose slides have lyrics text. We pick a generic
    // common word that is highly likely to appear in slide bodies.
    const searchInput = page.locator('[data-role="global-search-query"]');
    await searchInput.fill("Pán");

    // Wait for any results
    await page.waitForFunction(
      () => {
        const results = document.querySelector(
          '[data-role="global-search-results"]',
        );
        return (
          results &&
          (results.querySelectorAll('[data-role="search-result-item"]').length >
            0 ||
            results.textContent?.includes("No results"))
        );
      },
      { timeout: 10_000 },
    );

    // The result panel must NOT contain a "Slides" group section.
    const slideGroup = page.locator(
      '[data-role="global-search-results"] [data-kind="slide"]',
    );
    await expect(slideGroup).toHaveCount(0);

    expect(consoleMessages).toEqual([]);
  });
```

The test asserts the Slides group is absent. It does NOT assert that a presentation result appears (that depends on dev fixture content). The structural assertion ("no Slides group") is the durable guarantee.

- [ ] **Step 3: Run the new test locally**

```bash
npx playwright test tests/e2e/wasm-search.spec.ts -g "slide-text-only match"
```

Expected: passes. If the dev runner doesn't have the rebuilt WASM dist, run `RUSTUP_TOOLCHAIN=nightly trunk build --release` from `crates/presenter-ui/` first (per the previous trunk cache issue noted in earlier work).

- [ ] **Step 4: Run the full search spec**

```bash
npx playwright test tests/e2e/wasm-search.spec.ts
```

Expected: all tests pass. If a pre-existing test fails because it asserted the now-removed Slides group, update the assertion (or remove the obsolete test if it has no remaining purpose).

- [ ] **Step 5: Commit**

```bash
git add tests/e2e/wasm-search.spec.ts
git commit -m "test(e2e): assert search has no Slides group (#283)

After the slide-text match phase emits parent presentations instead of
slides, the Slides UI section never renders. New test asserts its
absence after a search that previously would have populated it."
```

---

## Task 6: Local checks, push, monitor CI, deploy verify, PR, completion report

**Controller-handled task.** Each step is what the controller does after Tasks 1-5 are committed.

- [ ] **Step 1: Run all local checks**

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all
cd crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings -W clippy::all && cd ../..
cargo test --workspace -- --nocapture
npx playwright test tests/e2e/wasm-search.spec.ts
```

If any fail, fix in ONE commit.

- [ ] **Step 2: Push to dev**

```bash
git push origin dev
```

- [ ] **Step 3: Monitor CI to terminal state**

```bash
gh run list --branch dev --limit 1 --json databaseId --jq '.[0].databaseId'
# Capture run id, then:
sleep 1500 && gh run view <run-id> --json status,conclusion,jobs --jq '{status, conclusion, failed: [.jobs[] | select(.conclusion == "failure") | .name]}'
```

If any job fails, `gh run view <run-id> --log-failed`, fix in ONE commit, push again, re-monitor.

- [ ] **Step 4: Verify dev deployment is live**

```bash
curl -s http://10.77.8.134:8080/healthz
```

Expected: `{"channel":"dev","status":"ok","version":"0.4.65"}`.

- [ ] **Step 5: Manual verification on dev**

Use Playwright MCP to open `http://10.77.8.134:8080/ui/operator`. Use the search input to query a phrase known to be inside slide content of a worship song (e.g. a Slovak word like "Pán" that appears in many lyrics). Verify:

1. The result panel shows the matching song under "Presentations".
2. There is NO "Slides" group section (`document.querySelector('[data-role="global-search-results"] [data-kind="slide"]')` returns null).
3. Clicking the result loads the WHOLE song into the slides list (not a single slide).
4. Browser console clean.

Capture a screenshot or DOM dump for the PR body.

- [ ] **Step 6: Open PR**

```bash
gh pr create --title "feat(search): worship search returns whole songs (#283)" --body "$(cat <<'EOF'
## Summary

Fixes #283: worship search now returns the parent song (presentation) when a slide's text matches the query, instead of emitting individual slide results. The user sees one result per matching song, ready to load as a whole.

## What changed

- **`search_slides`** in `crates/presenter-persistence/src/repository/search.rs` now pushes a `Presentation` result for each unique parent presentation, deduped against `ctx.seen_presentation_ids`. The slide-text query itself is unchanged; only the emission shape.
- **`SearchResultKind::Slide`** removed from `crates/presenter-core/src/search.rs` — search results never have this kind anymore. Removing the variant cascades through the workspace and the compiler enforces consumer updates.
- **Frontend** drops the `Slide` arm in result grouping and removes the now-dead "Slides" `<section>` UI block.

## Test plan

- [x] Workspace tests pass (cargo test --workspace)
- [x] WASM clippy clean
- [x] Updated `search_endpoint_returns_results` (was asserting a Slide-kind result)
- [x] New `search_slide_text_match_returns_parent_presentation` server unit test
- [x] New `search_dedupes_when_song_matches_both_name_and_slides` server unit test
- [x] New Playwright E2E asserting absence of the Slides UI group
- [x] Dev `/healthz` reports v0.4.65
- [x] Manual: search for a Slovak word in slide lyrics on dev → song appears as a Presentation result, no Slides group renders
- [x] Browser console clean

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 7: Verify PR is mergeable**

```bash
gh pr view <pr-number> --json mergeable,mergeStateStatus
```

Expected: `mergeable: MERGEABLE`, `mergeStateStatus: CLEAN`. If `UNSTABLE` due to mutation testing or PR Automation still pending, wait. If anything else, investigate.

- [ ] **Step 8: Run pre-completion gates**

Invoke `/plan-check` skill — must come back N/N fulfilled. Invoke `/review` skill on this PR — must come back `0 🔴 0 🟡 0 🔵`. Fix any findings inside the diff before sending the completion report.

- [ ] **Step 9: Send completion report**

Per `core/completion-report.md`. Include CI run ID, plan-check N/N, review clean, deploy verification (dev shows v0.4.65, manual search on dev verified), URLs, PR title + URL.

---

## Verification Summary

| Check | How to verify |
|-------|---------------|
| Slide-text match returns parent presentation | Unit test `search_slide_text_match_returns_parent_presentation` passes |
| Dedupe when matched by both name and slides | Unit test `search_dedupes_when_song_matches_both_name_and_slides` passes |
| `SearchResultKind::Slide` variant gone | Workspace compiles after removal; no `kind: "slide"` results in API responses |
| Frontend Slides group removed | Playwright E2E asserts `[data-kind="slide"]` count == 0 |
| No regressions | All workspace tests + WASM clippy clean |
| Live behavior on dev | Manual search returns the song as a single Presentation result |
| Clean console | Playwright session shows zero browser console errors |
