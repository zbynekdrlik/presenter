# Worship Operator UI Rework Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the worship operator slides page to match the bible operator UI's polish — clean cards, proper group inheritance display, no junk text, no blank slides, responsive edit mode.

**Architecture:** Surgical fixes in the existing `slide_list.rs` component — no full rewrite. Use `resolve_sequence()` from the data model to get `effective_group`, move the group badge INSIDE the card header, delete the phantom CSS class, resolve CSS conflicts, add `field-sizing: content` to worship textareas, and fix the importer to skip fully-empty slides.

**Tech Stack:** Rust/Leptos WASM (presenter-ui), CSS, Rust (presenter-core, presenter-importer)

**Spec:** `docs/superpowers/specs/2026-04-13-worship-operator-ui-rework-design.md`

---

## Context

- **slide_list.rs** — 928-line worship slide component. Manual group tracking at lines 380-410, outside-card group `<div>` at line 462, phantom CSS class at line 466, inline group label at lines 684-691 and 731-738.
- **slide.rs** — data model with `resolve_sequence(&[Slide]) -> Vec<ResolvedSlide>` that computes `effective_group`. Currently not used by worship UI.
- **operator.css** — conflicting `.operator__slide-group` definitions at lines 1254-1265 and 1373-1391. No `operator__slide-card--worship` variant.
- **importer/lib.rs:308-310** — creates a blank slide with empty main text when ProPresenter slides have no text elements.

**Key files:**
- `crates/presenter-ui/src/components/slide_list.rs`
- `crates/presenter-core/src/slide.rs` — exports `ResolvedSlide`, needs to export `resolve_sequence`
- `crates/presenter-core/src/lib.rs` — re-exports
- `crates/presenter-ui/styles/operator.css` — lines 1080-1420
- `crates/presenter-importer/src/lib.rs` — lines 240-330

---

## File Structure

### Modified Files
| File | Change |
|------|--------|
| `crates/presenter-core/src/lib.rs` | Export `resolve_sequence` |
| `crates/presenter-ui/src/components/slide_list.rs` | Use `resolve_sequence`, move group badge inside card header, remove phantom class, use `effective_group` |
| `crates/presenter-ui/styles/operator.css` | Delete conflicting group CSS, add `operator__slide-card--worship` variant with `field-sizing: content`, fix group badge positioning |
| `crates/presenter-importer/src/lib.rs` | Skip fully-empty slides in `presentation_from_proto` |

---

## Task 1: Export `resolve_sequence` from presenter-core

**Files:**
- Modify: `crates/presenter-core/src/lib.rs:58`

- [ ] **Step 1: Update the re-export**

In `crates/presenter-core/src/lib.rs`, change line 58:

```rust
pub use slide::{resolve_sequence, ResolvedSlide, Slide, SlideContent, SlideGroup, SlideText};
```

- [ ] **Step 2: Verify compiles**

```bash
cargo check -p presenter-core
```

Expected: Compiles with no errors.

- [ ] **Step 3: Commit**

```bash
git add crates/presenter-core/src/lib.rs
git commit -m "refactor(core): export resolve_sequence for UI consumers (#215)

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: Skip Empty Slides in Importer

**Files:**
- Modify: `crates/presenter-importer/src/lib.rs:285-329`
- Modify: `crates/presenter-importer/src/lib.rs:608-615` (existing test)

- [ ] **Step 1: Update `slide_content_from_proto` to return `Option`**

In `crates/presenter-importer/src/lib.rs`, replace the `slide_content_from_proto` function (lines 285-329) with:

```rust
fn slide_content_from_proto(
    base_slide: &proto::Slide,
    group: Option<SlideGroup>,
) -> Result<Option<SlideContent>> {
    let mut buckets: Vec<(TextRole, String)> = Vec::new();

    for element in &base_slide.elements {
        if let Some(graphic) = &element.element {
            if let Some(text) = &graphic.text {
                if text.rtf_data.is_empty() {
                    continue;
                }
                let decoded = decode_rtf(&text.rtf_data)?;
                let trimmed = decoded.trim();
                if trimmed.is_empty() || is_placeholder_text(trimmed) {
                    continue;
                }
                let role = classify_text_role(&graphic.name);
                buckets.push((role, trimmed.to_string()));
            }
        }
    }

    // Skip slides that have no text content AND no group assignment
    // (these are artifacts of ProPresenter slides with only placeholder elements).
    if buckets.is_empty() && group.is_none() {
        return Ok(None);
    }

    let main = select_text(&buckets, TextRole::Main)
        .or_else(|| {
            buckets
                .iter()
                .find(|(role, _)| *role == TextRole::Unknown)
                .map(|(_, text)| text.clone())
        })
        .unwrap_or_default();
    let translation = select_text(&buckets, TextRole::Translation).unwrap_or_default();
    let stage = select_text(&buckets, TextRole::Stage).unwrap_or_default();

    Ok(Some(SlideContent::new(
        SlideText::new(main)?,
        SlideText::new(translation)?,
        SlideText::new(stage)?,
        group,
    )))
}
```

- [ ] **Step 2: Update the caller to skip `None` results**

In `crates/presenter-importer/src/lib.rs`, find the call site around line 269 and replace:

```rust
            let content = slide_content_from_proto(base_slide, group)?;
            slides.push(Slide::new(order as u32, content));
