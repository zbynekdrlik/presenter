# Companion Plugin Action UX Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Convert the Companion plugin's `broadcast.set_live` action from a checkbox (state invisible on the button face) to a dropdown (Live ON / Live OFF labels visible), and convert `timer.set_preach_limit` from seconds (default 2700) to minutes (default 45). Wire format to the server is unchanged.

**Architecture:** Single-file change in `ops/companion/presenter/index.js`: action label, two action input definitions, two payload handlers. Plugin manifest version bumps 0.7.0 → 0.8.0. Tests add structural assertions verifying the new option ids and handler wire-format conversion.

**Tech Stack:** Node 18 (Companion plugin runtime), node:test (tests), Bitfocus Companion 1.13.2 API.

**Spec:** `docs/superpowers/specs/2026-05-06-companion-action-ux-design.md` (commit 655be48).

---

## Context

Issues #270 + #249 (both title-only):

- #270: Companion checkboxes don't render their bound state on the button face. Operator can't tell which state ("Live ON" or "Live OFF") a button is configured for. Switch to dropdown so the chosen option label appears on the button.
- #249: Preach timer limit input is in seconds (default 2700). Operators think in minutes. Switch to minutes (default 45) on the UI; multiply by 60 on the wire so the server still receives seconds.

**Key existing code:**

- `ops/companion/presenter/index.js:62` — action label `"Timer: set preach limit (seconds)"`.
- `ops/companion/presenter/index.js:361-365` — `timer.set_preach_limit` input definition (number `seconds`, default 2700).
- `ops/companion/presenter/index.js:392-400` — `broadcast.set_live` input definition (checkbox `enabled`, default false).
- `ops/companion/presenter/index.js:511-515` — `timer.set_preach_limit` handler (`payload = { seconds: Number(options.seconds) || 2700 }`).
- `ops/companion/presenter/index.js:527-532` — `broadcast.set_live` handler (`payload = { enabled: Boolean(options.enabled) }`).
- `ops/companion/presenter/companion/manifest.json:6` — plugin version `0.7.0`.
- `ops/companion/presenter/lib/commands.test.js` — existing tests parse the COMMANDS array via regex from `index.js`. Only verify registered command IDs; do NOT test payload shapes today.

**Server-side handlers:** unchanged. Server receives `{seconds: number}` for preach limit and `{enabled: boolean}` for broadcast live state.

**Deploy note:** the plugin ships via the GitHub Release workflow (`release.yml`), which deploys the tarball to `companion-pp.lan`. Merging to main alone does NOT deploy the plugin — a release tag must be cut separately. Flag this in the completion report.

---

## File Structure

### Modified Files

| File | Change |
|------|--------|
| `Cargo.toml` | Workspace version 0.4.66 → 0.4.67 |
| `crates/presenter-ui/Cargo.toml` | Version 0.1.35 → 0.1.36 |
| `ops/companion/presenter/companion/manifest.json` | `version` 0.7.0 → 0.8.0 |
| `ops/companion/presenter/index.js` | Label change at line 62; replace input defs at 361-365 and 392-400; replace handler bodies at 511-515 and 527-532 |
| `ops/companion/presenter/lib/commands.test.js` | Add structural assertions for new option ids and handler wire format |

---

## Task 1: Version Bumps (workspace + plugin)

**Files:**
- Modify: `Cargo.toml:15`
- Modify: `crates/presenter-ui/Cargo.toml:3`
- Modify: `ops/companion/presenter/companion/manifest.json:6`
- Modify: `Cargo.lock` (auto)
- Modify: `crates/presenter-ui/Cargo.lock` (auto)

- [ ] **Step 1: Bump workspace version**

In `Cargo.toml`, change line 15:

```toml
[workspace.package]
version = "0.4.67"
```

- [ ] **Step 2: Bump presenter-ui version**

In `crates/presenter-ui/Cargo.toml`, change line 3:

```toml
version = "0.1.36"
```

- [ ] **Step 3: Bump Companion plugin manifest version**

In `ops/companion/presenter/companion/manifest.json`, change line 6:

```json
  "version": "0.8.0",
```

- [ ] **Step 4: Update lockfiles**

