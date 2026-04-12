# Companion Plugin Sync Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Sync the Companion plugin with all 14 server-supported commands, fix BibleSlide event propagation, and auto-deploy to both Companion hosts.

**Architecture:** Three layers of changes: (1) JS plugin additions for 3 missing commands, 1 layout, 1 variable, 1 feedback, (2) Rust server fix so BibleSlide events update Companion variables, (3) CI/CD pipeline job to deploy the plugin to companion-snv.lan and companion-pp.lan after every dev pipeline success.

**Tech Stack:** JavaScript (Companion module API), Rust (tokio, axum), GitHub Actions (self-hosted runner, SSH)

**Spec:** `docs/superpowers/specs/2026-04-12-companion-plugin-sync-design.md`

---

## Context

Issue #214: The Companion plugin (v0.5.0 deployed) is out of sync with the server (supports 14 commands, plugin exposes 11). Three commands are unreachable: `stage.set`, `timer.set_preach_limit`, `timer.clear_preach_limit`. The `ndi-fullscreen` layout is missing. `BibleSlide` events from the new Bible tab don't update Companion variables. There is no automated deployment.

**Key files:**
- `ops/companion/presenter/index.js` — JS plugin (COMMANDS, VARIABLE_DEFINITIONS, STAGE_LAYOUT_CHOICES, actions, feedbacks, send logic)
- `ops/companion/presenter/package.json` — version 0.6.0
- `ops/companion/presenter/companion/manifest.json` — version 0.5.0
- `crates/presenter-server/src/companion/variables.rs` — `CompanionVariableState`, `apply_live_event()`, `write_bible_variables()`
- `crates/presenter-server/src/companion/tests.rs` — unit/integration tests
- `tests/e2e/companion-session.spec.ts` — E2E tests
- `.github/workflows/pipeline.yml` — CI/CD pipeline, `deploy-dev` job at line 608

**Deployment targets:**
- companion-snv.lan (10.77.9.205): Native CompanionPi, modules at `/home/companion/.config/companion-nodejs/modules/presenter/`, restart via `sudo systemctl restart companion`, SSH key `DEPLOY_SSH_KEY`
- companion-pp.lan: Docker, modules at `/opt/companion/v4.1/modules/`, restart via `docker restart companion`, SSH key `DEPLOY_SSH_KEY_PP`

---

## File Structure

### Modified Files
| File | Change |
|------|--------|
| `ops/companion/presenter/index.js` | Add 3 commands, 1 layout choice, 1 variable, 1 feedback, 3 command option definitions, 3 send-payload cases |
| `ops/companion/presenter/package.json` | Bump version to 0.7.0 |
| `ops/companion/presenter/companion/manifest.json` | Bump version to 0.7.0 |
| `crates/presenter-server/src/companion/variables.rs` | Add `BibleSlideOverride` struct, `apply_bible_slide()` method, update `BibleSlide` handler and `write_bible_variables()` |
| `crates/presenter-server/src/companion/tests.rs` | Add BibleSlide variable test, preach limit command tests |
| `tests/e2e/companion-session.spec.ts` | Add preach limit and ndi-fullscreen E2E tests |
| `.github/workflows/pipeline.yml` | Add `deploy-companion` job after `deploy-dev` |

---

## Task 1: Add Missing Commands, Layout, Variable, and Feedback to JS Plugin

**Files:**
- Modify: `ops/companion/presenter/index.js:10-72,296-367,369-419,421-481`

- [ ] **Step 1: Add `timer_preach_limit_seconds` to VARIABLE_DEFINITIONS**

In `ops/companion/presenter/index.js`, after line 40 (`"timer_preach_elapsed_readable",`), add:

```javascript
  "timer_preach_limit_seconds",
```

- [ ] **Step 2: Add 3 missing commands to COMMANDS array**

In `ops/companion/presenter/index.js`, after line 60 (`{ id: "timer.reset_preach", label: "Timer: reset preach" },`), add:

```javascript
  { id: "timer.set_preach_limit", label: "Timer: set preach limit (seconds)" },
  { id: "timer.clear_preach_limit", label: "Timer: clear preach limit" },
```

After line 61 (`{ id: "stage.layout", label: "Stage: set layout" },`), add:

```javascript
  { id: "stage.set", label: "Stage: set presentation/slide" },
```

- [ ] **Step 3: Add `ndi-fullscreen` to STAGE_LAYOUT_CHOICES**