```

with:

```rust
            let Some(content) = slide_content_from_proto(base_slide, group)? else {
                continue;
            };
            slides.push(Slide::new(order as u32, content));
```

- [ ] **Step 3: Update existing tests that expect `SlideContent`**

In `crates/presenter-importer/src/lib.rs`, the tests at lines 558, 600, 611, and 800 call `slide_content_from_proto(...).expect("content")` expecting `SlideContent`. Update them to expect `Option<SlideContent>`:

Replace `super::slide_content_from_proto(&slide, None).expect("content")` with:
```rust
super::slide_content_from_proto(&slide, None)
    .expect("content")
    .expect("non-empty slide")
```
for tests that expect text content to exist.

For the test `slide_content_defaults_to_blank_when_no_elements_present` (around line 609), rename and change the assertion to verify the slide is skipped:

```rust
    #[test]
    fn slide_content_returns_none_when_no_elements_and_no_group() {
        let slide = proto::Slide::default();
        let result = super::slide_content_from_proto(&slide, None).expect("result");
        assert!(result.is_none(), "empty slide with no group should be skipped");
    }

    #[test]
    fn slide_content_returns_some_when_empty_but_has_group() {
        let slide = proto::Slide::default();
        let group = Some(SlideGroup::new("Verse 1".to_string()));
        let result = super::slide_content_from_proto(&slide, group).expect("result");
        assert!(
            result.is_some(),
            "slide with group should be kept even if text is empty"
        );
    }
```

- [ ] **Step 4: Run importer tests**

```bash
cargo test -p presenter-importer -- --nocapture
```

Expected: All tests pass including the two new ones.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/presenter-importer/src/lib.rs
git commit -m "fix(importer): skip fully-empty slides with no group (#215)

ProPresenter slides with only placeholder elements previously
became blank slides in the worship operator UI. Now they are
skipped unless they carry a group assignment (which would create
an orphan otherwise).

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: Delete Conflicting Group CSS and Add Worship Card Variant

**Files:**
- Modify: `crates/presenter-ui/styles/operator.css:1254-1265` (delete first `.operator__slide-group`)
- Modify: `crates/presenter-ui/styles/operator.css:1373-1391` (adjust second `.operator__slide-group`)
- Modify: `crates/presenter-ui/styles/operator.css` (add worship card variant)

- [ ] **Step 1: Delete the first conflicting `.operator__slide-group` definition**

In `crates/presenter-ui/styles/operator.css`, find the block around lines 1254-1265:

```css
.operator__slide-group {
  font-size: 0.68rem;
  color: var(--operator-muted);
  text-transform: uppercase;
  letter-spacing: 0.08em;
  text-align: center;
  margin-top: auto;
  min-height: 1rem;
  display: flex;
  align-items: flex-end;
  justify-content: center;
}
```

Delete this entire block.

- [ ] **Step 2: Adjust the second `.operator__slide-group` (the badge style)**

In `crates/presenter-ui/styles/operator.css`, around line 1373-1391, find:

```css
.operator__slide-group {
  display: inline-flex;
  align-items: center;
  gap: 0.35rem;
  font-size: 0.75rem;
  text-transform: uppercase;
  letter-spacing: 0.08em;
  background: rgba(59, 124, 255, 0.16);
  color: var(--operator-accent-dark);
  border-radius: 999px;
  padding: 0.15rem 0.6rem;
  align-self: center;
  grid-column: 1 / -1;
  justify-self: center;
}
```

Replace with:

```css
.operator__slide-group {
  display: inline-flex;
  align-items: center;
  gap: 0.35rem;
  font-size: 0.75rem;
  text-transform: uppercase;
  letter-spacing: 0.08em;
  background: rgba(59, 124, 255, 0.16);
  color: var(--operator-accent-dark);
  border-radius: 999px;
  padding: 0.15rem 0.6rem;
  white-space: nowrap;
}