```bash
cargo update --workspace
cargo update --workspace --manifest-path crates/presenter-ui/Cargo.toml
```

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock crates/presenter-ui/Cargo.toml crates/presenter-ui/Cargo.lock ops/companion/presenter/companion/manifest.json
git commit -m "chore: bump version to 0.4.67 + companion plugin 0.8.0"
```

---

## Task 2: Apply the two action changes in `index.js`

**Files:**
- Modify: `ops/companion/presenter/index.js:62` (action label)
- Modify: `ops/companion/presenter/index.js:361-365` (preach limit input)
- Modify: `ops/companion/presenter/index.js:392-400` (broadcast.set_live input)
- Modify: `ops/companion/presenter/index.js:511-515` (preach limit handler)
- Modify: `ops/companion/presenter/index.js:527-532` (broadcast.set_live handler)

- [ ] **Step 1: Read the current state of the file**

```bash
sed -n '60,65p' ops/companion/presenter/index.js
sed -n '358,402p' ops/companion/presenter/index.js
sed -n '510,535p' ops/companion/presenter/index.js
```

Confirm the line ranges match the spec. If the file has shifted (e.g. due to recent edits), adjust the line references.

- [ ] **Step 2: Update the action label at line 62**

Find this line in the COMMANDS array:

```javascript
  { id: "timer.set_preach_limit", label: "Timer: set preach limit (seconds)" },
```

Replace with:

```javascript
  { id: "timer.set_preach_limit", label: "Timer: set preach limit (minutes)" },
```

- [ ] **Step 3: Replace the preach-limit input definition (around lines 361-365)**

Find the case in the action options switch:

```javascript
      case "timer.set_preach_limit":
        return [
          {
            type: "number",
            id: "seconds",
            label: "Limit (seconds)",
            ...
          },
        ];
```

(There may be `default`, `min`, `max`, `width` properties too — ignore the exact shape; replace the entire `return [...]` for this case.)

Replace with:

```javascript
      case "timer.set_preach_limit":
        return [
          {
            type: "number",
            id: "minutes",
            label: "Limit (minutes)",
            default: 45,
            min: 1,
            max: 240,
          },
        ];
```

- [ ] **Step 4: Replace the broadcast.set_live input definition (around lines 392-400)**

Find:

```javascript
      case "broadcast.set_live":
        return [
          {
            type: "checkbox",
            id: "enabled",
            label: "Live",
            default: false,
          },
        ];
```

Replace with:

```javascript
      case "broadcast.set_live":
        return [
          {
            type: "dropdown",
            id: "state",
            label: "Live state",
            choices: [
              { id: "on", label: "Live ON" },
              { id: "off", label: "Live OFF" },
            ],
            default: "on",
          },
        ];
```

- [ ] **Step 5: Replace the preach-limit handler (around lines 511-515)**

Find:

```javascript
      case "timer.set_preach_limit": {
        payload = {
          seconds: Number(options.seconds) || 2700,
        };
        break;
      }
```

Replace with:

```javascript
      case "timer.set_preach_limit": {
        payload = {
          seconds: (Number(options.minutes) || 45) * 60,
        };
        break;
      }
```

- [ ] **Step 6: Replace the broadcast.set_live handler (around lines 527-532)**

Find:

```javascript
      case "broadcast.set_live": {
        payload = {
          enabled: Boolean(options.enabled),
        };
        break;
      }
```

Replace with:

```javascript
      case "broadcast.set_live": {
        payload = {
          enabled: options.state === "on",
        };
        break;
      }
```

- [ ] **Step 7: Run plugin tests to confirm nothing broke**

```bash
cd ops/companion/presenter && npm test
```

Expected: existing tests pass (they only check command IDs which are unchanged).

If tests reveal a breakage (e.g. a test parses the source for the OLD `seconds` field), report and adapt.

- [ ] **Step 8: Commit**

```bash
git add ops/companion/presenter/index.js
git commit -m "feat(companion): live-state dropdown + preach limit in minutes (#270 #249)

- broadcast.set_live: checkbox→dropdown so 'Live ON' / 'Live OFF'
  shows on the Companion button face. Wire format unchanged
  ({ enabled: boolean }).
- timer.set_preach_limit: input is minutes (default 45, range 1-240).
  Handler multiplies by 60; wire format unchanged ({ seconds: number })."
```

---

## Task 3: Add structural assertions in `commands.test.js`

**Files:**
- Modify: `ops/companion/presenter/lib/commands.test.js` (append new test block)

The existing tests parse `index.js` source via regex and verify command IDs. Add a parallel block that verifies the new option ids ("minutes", "state") exist in the source and that the new handler wire format ("minutes * 60", "state === \"on\"") is present.

- [ ] **Step 1: Append the new test block at the end of the file**

In `ops/companion/presenter/lib/commands.test.js`, append (before the final newline):

```javascript

