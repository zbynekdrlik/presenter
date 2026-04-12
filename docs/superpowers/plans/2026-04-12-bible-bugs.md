# Bible Bug Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix three bible bugs: remove bible overlay from non-bible stage layouts (#237), stack reference blocks vertically in edit mode (#232), and fix adding multiple empty slides (#230).

**Architecture:** Three independent fixes: (1) remove `<BibleOverlay>` from 4 layouts, create new `bible_layout.rs` component, register "bible" as built-in layout, (2) change CSS grid to single column, (3) investigate and fix WASM UI state update in AddEmptySlideButton.

**Tech Stack:** Rust/Leptos WASM (presenter-ui), CSS, Rust (presenter-core)

**Spec:** `docs/superpowers/specs/2026-04-12-bible-bugs-design.md`

---

## Context

- **#237:** `<BibleOverlay>` is hardcoded into worship_snv.rs:125, worship_pp.rs:145, timer_layout.rs:53, preach_layout.rs:70. When a bible slide is triggered, it overlays ALL layouts. Should only show on a dedicated "bible" layout.
- **#232:** `.operator__slide-editor-grid` in operator.css:1353 uses `grid-template-columns: 1fr 1fr`, placing references side by side. Should be `1fr` (stacked).
- **#230:** `AddEmptySlideButton` in bible_slides.rs:40-84 appears correct backend-wise (no constraints). Bug likely in UI state — the `active_slides.set(detail.slides)` replaces slides from the append response, which should work. Need to investigate.

**Key files:**
- `crates/presenter-ui/src/components/stage/` — layout components
- `crates/presenter-ui/src/pages/stage.rs:144-164` — layout dispatch match
- `crates/presenter-core/src/stage_display.rs:22-49` — built-in layouts
- `crates/presenter-ui/styles/operator.css:1351-1355` — editor grid
- `crates/presenter-ui/src/pages/bible_slides.rs:40-84` — empty slide button
- `ops/companion/presenter/index.js:67-73` — STAGE_LAYOUT_CHOICES

---

## File Structure

### Modified Files
| File | Change |
|------|--------|
| `crates/presenter-ui/src/components/stage/worship_snv.rs` | Remove `<BibleOverlay>` line |
| `crates/presenter-ui/src/components/stage/worship_pp.rs` | Remove `<BibleOverlay>` line |
| `crates/presenter-ui/src/components/stage/timer_layout.rs` | Remove `<BibleOverlay>` line |
| `crates/presenter-ui/src/components/stage/preach_layout.rs` | Remove `<BibleOverlay>` line |
| `crates/presenter-ui/src/components/stage/mod.rs` | Add `pub mod bible_layout;` |
| `crates/presenter-ui/src/pages/stage.rs` | Add bible layout to match dispatch |
| `crates/presenter-core/src/stage_display.rs` | Add "bible" to built_in() |
| `crates/presenter-ui/styles/operator.css` | Change grid to 1fr |
| `crates/presenter-ui/src/pages/bible_slides.rs` | Fix empty slide state update |
| `ops/companion/presenter/index.js` | Add bible layout choice |

### New Files
| File | Purpose |
|------|---------|
| `crates/presenter-ui/src/components/stage/bible_layout.rs` | Dedicated bible stage layout component |

---

## Task 1: Remove BibleOverlay from Non-Bible Layouts (#237)

**Files:**
- Modify: `crates/presenter-ui/src/components/stage/worship_snv.rs:125`
- Modify: `crates/presenter-ui/src/components/stage/worship_pp.rs:145`
- Modify: `crates/presenter-ui/src/components/stage/timer_layout.rs:53`
- Modify: `crates/presenter-ui/src/components/stage/preach_layout.rs:70`

- [ ] **Step 1: Remove BibleOverlay from worship_snv.rs**

In `crates/presenter-ui/src/components/stage/worship_snv.rs`, delete line 125:
```rust
            <super::bible_overlay::BibleOverlay overlay=ctx.bible_overlay />
```

- [ ] **Step 2: Remove BibleOverlay from worship_pp.rs**

In `crates/presenter-ui/src/components/stage/worship_pp.rs`, delete line 145:
```rust
            <super::bible_overlay::BibleOverlay overlay=ctx.bible_overlay />
```

- [ ] **Step 3: Remove BibleOverlay from timer_layout.rs**

In `crates/presenter-ui/src/components/stage/timer_layout.rs`, delete line 53:
```rust
            <super::bible_overlay::BibleOverlay overlay=ctx.bible_overlay />
```

- [ ] **Step 4: Remove BibleOverlay from preach_layout.rs**

In `crates/presenter-ui/src/components/stage/preach_layout.rs`, delete line 70:
```rust
            <super::bible_overlay::BibleOverlay overlay=ctx.bible_overlay />
```

- [ ] **Step 5: Verify builds**