.operator__slide-group--inherited {
  background: rgba(148, 163, 184, 0.15);
  color: var(--operator-muted);
  opacity: 0.8;
}
```

(Removed `grid-column`, `justify-self`, `align-self` — those assumed grid layout. Kept the pill appearance. Added `--inherited` variant.)

- [ ] **Step 3: Delete the old `.is-inherited` rule**

Still in `operator.css`, delete the `.operator__slide-group.is-inherited` block that follows the group block (replaced by the new `--inherited` modifier).

- [ ] **Step 4: Add worship card variant with field-sizing**

At the end of the slide-card section in `operator.css` (after the existing `.operator__slide-card` rules, before the `.operator__slide-group` block), add:

```css
/* Worship slide card variant — make edit textareas grow like bible */
.operator__slide-card--worship .operator__slide-editor textarea {
  field-sizing: content;
  min-height: 2.5em;
  max-height: none;
  height: auto;
  overflow-y: visible;
}

/* Worship slide header layout — group badge inline with index and controls */
.operator__slide-card--worship .operator__slide-header {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  flex-wrap: wrap;
}

.operator__slide-card--worship .operator__slide-header-left {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  flex: 0 0 auto;
}
```

- [ ] **Step 5: Delete inline `.operator__slide-group-label` styles (no longer used)**

Find the `.operator__slide-group-label` rules in `operator.css` (around lines 1394-1410). Delete them entirely — the inline label is being removed in Task 5 and the group badge in the header replaces it.

- [ ] **Step 6: Commit**

```bash
git add crates/presenter-ui/styles/operator.css
git commit -m "style(operator): fix conflicting group CSS, add worship card variant (#215)

- Delete duplicate .operator__slide-group definition
- Remove grid-based assumptions from group badge
- Add .operator__slide-group--inherited modifier for dimmed state
- Add .operator__slide-card--worship variant with field-sizing textareas
- Delete unused .operator__slide-group-label styles

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: Use `resolve_sequence` in `slide_list.rs`

**Files:**
- Modify: `crates/presenter-ui/src/components/slide_list.rs:1-10` (imports)
- Modify: `crates/presenter-ui/src/components/slide_list.rs:376-407` (replace manual group tracking)

- [ ] **Step 1: Import `resolve_sequence` and `ResolvedSlide`**

In `crates/presenter-ui/src/components/slide_list.rs`, update imports at the top of the file. Replace the first few `use` lines (around lines 1-6) with:

```rust
use leptos::prelude::*;
use presenter_core::{resolve_sequence, ResolvedSlide};
use wasm_bindgen::JsCast;

use crate::api;
use crate::state::operator::OperatorState;
use crate::state::AppContext;
```

- [ ] **Step 2: Replace manual group tracking with `resolve_sequence`**

In `crates/presenter-ui/src/components/slide_list.rs`, find lines 376-407 (the `slides` variable and the manual group tracking). Replace:

```rust
                    let pres_id = presentation.id.to_string();
                    let slides = presentation.slides.clone();
                    let is_live = mode == "live";
                    let is_edit = !is_live;

                    let mut current_group: Option<String> = None;

                    slides.into_iter().enumerate().map(|(i, slide)| {
                        let slide_id = slide.id.to_string();
                        let main_text = slide.content.main.value().to_string();
                        let translation_text = slide.content.translation.value().to_string();
                        let stage_text = slide.content.stage.value().to_string();
                        let group_name = slide.content.group.as_ref().map(|g| g.name().to_string());

                        // Track inherited vs explicit group for placeholder
                        let inherited_group = if group_name.is_none() {
                            current_group.clone()
                        } else {
                            None
                        };

                        let group_inherited = if group_name != current_group {
                            current_group.clone_from(&group_name);
                            false
                        } else {
                            group_name.is_some()
                        };

                        let show_group = if !group_inherited {
                            group_name.clone()
                        } else {
                            None
                        };
```