In `ops/companion/presenter/index.js`, after line 71 (`{ id: "preach", label: "PREACH" },`), add:

```javascript
  { id: "ndi-fullscreen", label: "NDI FULLSCREEN" },
```

- [ ] **Step 4: Add command options for new commands**

In `ops/companion/presenter/index.js`, in the `_commandOptionsFor` method (line 296), add these cases before the `default:` case (line 364):

```javascript
      case "timer.set_preach_limit":
        return [
          {
            type: "number",
            id: "seconds",
            label: "Limit (seconds)",
            default: 2700,
            min: 1,
          },
        ];
      case "stage.set":
        return [
          {
            type: "textinput",
            id: "presentationId",
            label: "Presentation ID (UUID)",
            default: "",
          },
          {
            type: "textinput",
            id: "currentSlideId",
            label: "Current Slide ID (UUID)",
            default: "",
          },
          {
            type: "textinput",
            id: "nextSlideId",
            label: "Next Slide ID (UUID, optional)",
            default: "",
          },
        ];
```

- [ ] **Step 5: Add send-payload cases for new commands**

In `ops/companion/presenter/index.js`, in the `_sendCommand` method (line 421), add these cases before the `default:` case (line 469):

```javascript
      case "timer.set_preach_limit": {
        payload = {
          seconds: Number(options.seconds) || 2700,
        };
        break;
      }
      case "stage.set": {
        payload = {
          presentationId: options.presentationId || "",
          currentSlideId: options.currentSlideId || "",
        };
        if (options.nextSlideId) {
          payload.nextSlideId = options.nextSlideId;
        }
        break;
      }
```

- [ ] **Step 6: Add `preach_running` feedback**

In `ops/companion/presenter/index.js`, in `_setupFeedbacks()`, after the `countdown_running` feedback block (after line 405), add:

```javascript
    feedbacks["preach_running"] = {
      type: "boolean",
      name: "Preach running",
      options: [],
      defaultStyle: {
        color: 0xffffff,
        bgcolor: 0x00ff00,
      },
      callback: () => this.variables.get("timer_preach_state") === "running",
    };
```

- [ ] **Step 7: Run companion JS tests**

```bash
npm run test:companion
```

Expected: All tests pass.

- [ ] **Step 8: Commit**

```bash
git add ops/companion/presenter/index.js
git commit -m "feat(companion): add missing commands, layout, variable, and feedback (#214)

Add stage.set, timer.set_preach_limit, timer.clear_preach_limit commands.
Add ndi-fullscreen layout choice. Add timer_preach_limit_seconds variable.
Add preach_running boolean feedback."
```

---

## Task 2: Bump Plugin Version

**Files:**
- Modify: `ops/companion/presenter/package.json`
- Modify: `ops/companion/presenter/companion/manifest.json`

- [ ] **Step 1: Update package.json version**

In `ops/companion/presenter/package.json`, change line 3:

```json
  "version": "0.7.0",
```

- [ ] **Step 2: Update manifest.json version**

In `ops/companion/presenter/companion/manifest.json`, change line 6:

```json
  "version": "0.7.0",
```

- [ ] **Step 3: Rebuild release tarball**

```bash
bash scripts/companion/package-module.sh
```

Expected: Creates `ops/companion/releases/presenter-companion-ws-0.7.0.tgz` and updates `ops/companion/releases/latest.json`.

- [ ] **Step 4: Commit**

```bash
git add ops/companion/presenter/package.json ops/companion/presenter/companion/manifest.json ops/companion/releases/
git commit -m "chore(companion): bump plugin version to 0.7.0 (#214)"
```

---

## Task 3: BibleSlide Event Updates Companion Variables (Rust)

**Files:**
- Modify: `crates/presenter-server/src/companion/variables.rs:11-18,21-49,121-137,139-150,377-401`
- Modify: `crates/presenter-server/src/companion/tests.rs`

- [ ] **Step 1: Write failing test for BibleSlide variable update**

In `crates/presenter-server/src/companion/tests.rs`, add after the last test (after line 361):