```bash
cargo check -p presenter-ui --target wasm32-unknown-unknown
```

Expected: Compiles (bible_overlay module still exists, just no longer imported in these 4 files). There may be unused import warnings — remove any unused `bible_overlay` imports if the compiler warns.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add crates/presenter-ui/src/components/stage/worship_snv.rs crates/presenter-ui/src/components/stage/worship_pp.rs crates/presenter-ui/src/components/stage/timer_layout.rs crates/presenter-ui/src/components/stage/preach_layout.rs
git commit -m "fix(stage): remove bible overlay from non-bible layouts (#237)

Bible text no longer appears as overlay on worship, timer, preach,
or NDI layouts. Bible will only show on the dedicated bible layout."
```

---

## Task 2: Create Bible Stage Layout (#237)

**Files:**
- Create: `crates/presenter-ui/src/components/stage/bible_layout.rs`
- Modify: `crates/presenter-ui/src/components/stage/mod.rs`
- Modify: `crates/presenter-ui/src/pages/stage.rs:5-8,144-164`
- Modify: `crates/presenter-core/src/stage_display.rs:22-49`

- [ ] **Step 1: Create bible_layout.rs**

Create `crates/presenter-ui/src/components/stage/bible_layout.rs`:

```rust
use leptos::prelude::*;

use crate::components::stage::status_bar::StatusBar;
use crate::state::stage::StageContext;
use crate::ws::stage::StageWsState;

#[component]
pub fn BibleLayout(
    ws_state: ReadSignal<StageWsState>,
    latency_ms: ReadSignal<Option<f64>>,
) -> impl IntoView {
    let ctx = use_context::<StageContext>().expect("StageContext not provided");
    let bible_overlay = ctx.bible_overlay;

    let has_content = move || bible_overlay.get().is_some();

    view! {
        <div class="stage-container" data-layout="bible">
            {move || {
                if let Some(output) = bible_overlay.get() {
                    let has_secondary = !output.secondary_text.is_empty();
                    let secondary_visible = if has_secondary { "true" } else { "false" };

                    view! {
                        <div class="stage__bible-content">
                            <div class="stage__bible-text">{output.main_text.clone()}</div>
                            <div class="stage__bible-reference">{output.main_reference.clone()}</div>

                            <div class="stage__bible-secondary" data-visible=secondary_visible>
                                <div class="stage__bible-secondary-text">
                                    {output.secondary_text.clone()}
                                </div>
                                <div class="stage__bible-secondary-ref">
                                    {output.secondary_reference.clone()}
                                </div>
                            </div>
                        </div>
                    }.into_any()
                } else {
                    view! {
                        <div class="stage__bible-waiting">
                            "Waiting for Bible passage…"
                        </div>
                    }.into_any()
                }
            }}
            <StatusBar ws_state=ws_state latency_ms=latency_ms />
        </div>
    }
}
```

- [ ] **Step 2: Register module in mod.rs**

In `crates/presenter-ui/src/components/stage/mod.rs`, add after line 1:
```rust
pub mod bible_layout;
```

- [ ] **Step 3: Add "bible" to built_in layouts**

In `crates/presenter-core/src/stage_display.rs`, add after the "ndi-fullscreen" entry (after line 48, before the closing `]`):
```rust
            Self::new(
                "bible",
                "BIBLE",
                "Full-screen Bible passage display",
            ),
```

- [ ] **Step 4: Add bible layout to stage page dispatch**

In `crates/presenter-ui/src/pages/stage.rs`, add the import at line 6:
```rust
    bible_layout::BibleLayout,
```

And add a match arm in the layout dispatch (after the "preach" arm, before "ndi-fullscreen"):
```rust
                "bible" => {
                    view! { <BibleLayout ws_state=ws_state latency_ms=latency_ms /> }.into_any()
                }
```

- [ ] **Step 5: Add CSS for bible layout**

In `crates/presenter-ui/styles/stage.css`, after the bible overlay section (after line ~240), add:

```css
/* ===== Bible layout (dedicated) ===== */

.stage-container[data-layout="bible"] {
    background: #0a0a0a;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100vh;
    padding: 4%;
    box-sizing: border-box;
}

.stage__bible-content {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    flex: 1;
    width: 100%;
}

.stage__bible-waiting {
    color: #475569;
    font-size: 2vw;
    font-style: italic;
    text-align: center;
}
```

- [ ] **Step 6: Add bible to Companion plugin layout choices**

In `ops/companion/presenter/index.js`, add after the `ndi-fullscreen` entry in `STAGE_LAYOUT_CHOICES`:
```javascript
  { id: "bible", label: "BIBLE" },