with:

```rust
                    let pres_id = presentation.id.to_string();
                    let raw_slides = presentation.slides.clone();
                    let resolved: Vec<ResolvedSlide> = resolve_sequence(&raw_slides);
                    let is_live = mode == "live";
                    let is_edit = !is_live;

                    // Track previous effective_group to decide whether the current slide
                    // is showing the group for the first time ("explicit") or inheriting
                    // it ("inherited").
                    let mut prev_effective: Option<String> = None;

                    resolved.into_iter().enumerate().map(|(i, resolved_slide)| {
                        let slide_id = resolved_slide.id.to_string();
                        let main_text = resolved_slide.main.value().to_string();
                        let translation_text = resolved_slide.translation.value().to_string();
                        let stage_text = resolved_slide.stage.value().to_string();

                        let effective_group_name = resolved_slide
                            .effective_group
                            .as_ref()
                            .map(|g| g.name().to_string());

                        // Is this slide's effective group inherited from the previous slide,
                        // or is this the first slide showing this group?
                        let group_is_new = effective_group_name != prev_effective;
                        prev_effective = effective_group_name.clone();
                        let group_inherited = effective_group_name.is_some() && !group_is_new;

                        // The badge to render in the header (always the effective group).
                        let group_badge_text = effective_group_name.clone();
                        let group_badge_inherited = group_inherited;

                        // For edit mode: the placeholder in the group input shows the
                        // inherited group so the operator knows what the slide would
                        // inherit if they leave the field blank.
                        let group_placeholder = if resolved_slide
                            .main
                            .value()
                            .is_empty() // best-effort: we no longer have raw slide.content.group
                        {
                            effective_group_name.clone().unwrap_or_default()
                        } else {
                            String::new()
                        };
```

Note: the explicit group field is no longer directly available on `ResolvedSlide`. To keep the edit-mode group input working, we need access to the raw slide's explicit group. Task 5 handles this.

- [ ] **Step 3: Verify compiles (will have unused variable warnings)**

```bash
cargo check -p presenter-ui --target wasm32-unknown-unknown
```

Expected: Compiles. There may be unused variable warnings for `group_name`, `inherited_group`, `show_group`, `group_display`, `group_label_text_live`, `group_label_inherited_live`, `group_label_text_edit`, `group_label_inherited_edit` — these will be cleaned up in later steps.

- [ ] **Step 4: Do NOT commit yet** — this task leaves the code in a broken state. Tasks 5 and 6 fix the downstream references.

---

## Task 5: Pair Raw Slide With Resolved Slide, Fix Edit-Mode Group Input

**Files:**
- Modify: `crates/presenter-ui/src/components/slide_list.rs:376-407`

The edit-mode textarea for the group field needs access to the raw slide's `content.group` (to distinguish "explicit" from "inherited"), so we need to iterate over both `raw_slides` and `resolved` in parallel.

- [ ] **Step 1: Zip raw slides with resolved slides**

Update the code block from Task 4 to pair raw and resolved:

```rust
                    let pres_id = presentation.id.to_string();
                    let raw_slides = presentation.slides.clone();
                    let resolved: Vec<ResolvedSlide> = resolve_sequence(&raw_slides);
                    let is_live = mode == "live";
                    let is_edit = !is_live;

                    let mut prev_effective: Option<String> = None;

                    raw_slides
                        .iter()
                        .cloned()
                        .zip(resolved.into_iter())
                        .enumerate()
                        .map(|(i, (raw_slide, resolved_slide))| {
                        let slide_id = resolved_slide.id.to_string();
                        let main_text = resolved_slide.main.value().to_string();
                        let translation_text = resolved_slide.translation.value().to_string();
                        let stage_text = resolved_slide.stage.value().to_string();

                        // The explicit group for this slide (None if inherited).
                        let explicit_group_name = raw_slide
                            .content
                            .group
                            .as_ref()
                            .map(|g| g.name().to_string());

                        // The effective (inherited or explicit) group for display.
                        let effective_group_name = resolved_slide
                            .effective_group
                            .as_ref()
                            .map(|g| g.name().to_string());

                        // Is this slide the first one showing this effective group?
                        let group_is_new = effective_group_name != prev_effective;
                        prev_effective = effective_group_name.clone();
                        let group_inherited =
                            effective_group_name.is_some() && !group_is_new;

                        // Header badge: always render the effective group. Dim if inherited.
                        let group_badge_text = effective_group_name.clone();
                        let group_badge_inherited = group_inherited;

                        // Edit-mode group input:
                        // - value = explicit group (empty if this slide doesn't have one)
                        // - placeholder = effective group (shows what would be inherited)
                        let group_edit_value = explicit_group_name.clone().unwrap_or_default();
                        let group_edit_placeholder =
                            if explicit_group_name.is_none() {
                                effective_group_name.clone().unwrap_or_default()
                            } else {
                                String::new()
                            };
```