```rust
#[test]
fn bible_slide_event_updates_companion_variables() {
    use presenter_core::bible::BibleSlideOutput;

    let mut state = CompanionVariableState::default();
    let now = Utc::now();

    let output = BibleSlideOutput {
        main_text: "For God so loved the world".into(),
        main_reference: "John 3:16 (KJV)".into(),
        secondary_text: String::new(),
        secondary_reference: String::new(),
        triggered_at: now,
    };

    let changed = state.apply_live_event(LiveEvent::BibleSlide { output });
    assert!(changed, "BibleSlide should mark variables as changed");

    let vars: std::collections::HashMap<_, _> = state
        .to_variables()
        .into_iter()
        .map(|v| (v.name, v.value))
        .collect();

    assert_eq!(vars.get("bible_reference").unwrap(), "John 3:16 (KJV)");
    assert_eq!(vars.get("bible_text").unwrap(), "For God so loved the world");
    assert_eq!(vars.get("bible_translation_code").unwrap(), "KJV");
    assert!(!vars.get("bible_triggered_at").unwrap().is_empty());
}

#[test]
fn bible_slide_event_cleared_by_bible_cleared() {
    use presenter_core::bible::BibleSlideOutput;

    let mut state = CompanionVariableState::default();

    let output = BibleSlideOutput {
        main_text: "In the beginning".into(),
        main_reference: "Genesis 1:1 (SEB)".into(),
        secondary_text: String::new(),
        secondary_reference: String::new(),
        triggered_at: Utc::now(),
    };

    state.apply_live_event(LiveEvent::BibleSlide { output });
    assert!(state.apply_live_event(LiveEvent::BibleCleared));

    let vars: std::collections::HashMap<_, _> = state
        .to_variables()
        .into_iter()
        .map(|v| (v.name, v.value))
        .collect();

    assert_eq!(vars.get("bible_text").unwrap(), "");
    assert_eq!(vars.get("bible_reference").unwrap(), "");
    assert_eq!(vars.get("bible_translation_code").unwrap(), "");
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p presenter-server -- bible_slide_event --nocapture
```

Expected: FAIL — BibleSlide handler returns `false` and doesn't update variables.

- [ ] **Step 3: Add BibleSlideOverride struct and methods**

In `crates/presenter-server/src/companion/variables.rs`, after line 17 (`pub(super) broadcast_live: bool,`), add:

```rust
    pub(super) bible_slide: Option<BibleSlideOverride>,
```

After line 18 (end of `CompanionVariableState` struct), but before `impl CompanionVariableState`, add:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BibleSlideOverride {
    pub(super) translation_code: String,
    pub(super) reference: String,
    pub(super) text: String,
    pub(super) triggered_at: String,
}
```

- [ ] **Step 4: Update BibleSlide handler in apply_live_event**

In `crates/presenter-server/src/companion/variables.rs`, replace lines 28-33 (the `BibleSlide` arm):

```rust
            crate::live::LiveEvent::BibleSlide { output } => {
                self.apply_bible_slide(output)
            }
```

- [ ] **Step 5: Add apply_bible_slide method**

In `crates/presenter-server/src/companion/variables.rs`, after the `apply_bible` method (after line 128), add:

```rust
    pub(super) fn apply_bible_slide(
        &mut self,
        output: presenter_core::bible::BibleSlideOutput,
    ) -> bool {
        // Extract translation code from reference parentheses, e.g. "John 3:16 (KJV)" → "KJV"
        let translation_code = output
            .main_reference
            .rfind('(')
            .and_then(|start| {
                output.main_reference[start + 1..]
                    .find(')')
                    .map(|end| output.main_reference[start + 1..start + 1 + end].to_string())
            })
            .unwrap_or_default();

        let override_data = BibleSlideOverride {
            translation_code,
            reference: output.main_reference,
            text: output.main_text,
            triggered_at: output.triggered_at.to_rfc3339(),
        };

        if self.bible_slide.as_ref() == Some(&override_data) {
            return false;
        }

        self.bible = None; // Clear legacy bible data
        self.bible_slide = Some(override_data);
        true
    }
```

- [ ] **Step 6: Update apply_bible to clear bible_slide**

In `crates/presenter-server/src/companion/variables.rs`, in the `apply_bible` method (line 121-128), add `self.bible_slide = None;` before setting bible:

```rust
    pub(super) fn apply_bible(&mut self, broadcast: BibleBroadcast) -> bool {
        if self.bible.as_ref() == Some(&broadcast) && self.bible_slide.is_none() {
            false
        } else {
            self.bible_slide = None;
            self.bible = Some(broadcast);
            true
        }
    }