```

- [ ] **Step 7: Verify builds**

```bash
cargo check -p presenter-ui --target wasm32-unknown-unknown
cargo check -p presenter-core
```

Expected: Both compile.

- [ ] **Step 8: Commit**

```bash
cargo fmt --all
git add crates/presenter-ui/src/components/stage/bible_layout.rs crates/presenter-ui/src/components/stage/mod.rs crates/presenter-ui/src/pages/stage.rs crates/presenter-core/src/stage_display.rs crates/presenter-ui/styles/stage.css ops/companion/presenter/index.js
git commit -m "feat(stage): add dedicated bible stage layout (#237)

New 'bible' built-in layout renders bible passages full-screen.
Operator switches to bible layout for Bible on stage, switches
back to worship for lyrics. Closes #237."
```

---

## Task 3: Stack Reference Blocks Vertically (#232)

**Files:**
- Modify: `crates/presenter-ui/styles/operator.css:1351-1355`

- [ ] **Step 1: Change grid to single column**

In `crates/presenter-ui/styles/operator.css`, replace line 1353:

```css
  grid-template-columns: 1fr 1fr;
```

with:

```css
  grid-template-columns: 1fr;
```

- [ ] **Step 2: Commit**

```bash
git add crates/presenter-ui/styles/operator.css
git commit -m "fix(ui): stack bible reference blocks vertically (#232)

Change editor grid from 2-column to single column so main_reference
and translation_reference are stacked under one another."
```

---

## Task 4: Fix Multiple Empty Slides (#230)

**Files:**
- Modify: `crates/presenter-ui/src/pages/bible_slides.rs:40-84`

- [ ] **Step 1: Investigate the AddEmptySlideButton behavior**

Read `crates/presenter-ui/src/pages/bible_slides.rs` lines 39-84 carefully. The current flow:

1. User clicks "+"
2. `append_presentation_slides(&id, &[input])` sends POST to server
3. Server returns the full presentation detail (with all slides)
4. `active_slides.set(detail.slides)` replaces the slide list signal

The problem: `append_presentation_slides` returns a `BiblePresentationDetail` with a `slides` field. Check if the API response actually returns ALL slides or just the appended ones. If it returns all slides, the signal update should work. If it returns only the new slide, then `set()` replaces the list with just the new slide.

Read the API endpoint at `crates/presenter-server/src/router/bible.rs` to verify what `append` returns.

- [ ] **Step 2: Check the append response**

Read `crates/presenter-server/src/router/bible.rs` around the append endpoint. Look for `append_bible_presentation_slides` or similar. Check if the response fetches the full presentation or just returns the appended slides.

If the response only returns appended slides, the fix is to either:
(a) Change the server to return the full presentation after append, or
(b) Change the UI to merge the new slides into the existing list instead of replacing

- [ ] **Step 3: Apply the fix**

Based on investigation, apply the appropriate fix. If the server returns all slides correctly, the bug may be in signal reactivity (e.g., Leptos `set()` not triggering re-render when the new list differs by only an appended item). In that case, try `update()` instead of `set()` or force a re-fetch.

- [ ] **Step 4: Verify**

Build and test:
```bash
cargo check -p presenter-ui --target wasm32-unknown-unknown
```

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/presenter-ui/src/pages/bible_slides.rs
git commit -m "fix(bible): allow adding multiple empty slides (#230)"
```

---

## Task 5: Version Bump, Local Checks, Push, CI, PR

- [ ] **Step 1: Bump version (if needed)**

Check if dev version > main version. If not, bump patch version in `Cargo.toml`.

- [ ] **Step 2: Build and test locally**

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all
cargo test -p presenter-core -- stage_display --nocapture
npm run test:companion
```

- [ ] **Step 3: Push and monitor CI**

```bash
git push origin dev
```

Monitor until all jobs pass.

- [ ] **Step 4: Create PR**

```bash
gh pr create --title "fix: bible stage layout, edit stacking, empty slides (#237, #232, #230)" --body "$(cat <<'EOF'
## Summary
- Remove bible overlay from worship/timer/preach stage layouts
- Add dedicated 'bible' stage layout for full-screen Bible display
- Stack reference blocks vertically in bible edit mode
- Fix adding multiple empty slides to bible presentations

Closes #237, #232, #230

## Test plan
- [ ] Switch to bible layout → trigger bible → shows on stage
- [ ] Switch to worship-snv → trigger bible → does NOT show
- [ ] Bible edit mode: references stacked vertically
- [ ] Add 3 empty slides to bible presentation → all appear

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## Verification Summary

| Check | How to verify |
|-------|---------------|
| Bible not on worship layout | Switch to worship-snv, trigger bible passage, stage shows lyrics only |
| Bible not on timer layout | Switch to timer, trigger bible, stage shows countdown only |
| Bible shows on bible layout | Switch to bible, trigger bible, full-screen passage appears |
| Bible layout in Companion | Companion dropdown includes "BIBLE" option |
| References stacked | Open bible edit mode, reference fields are vertical |
| Multiple empty slides | Click "+" 3 times, 3 empty slides appear in list |