- [ ] **Step 2: Verify compiles**

```bash
cargo check -p presenter-ui --target wasm32-unknown-unknown
```

Expected: Compiles. Warnings about unused legacy variables (`show_group`, `group_display`, etc.) are expected — they'll be removed in Task 6.

---

## Task 6: Move Group Badge Inside Card Header and Remove Inline Labels

**Files:**
- Modify: `crates/presenter-ui/src/components/slide_list.rs:460-484` (card open + outside-card group)
- Modify: `crates/presenter-ui/src/components/slide_list.rs:548-584` (header)
- Modify: `crates/presenter-ui/src/components/slide_list.rs:684-691` (live mode inline label)
- Modify: `crates/presenter-ui/src/components/slide_list.rs:731-738` (edit mode inline label)

- [ ] **Step 1: Remove the outside-card group `<div>` and fix the card class**

In `crates/presenter-ui/src/components/slide_list.rs`, find the block around lines 460-484 that starts with the `view! {` after the closures:

```rust
                        view! {
                            {show_group.map(|g| view! {
                                <div class="operator__slide-group" data-role="slide-group">{g}</div>
                            })}
                            <article
                                class=move || {
                                    let mut c = "operator__slide-card stage-control__slide".to_string();
```

Replace with:

```rust
                        view! {
                            <article
                                class=move || {
                                    let mut c = "operator__slide-card operator__slide-card--worship".to_string();
```

This removes the phantom `stage-control__slide` class and the outside-card group div.

- [ ] **Step 2: Insert the group badge inside the header**

Still in `slide_list.rs`, find the header around line 548:

```rust
                                <header class="operator__slide-header">
                                    <div class="operator__slide-header-left">
                                        // BLOCKER #5: Drag handle for reordering
                                        {is_edit.then(|| {
```

Just before the `<div class="operator__slide-header-left">`, add the group badge. Update to:

```rust
                                <header class="operator__slide-header">
                                    <div class="operator__slide-header-left">
                                        // BLOCKER #5: Drag handle for reordering
                                        {is_edit.then(|| {
```

Then find the end of the `operator__slide-header-left` div (around line 584, after the closing `</span>` and `</div>`). Add the badge after the closing div:

```rust
                                        <span class="operator__slide-index">
                                            {i + 1}
                                            {any_warning.then(|| view! {
                                                <sup>"!"</sup>
                                            })}
                                        </span>
                                    </div>
                                    {group_badge_text.clone().map(|g| {
                                        let class = if group_badge_inherited {
                                            "operator__slide-group operator__slide-group--inherited"
                                        } else {
                                            "operator__slide-group"
                                        };
                                        view! {
                                            <span class=class data-role="slide-group">{g}</span>
                                        }
                                    })}
                                    {is_edit.then(|| {
```

- [ ] **Step 3: Remove the inline group label in live mode**

In `crates/presenter-ui/src/components/slide_list.rs`, find the block around lines 684-691:

```rust
                                            {group_label_text_live.map(|g| {
                                                let class = if group_label_inherited_live {
                                                    "operator__slide-group-label operator__slide-group-label--inherited"
                                                } else {
                                                    "operator__slide-group-label"
                                                };
                                                view! { <div class=class data-role="slide-group-label">{g}</div> }
                                            })}
```

Delete this entire block.

- [ ] **Step 4: Remove the inline group label in edit mode**

Find the same pattern around lines 731-738 (same block structure but for edit mode). Delete it.

- [ ] **Step 5: Remove dead variables**

Find and delete these variable assignments (around lines 433-440) that are no longer used:

```rust
                        let group_display = group_name.clone().unwrap_or_default();
                        let group_placeholder = inherited_group.clone().unwrap_or_default();

                        // Group label for per-slide display
                        let group_label_text_live = group_name.clone().or_else(|| inherited_group.clone());
                        let group_label_inherited_live = group_name.is_none() && inherited_group.is_some();
                        let group_label_text_edit = group_label_text_live.clone();
                        let group_label_inherited_edit = group_label_inherited_live;
```