```

- [ ] **Step 7: Update clear_bible to also clear bible_slide**

In `crates/presenter-server/src/companion/variables.rs`, in the `clear_bible` method (line 130-137):

```rust
    pub(super) fn clear_bible(&mut self) -> bool {
        if self.bible.is_some() || self.bible_slide.is_some() {
            self.bible = None;
            self.bible_slide = None;
            true
        } else {
            false
        }
    }
```

- [ ] **Step 8: Update write_bible_variables to check bible_slide first**

In `crates/presenter-server/src/companion/variables.rs`, in `to_variables()` (line 144), change:

```rust
        write_bible_variables(&mut builder, self.bible.as_ref());
```

to:

```rust
        write_bible_variables(&mut builder, self.bible.as_ref(), self.bible_slide.as_ref());
```

Then update the `write_bible_variables` function signature and body (lines 377-401):

```rust
pub(super) fn write_bible_variables(
    builder: &mut VariableBuilder,
    broadcast: Option<&BibleBroadcast>,
    slide_override: Option<&BibleSlideOverride>,
) {
    if let Some(ov) = slide_override {
        builder.set("bible_translation_code", ov.translation_code.clone());
        builder.set("bible_translation_name", ov.translation_code.clone());
        builder.set("bible_reference", ov.reference.clone());
        builder.set("bible_text", ov.text.clone());
        builder.set("bible_triggered_at", ov.triggered_at.clone());
    } else if let Some(broadcast) = broadcast {
        let reference = broadcast.passage.reference.to_human_readable();
        builder.set(
            "bible_translation_code",
            broadcast.passage.translation.code.clone(),
        );
        builder.set(
            "bible_translation_name",
            broadcast.passage.translation.name.clone(),
        );
        builder.set("bible_reference", reference);
        builder.set("bible_text", broadcast.passage.text.clone());
        builder.set("bible_triggered_at", broadcast.triggered_at.to_rfc3339());
    } else {
        builder.set("bible_translation_code", "".into());
        builder.set("bible_translation_name", "".into());
        builder.set("bible_reference", "".into());
        builder.set("bible_text", "".into());
        builder.set("bible_triggered_at", "".into());
    }
}
```

- [ ] **Step 9: Run tests**

```bash
cargo test -p presenter-server -- companion --nocapture
```

Expected: All tests pass including the 2 new ones.

- [ ] **Step 10: Commit**

```bash
cargo fmt --all
git add crates/presenter-server/src/companion/variables.rs crates/presenter-server/src/companion/tests.rs
git commit -m "fix(companion): BibleSlide events update Companion variables (#214)

Add BibleSlideOverride to CompanionVariableState. BibleSlide events
from the Bible tab now populate bible_* variables. Translation code
is extracted from the reference parentheses. Legacy BibleBroadcast
and new BibleSlide are mutually exclusive — whichever arrives last
wins."
```

---

## Task 4: E2E Tests for New Commands

**Files:**
- Modify: `tests/e2e/companion-session.spec.ts`

- [ ] **Step 1: Add preach limit and ndi-fullscreen E2E tests**

In `tests/e2e/companion-session.spec.ts`, after the `"@companion stage layout command"` test (after line 316), add:

```typescript
  test("@companion preach limit commands", async () => {
    const { socket, errors, sendCommand, extractVarMap, handshake } =
      createCompanionSocket(wsURL);
    await handshake();

    // Set preach limit to 45 minutes (2700 seconds)
    const setResult = await sendCommand("timer.set_preach_limit", {
      seconds: 2700,
    });
    expect(setResult.error).toBeNull();
    expect(setResult.vars).toBeTruthy();
    if (setResult.vars) {
      const vars = extractVarMap(setResult.vars);
      expect(vars.get("timer_preach_limit_seconds")).toBe("2700");
    }

    // Clear preach limit
    const clearResult = await sendCommand("timer.clear_preach_limit");
    expect(clearResult.error).toBeNull();
    expect(clearResult.vars).toBeTruthy();
    if (clearResult.vars) {
      const vars = extractVarMap(clearResult.vars);
      expect(vars.get("timer_preach_limit_seconds")).toBe("");
    }

    expect(errors).toHaveLength(0);
    socket.close();
  });

  test("@companion ndi-fullscreen layout", async () => {
    const { socket, errors, sendCommand, extractVarMap, handshake } =
      createCompanionSocket(wsURL);
    await handshake();

    const result = await sendCommand("stage.layout", {
      code: "ndi-fullscreen",
    });
    expect(result.vars).toBeTruthy();
    if (result.vars) {
      const vars = extractVarMap(result.vars);
      expect(vars.get("stage_layout_code")).toBe("ndi-fullscreen");
    }

    // Switch back to default
    const result2 = await sendCommand("stage.layout", { code: "worship-snv" });
    expect(result2.vars).toBeTruthy();
    if (result2.vars) {
      const vars = extractVarMap(result2.vars);
      expect(vars.get("stage_layout_code")).toBe("worship-snv");
    }

    expect(errors).toHaveLength(0);
    socket.close();
  });