describe("Companion action UX (#270 #249)", () => {
  test("timer.set_preach_limit input uses 'minutes' field with default 45", () => {
    // The action options switch case for timer.set_preach_limit must
    // expose a 'minutes' input with default 45 (not 'seconds' / 2700).
    const preachOptionsRegion = indexSource.match(
      /case ["']timer\.set_preach_limit["']:\s*return\s*\[([\s\S]*?)\];/,
    );
    assert.ok(
      preachOptionsRegion,
      "Could not find timer.set_preach_limit options block",
    );
    const optionsText = preachOptionsRegion[1];
    assert.match(
      optionsText,
      /id:\s*["']minutes["']/,
      "preach-limit input id should be 'minutes'",
    );
    assert.match(
      optionsText,
      /label:\s*["']Limit \(minutes\)["']/,
      "preach-limit label should say (minutes)",
    );
    assert.match(
      optionsText,
      /default:\s*45\b/,
      "preach-limit default should be 45",
    );
  });

  test("timer.set_preach_limit handler multiplies minutes by 60", () => {
    const handlerRegion = indexSource.match(
      /case ["']timer\.set_preach_limit["']:\s*\{([\s\S]*?)\}/,
    );
    assert.ok(handlerRegion, "Could not find timer.set_preach_limit handler");
    const handlerText = handlerRegion[1];
    assert.match(
      handlerText,
      /options\.minutes/,
      "handler should read options.minutes",
    );
    assert.match(
      handlerText,
      /\*\s*60\b/,
      "handler should multiply by 60 to convert minutes → seconds",
    );
  });

  test("broadcast.set_live input is a dropdown with 'state' field", () => {
    const liveOptionsRegion = indexSource.match(
      /case ["']broadcast\.set_live["']:\s*return\s*\[([\s\S]*?)\];/,
    );
    assert.ok(
      liveOptionsRegion,
      "Could not find broadcast.set_live options block",
    );
    const optionsText = liveOptionsRegion[1];
    assert.match(
      optionsText,
      /type:\s*["']dropdown["']/,
      "broadcast.set_live input should be a dropdown",
    );
    assert.match(
      optionsText,
      /id:\s*["']state["']/,
      "broadcast.set_live id should be 'state'",
    );
    assert.match(
      optionsText,
      /id:\s*["']on["']/,
      "dropdown should have an 'on' choice",
    );
    assert.match(
      optionsText,
      /id:\s*["']off["']/,
      "dropdown should have an 'off' choice",
    );
  });

  test("broadcast.set_live handler maps state==='on' to enabled boolean", () => {
    const handlerRegion = indexSource.match(
      /case ["']broadcast\.set_live["']:\s*\{([\s\S]*?)\}/,
    );
    assert.ok(handlerRegion, "Could not find broadcast.set_live handler");
    const handlerText = handlerRegion[1];
    assert.match(
      handlerText,
      /options\.state\s*===\s*["']on["']/,
      "handler should compare options.state === 'on'",
    );
  });

  test("action label for timer.set_preach_limit says (minutes)", () => {
    // The action registration entry in COMMANDS must show "(minutes)" so
    // the operator sees the unit when picking the action.
    assert.match(
      indexSource,
      /id:\s*["']timer\.set_preach_limit["'],\s*label:\s*["'][^"']*\(minutes\)/,
      "timer.set_preach_limit label should contain '(minutes)'",
    );
  });
});
```

- [ ] **Step 2: Run tests**

```bash
cd ops/companion/presenter && npm test
```

Expected: all tests pass — the original 3 (command IDs) plus the 5 new ones.

If any new test fails because Task 2 didn't apply the change correctly (e.g. forgot to rename `seconds` → `minutes` in one location), surface that to Task 2 and re-run.

- [ ] **Step 3: Commit**

```bash
git add ops/companion/presenter/lib/commands.test.js
git commit -m "test(companion): assert new action UX shapes (#270 #249)

Five new structural assertions verify the new option ids ('minutes',
'state'), the dropdown choices ('on'/'off'), and the handler wire-
format conversion ('minutes * 60', 'state === \"on\"')."
```

---

## Task 4: Local checks, push, monitor CI, open PR, completion report

**Controller-handled task.**

- [ ] **Step 1: Run all local checks**

```bash
cd ops/companion/presenter && npm test && cd ../../..
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all
cargo test --workspace -- --nocapture
```

Expected: all pass.

- [ ] **Step 2: Push to dev**

```bash
git push origin dev
```

- [ ] **Step 3: Monitor CI to terminal state**

```bash
gh run list --branch dev --limit 1 --json databaseId --jq '.[0].databaseId'
sleep 1500 && gh run view <run-id> --json status,conclusion,jobs --jq '{status, conclusion, failed: [.jobs[] | select(.conclusion == "failure") | .name]}'
```

If any job fails, `gh run view <run-id> --log-failed`, fix in ONE commit, push again, re-monitor.

- [ ] **Step 4: Verify dev deployment is live**

```bash
curl -s http://10.77.8.134:8080/healthz
```

Expected: `{"channel":"dev","status":"ok","version":"0.4.67"}`. The server itself doesn't change behavior, but the version proves the merge deployed.

- [ ] **Step 5: Open PR**

```bash
gh pr create --title "feat(companion): live-state dropdown + preach minutes (#270 #249)" --body "$(cat <<'EOF'
## Summary

Two Bitfocus Companion plugin UX fixes:

- **#270:** `broadcast.set_live` action converts from a `checkbox` (which doesn't show its state on the Companion button) to a `dropdown` with `Live ON` / `Live OFF` choices. The button face now displays the chosen label.
- **#249:** `timer.set_preach_limit` action takes a value in **minutes** (default 45, range 1-240) instead of seconds. The handler multiplies by 60 so the server still receives `{ seconds: number }`.

Plugin version bumps from 0.7.0 → 0.8.0.

## Wire format

Server-side handlers UNCHANGED. Server still receives:
- `{ seconds: number }` for preach limit
- `{ enabled: boolean }` for broadcast live state

## Backwards compatibility

Existing Companion button bindings will lose their saved options on plugin upgrade (Companion drops unknown option ids):
- `enabled: true/false` → dropdown defaults to `Live ON`
- `seconds: <n>` → minutes input defaults to 45

Operators re-bind affected buttons after upgrade.

## Deploy

The Companion plugin ships via the `release.yml` workflow when a Release is cut. Merging this PR to main does NOT auto-deploy the plugin to companion-pp.lan — a release tag must be cut separately. After merge:

```bash
gh release create v0.4.67 --generate-notes
```

(or use the GitHub web UI). The release workflow then deploys the plugin tarball to companion-pp.lan.

## Test plan

- [x] Plugin tests pass (3 existing + 5 new structural assertions)
- [x] All workspace Rust tests pass
- [x] Dev `/healthz` reports v0.4.67
- [x] Manual verification deferred to post-release on companion-pp.lan (see deploy note)

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 6: Verify PR is mergeable**

```bash
gh pr view <pr-number> --json mergeable,mergeStateStatus
```

Expected: `mergeable: MERGEABLE`, `mergeStateStatus: CLEAN` (after Mutation Testing + Label PR finish). If `UNSTABLE` due to checks still pending, wait. If anything failed, fix.

- [ ] **Step 7: Run pre-completion gates**

Invoke `/plan-check` skill — must come back N/N fulfilled. Invoke `/review` skill on this PR — must come back `0 🔴 0 🟡 0 🔵`. Fix any findings inside the diff before sending the completion report.

- [ ] **Step 8: Send completion report**

Per `core/completion-report.md`. Include:

- ✅ CI green (run id)
- ✅ /plan-check N/N
- ✅ /review clean
- ✅ Deploy: dev shows v0.4.67. **Note: Companion plugin needs a release tag to deploy to companion-pp.lan — call this out in the report so the user knows the plugin isn't yet on the production-PP host even after the PR merges.**
- 🌐 Dev URL
- 🌐 Prod URL (server only — plugin requires release)
- PR number + title + URL — mergeable, clean

The completion report should explicitly note: "Companion plugin will deploy to companion-pp.lan when a release tag is cut on main."

---

## Verification Summary

| Check | How to verify |
|-------|---------------|
| Workspace + plugin versions bumped | `grep '^version' Cargo.toml`, `grep version ops/companion/presenter/companion/manifest.json` |
| Action label says minutes | New test `action label for timer.set_preach_limit says (minutes)` passes |
| Preach limit input is minutes/45 | New test `timer.set_preach_limit input uses 'minutes' field with default 45` passes |
| Preach handler multiplies by 60 | New test `timer.set_preach_limit handler multiplies minutes by 60` passes |
| Broadcast input is dropdown | New test `broadcast.set_live input is a dropdown with 'state' field` passes |
| Broadcast handler maps state→bool | New test `broadcast.set_live handler maps state==='on' to enabled boolean` passes |
| No regressions | Existing 3 plugin tests + workspace Rust tests still pass |
| CI green | All Pipeline jobs ✅ |
| Plugin deploy tracking | Completion report calls out the release-tag requirement |