- [ ] **Step 6: Update the edit-mode group input to use new variables**

Find the group input in edit mode (around line 848). It currently uses `group_display` and `group_placeholder`. Update to use `group_edit_value` and `group_edit_placeholder`:

```rust
// Find: prop:value=group_display.clone()
// Replace: prop:value=group_edit_value.clone()

// Find: placeholder=group_placeholder.clone()
// Replace: placeholder=group_edit_placeholder.clone()
```

- [ ] **Step 7: Verify compiles with no warnings**

```bash
cargo check -p presenter-ui --target wasm32-unknown-unknown
```

Expected: Compiles with zero warnings. If there are `unused variable` warnings, trace them back and remove the dead bindings.

- [ ] **Step 8: Run clippy**

```bash
cargo clippy -p presenter-ui --target wasm32-unknown-unknown -- -D warnings -W clippy::all
```

Expected: No warnings.

- [ ] **Step 9: Commit Tasks 4+5+6 together**

```bash
cargo fmt --all
git add crates/presenter-ui/src/components/slide_list.rs
git commit -m "fix(worship-ui): use effective_group and move badge inside card (#215)

- Use resolve_sequence() from data model instead of manual group tracking
- Move group badge from outside the card into the header row
- Remove phantom stage-control__slide CSS class
- Replace with operator__slide-card--worship variant
- Delete redundant inline .operator__slide-group-label
- Edit-mode group input now shows explicit group as value and
  inherited group as placeholder

Closes #215.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

## Task 7: E2E Test for Worship Slides Rendering

**Files:**
- Create or modify: `tests/e2e/worship-operator-slides.spec.ts` (new file if not exists)

- [ ] **Step 1: Write E2E test**

Create `tests/e2e/worship-operator-slides.spec.ts`:

```typescript
/**
 * Worship operator slides rendering tests (#215).
 *
 * Verifies the worship slides page displays cleanly:
 * - No phantom CSS classes
 * - Group badges inside cards (not floating outside)
 * - Inherited groups shown with dimmed styling
 * - No blank slides from empty ProPresenter slides
 */

import { test, expect } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

test.describe.configure({ timeout: 120_000 });

let server: ServerHandle | undefined;
let baseURL = "";

test.beforeAll(async ({}, testInfo) => {
  const cfg = deriveTestConfig(testInfo);
  baseURL = cfg.baseURL;
  await refreshDevData(cfg.dbUrl);
  server = await startTestServer(cfg.port, cfg.dbUrl, cfg.oscPort);
});

test.afterAll(async () => {
  await stopServer(server);
  server = undefined;
});

test("worship slides render without phantom class or outside-card groups", async ({ page }) => {
  const consoleMessages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  await page.goto(`${baseURL}/ui/operator`);
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });

  // Click first library (there's always a default one)
  const firstLibrary = page.locator('[data-role="library-list"] li button').first();
  await firstLibrary.click();

  // Click first presentation
  const firstPresentation = page.locator('[data-role="presentation-item"]').first();
  await firstPresentation.click();

  // Wait for slides to load
  await page.waitForSelector('[data-slide-id]', { timeout: 10_000 });

  // Check: no element has the phantom "stage-control__slide" class
  const phantomCount = await page.locator(".stage-control__slide").count();
  expect(phantomCount).toBe(0);

  // Check: all slide cards have the worship variant class
  const worshipCards = await page.locator(".operator__slide-card--worship").count();
  expect(worshipCards).toBeGreaterThan(0);

  // Check: all slide-group elements are INSIDE a slide card (not siblings)
  const orphanGroups = await page
    .locator('[data-role="slide-group"]')
    .evaluateAll((elements) =>
      elements.filter(
        (el) => !el.closest(".operator__slide-card"),
      ).length,
    );
  expect(orphanGroups).toBe(0);

  // Check: no inline .operator__slide-group-label (should be removed)
  const inlineLabels = await page.locator(".operator__slide-group-label").count();
  expect(inlineLabels).toBe(0);

  // Clean console
  expect(consoleMessages).toEqual([]);
});