```

- [ ] **Step 2: Run E2E tests locally**

```bash
npm run test:playwright -- companion-session
```

Expected: All companion E2E tests pass (existing + 2 new).

- [ ] **Step 3: Commit**

```bash
git add tests/e2e/companion-session.spec.ts
git commit -m "test(e2e): add preach limit and ndi-fullscreen companion tests (#214)"
```

---

## Task 5: Add preach limit commands to parse_command test

**Files:**
- Modify: `crates/presenter-server/src/companion/tests.rs`

- [ ] **Step 1: Add missing commands to parse_command_accepts_all_documented_commands test**

In `crates/presenter-server/src/companion/tests.rs`, in the `parse_command_accepts_all_documented_commands` test (line 320), add these entries to the `cases` vector after the `timer.reset_preach` entry (after line 329):

```rust
        ("timer.set_preach_limit", json!({ "seconds": 2700 })),
        ("timer.clear_preach_limit", json!({})),
```

Also add after the `broadcast.set_live` entry is missing from the test — add it too:

```rust
        ("broadcast.set_live", json!({ "enabled": true })),
```

- [ ] **Step 2: Run tests**

```bash
cargo test -p presenter-server -- parse_command_accepts_all --nocapture
```

Expected: PASS — all 14 commands now tested.

- [ ] **Step 3: Commit**

```bash
git add crates/presenter-server/src/companion/tests.rs
git commit -m "test(companion): cover all 14 commands in parse_command test (#214)"
```

---

## Task 6: CI/CD Deploy Companion Plugin

**Files:**
- Modify: `.github/workflows/pipeline.yml`

- [ ] **Step 1: Add deploy-companion job**

In `.github/workflows/pipeline.yml`, after the `deploy-dev` job (after the last line, currently line 959), add:

```yaml

  # ============================================
  # Deploy Companion Plugin to both hosts
  # ============================================
  deploy-companion:
    name: Deploy Companion Plugin
    runs-on: self-hosted
    needs: deploy-dev
    concurrency:
      group: deploy-companion
      cancel-in-progress: false
    steps:
      - uses: actions/checkout@v4

      - name: Setup SSH for SNV (companion-snv.lan = production host)
        run: |
          mkdir -p ~/.ssh
          echo "${{ secrets.DEPLOY_SSH_KEY }}" > ~/.ssh/id_snv
          chmod 600 ~/.ssh/id_snv
          ssh-keyscan 10.77.9.205 >> ~/.ssh/known_hosts 2>/dev/null
          cat >> ~/.ssh/config <<EOF
          Host companion-snv
              HostName 10.77.9.205
              User newlevel
              IdentityFile ~/.ssh/id_snv
          EOF

      - name: Setup SSH for PP (companion-pp.lan)
        run: |
          echo "${{ secrets.DEPLOY_SSH_KEY_PP }}" > ~/.ssh/id_pp
          chmod 600 ~/.ssh/id_pp
          ssh-keyscan companion-pp.lan >> ~/.ssh/known_hosts 2>/dev/null
          cat >> ~/.ssh/config <<EOF
          Host companion-pp
              HostName companion-pp.lan
              User newlevel
              IdentityFile ~/.ssh/id_pp
          EOF

      - name: Deploy to companion-snv.lan
        run: |
          MODULE_DIR="/home/companion/.config/companion-nodejs/modules/presenter"
          echo "Deploying Companion plugin to companion-snv.lan..."
          ssh companion-snv "sudo mkdir -p $MODULE_DIR && sudo chown -R newlevel:newlevel $MODULE_DIR"
          scp -r ops/companion/presenter/* companion-snv:$MODULE_DIR/
          ssh companion-snv "cd $MODULE_DIR && npm install --omit=dev"
          ssh companion-snv "sudo chown -R companion:companion $MODULE_DIR"
          echo "Restarting Companion on SNV..."
          ssh companion-snv "sudo systemctl restart companion"
          sleep 5
          ssh companion-snv "systemctl is-active companion" || { echo "::error::Companion failed to restart on SNV"; exit 1; }
          echo "Companion plugin deployed to companion-snv.lan"

      - name: Deploy to companion-pp.lan
        run: |
          MODULE_DIR="/opt/companion/v4.1/modules/presenter"
          echo "Deploying Companion plugin to companion-pp.lan..."
          ssh companion-pp "mkdir -p $MODULE_DIR"
          scp -r ops/companion/presenter/* companion-pp:$MODULE_DIR/
          ssh companion-pp "cd $MODULE_DIR && npm install --omit=dev"
          echo "Restarting Companion on PP..."
          ssh companion-pp "docker restart companion"
          sleep 10
          ssh companion-pp "docker exec companion curl -sf http://127.0.0.1:8000 > /dev/null" || { echo "::error::Companion failed to restart on PP"; exit 1; }
          echo "Companion plugin deployed to companion-pp.lan"

      - name: Show deployment info
        run: |
          VERSION=$(node -e "console.log(require('./ops/companion/presenter/package.json').version)")
          echo "=== Companion Plugin Deployment Complete ==="
          echo "Version: $VERSION"
          echo "Deployed to: companion-snv.lan, companion-pp.lan"
```

- [ ] **Step 2: Run local lint checks**

```bash
cargo fmt --all --check
```

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/pipeline.yml
git commit -m "ci(companion): auto-deploy plugin to SNV and PP after dev deploy (#214)

New deploy-companion job runs after deploy-dev succeeds. Copies
plugin files via SCP, runs npm install, and restarts Companion
on both hosts (systemd on SNV, Docker on PP)."
```

---

## Task 7: Push, Monitor CI, Create PR

- [ ] **Step 1: Run local checks**

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all
cargo test -p presenter-server -- companion --nocapture
npm run test:companion
```

Fix any issues in ONE commit if needed.

- [ ] **Step 2: Push and monitor CI**

```bash
git push origin dev
gh run list --branch dev --limit 3
```

Monitor until ALL jobs complete including the new `deploy-companion` job. If any fail, `gh run view <run-id> --log-failed`, fix ALL issues in ONE commit, push again.

- [ ] **Step 3: Create PR**

```bash
gh pr create --title "fix(companion): sync plugin with server capabilities (#214)" --body "$(cat <<'EOF'
## Summary
- Add 3 missing commands: stage.set, timer.set_preach_limit, timer.clear_preach_limit
- Add ndi-fullscreen layout choice and timer_preach_limit_seconds variable
- Fix BibleSlide events not updating Companion variables
- Add preach_running boolean feedback
- Bump plugin version 0.5.0 → 0.7.0
- Auto-deploy plugin to companion-snv.lan and companion-pp.lan after every dev pipeline

Closes #214

## Test plan
- [ ] Rust unit tests: BibleSlide variable update, parse_command covers all 14 commands
- [ ] E2E: preach limit set/clear, ndi-fullscreen layout switch
- [ ] CI: deploy-companion job succeeds on both hosts
- [ ] Verify Companion on both hosts shows v0.7.0 module

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 4: Monitor CI on PR, ensure green**

- [ ] **Step 5: Provide PR URL to user**

---

## Verification Summary

| Check | How to verify |
|-------|---------------|
| stage.set command works | Existing E2E test `@companion stage.set via WebSocket` passes |
| preach limit commands work | New E2E test `@companion preach limit commands` passes |
| ndi-fullscreen layout works | New E2E test `@companion ndi-fullscreen layout` passes |
| BibleSlide updates variables | Rust unit test `bible_slide_event_updates_companion_variables` passes |
| BibleCleared resets slide override | Rust unit test `bible_slide_event_cleared_by_bible_cleared` passes |
| All 14 commands parseable | Rust test `parse_command_accepts_all_documented_commands` passes |
| Plugin version consistent | Both package.json and manifest.json say 0.7.0 |
| Auto-deploy to SNV | CI deploy-companion job succeeds, Companion restarts |
| Auto-deploy to PP | CI deploy-companion job succeeds, Companion restarts |
| No regressions | All existing companion tests still pass |