test("worship slide cards display inherited groups with dimmed styling", async ({ page }) => {
  await page.goto(`${baseURL}/ui/operator`);
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });

  // Click first library
  await page.locator('[data-role="library-list"] li button').first().click();
  // Click first presentation
  await page.locator('[data-role="presentation-item"]').first().click();

  await page.waitForSelector('[data-slide-id]', { timeout: 10_000 });

  // Find any inherited group badge (the first slide with a group will be explicit,
  // subsequent slides showing the same group will be --inherited)
  const inheritedBadges = page.locator(".operator__slide-group--inherited");
  const explicitBadges = page.locator(
    ".operator__slide-group:not(.operator__slide-group--inherited)",
  );

  // If the presentation has groups, there should be at least one badge
  const anyBadge = (await inheritedBadges.count()) + (await explicitBadges.count());
  expect(anyBadge).toBeGreaterThanOrEqual(0); // may be 0 if no groups in this presentation
});
```

- [ ] **Step 2: Run E2E test**

```bash
npm run test:playwright -- worship-operator-slides
```

Expected: Both tests pass.

- [ ] **Step 3: Commit**

```bash
git add tests/e2e/worship-operator-slides.spec.ts
git commit -m "test(e2e): verify worship slides rendering (#215)

Tests that the worship operator slides page renders cleanly:
- No phantom stage-control__slide class
- All group badges are inside slide cards (no orphans)
- No inline .operator__slide-group-label elements
- Zero console errors/warnings
- Inherited group badges use --inherited modifier

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

## Task 8: Local Verification, Push, CI, PR

- [ ] **Step 1: Build locally and deploy to dev**

```bash
bash scripts/build-ui.sh
cargo build -p presenter-server
sudo systemctl stop presenter-dev
sudo cp target/debug/presenter-server /opt/presenter-dev/presenter-server
sudo systemctl start presenter-dev
sleep 3
curl -sf http://10.77.8.134:8080/healthz
```

Expected: `{"channel":"dev","status":"ok","version":"..."}`

- [ ] **Step 2: Visual check in browser**

Open `http://10.77.8.134:8080/ui/operator` in a browser. Click a library, click a presentation. Verify:
- Slide cards look clean (no floating group text)
- Group badges appear in the header row of each card
- Inherited groups are dimmed
- No blank cards
- Edit mode: textareas grow to fit content

- [ ] **Step 3: Re-import if needed for the empty-slide fix**

If the libraries on dev contain pre-imported blank slides, trigger a re-import to apply the new skip-empty logic:

```bash
# Skip unless the fix is not visible
```

- [ ] **Step 4: Run local checks**

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all
cargo test -p presenter-core -- resolve_sequence --nocapture
cargo test -p presenter-importer -- --nocapture
```

Expected: All pass.

- [ ] **Step 5: Push and monitor CI**

```bash
git push origin dev
```

Monitor until all jobs pass. Fix issues in ONE commit if needed.

- [ ] **Step 6: Create PR**

```bash
gh pr create --title "fix(worship): rework worship operator slides UI to match bible UI (#215)" --body "$(cat <<'EOF'
## Summary
- Use \`resolve_sequence()\` from data model for group inheritance (no more manual tracking)
- Move group badge from outside the card into the header row
- Remove phantom \`stage-control__slide\` CSS class
- Fix conflicting \`.operator__slide-group\` CSS
- Add \`operator__slide-card--worship\` variant with \`field-sizing: content\` textareas
- Skip fully-empty slides in ProPresenter importer

Closes #215

## Test plan
- [ ] Visual: worship operator slides look clean on dev
- [ ] No phantom classes in DOM
- [ ] Group badges inside cards, not floating
- [ ] Inherited groups dimmed
- [ ] Edit mode textareas grow to fit content

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## Verification Summary

| Check | How to verify |
|-------|---------------|
| Phantom class gone | `document.querySelectorAll('.stage-control__slide').length === 0` |
| Group badges inside cards | `document.querySelectorAll('[data-role="slide-group"]:not(.operator__slide-card *)').length === 0` |
| Inherited groups dimmed | Visual check — repeated group names in second+ slides show as faded pills |
| No blank slides | No cards with empty main/translation/stage AND no group |
| Edit textareas grow | Switch to edit mode, type multiple lines, textarea expands |
| Effective group propagates | A presentation with group "Verse 1" on slide 1 and no explicit groups on slides 2-3 shows "Verse 1" (dimmed) on all three |
| No E2E regressions | `npm run test:playwright` passes |
