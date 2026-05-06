# Companion plugin: live-state dropdown + preach limit in minutes

**Issues:** #270, #249

**Date:** 2026-05-06

**Branch:** dev (workspace 0.4.66, plugin 0.7.0 → 0.8.0)

## Problem

Two Bitfocus Companion plugin issues, both in `ops/companion/presenter/index.js`:

**#270 — `broadcast.set_live` action shows no state.** The action uses a `checkbox` input. Companion checkboxes don't render their bound state on the button face — the operator can't tell whether their button is configured for "go live" or "go offline" without opening the action editor. Switch to a `dropdown` so the chosen option label shows on the button.

**#249 — `timer.set_preach_limit` is in seconds.** The action's input field is "Limit (seconds)" with default 2700. Operators think in minutes (a sermon is "45 minutes", not "2700 seconds"). Switch the field to minutes; multiply by 60 on the wire so the server still receives seconds.

## Goal

The two actions are simpler to configure and the live-state action shows its bound value on the Companion button face.

## Architecture

Single file change: `ops/companion/presenter/index.js`. Plugin version bumps 0.7.0 → 0.8.0 in `companion/manifest.json`. Tests at `lib/commands.test.js` updated for the new payload shapes.

### Fix #270 — checkbox → dropdown for `broadcast.set_live`

Action input definition (currently around lines 392-400):

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

Action handler (currently around lines 527-530):

```javascript
case "broadcast.set_live": {
  payload = {
    enabled: options.state === "on",
  };
  break;
}
```

Wire payload stays as `{ enabled: boolean }`. Server-side handler is unchanged.

### Fix #249 — seconds → minutes for `timer.set_preach_limit`

Action label (currently around line 62):

```javascript
{ id: "timer.set_preach_limit", label: "Timer: set preach limit (minutes)" }
```

Action input definition (currently around lines 361-365):

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

Action handler (currently around lines 511-513):

```javascript
case "timer.set_preach_limit": {
  payload = {
    seconds: (Number(options.minutes) || 45) * 60,
  };
  break;
}
```

Wire payload still `{ seconds: number }`. Server-side handler unchanged. The 1–240 minute range is a sanity guardrail (1 min lower bound, 4 hours upper).

### Plugin version bump

`ops/companion/presenter/companion/manifest.json` field `version` 0.7.0 → 0.8.0. The bump signals a behavior-affecting change so users know the action options changed.

## Backwards compatibility

Existing Companion button configurations bound to either action will lose their saved option on plugin upgrade (Companion drops unknown option ids):

- Buttons with `enabled: true/false` lose the value; the new dropdown defaults to "Live ON".
- Buttons with `seconds: <n>` lose the value; the new minutes field defaults to 45.

Both are acceptable for a v0.x plugin. Operators re-bind affected buttons after upgrade. The plugin's RELEASE notes (or HELP.md) should call this out so the upgrade isn't surprising.

## Testing

### Unit tests in `ops/companion/presenter/lib/commands.test.js`

Existing tests at lines 30-31 and 36 reference the action ids. Verify each test:

- If the test only asserts that the action ids are registered, no change needed.
- If the test asserts payload shapes for the OLD inputs (`enabled` checkbox or `seconds` field), update to assert the NEW shape (dropdown `state` → boolean payload, `minutes` field → seconds payload `× 60`).

Add 2 new assertions if the file already has a "payload shape per action" test pattern:

1. `timer.set_preach_limit` with `options = { minutes: 30 }` produces `payload = { seconds: 1800 }`.
2. `broadcast.set_live` with `options = { state: "on" }` produces `payload = { enabled: true }`.
3. `broadcast.set_live` with `options = { state: "off" }` produces `payload = { enabled: false }`.

If `commands.test.js` doesn't have such a pattern, add a new `describe("action payloads")` block that does.

### Manual verification on companion-pp.lan after deploy

After the release ships:

1. Open Bitfocus Companion on the production-PP host.
2. Add a `Broadcast: set live state` button. Confirm the action editor shows the new "Live state" dropdown with "Live ON" / "Live OFF" options. Confirm the button face displays "Live ON" or "Live OFF" depending on the chosen value.
3. Add a `Timer: set preach limit (minutes)` button. Confirm the action editor shows "Limit (minutes)" with default 45 and min 1, max 240. Set value 30, press the button, verify the preach timer state on the operator UI shows the limit at 30 min (= 1800 s).
4. Existing buttons (if any from the previous plugin version) — re-bind their action options.

### Deploy

Per project convention, the Companion plugin ships via the `release.yml` GitHub workflow when a Release is cut. After merging the PR to main, the controller (or human) creates a release tag matching the workspace version, which deploys the plugin tarball to companion-pp.lan. The plugin version (0.8.0) in `manifest.json` is what Companion sees as the upgrade.

## Files

| File | Change |
|---|---|
| `ops/companion/presenter/index.js` | Action label change, two action input definitions, two payload handler updates |
| `ops/companion/presenter/companion/manifest.json` | `version` 0.7.0 → 0.8.0 |
| `ops/companion/presenter/lib/commands.test.js` | Update or add payload-shape assertions for the two actions |

## Acceptance

- All Companion plugin tests pass (`npm test` from `ops/companion/presenter/`).
- Workspace tests pass.
- Manual verification on companion-pp.lan after release confirms both actions work as expected.
- Operators can SEE the live state on the button face (no more invisible checkbox).
- Preach limit fields read in minutes.

## Out of scope

- Adding a "Toggle current" third dropdown option (rejected during brainstorming — user picked the simpler 2-option list).
- Server-side changes (server's preach-limit handler still receives seconds; broadcast handler still receives `{enabled: boolean}`).
- Changes to other actions (clip triggers, stage navigation, etc.).

## Risk

- Low. Single plugin file change. Server-side wire format unchanged. Companion's option-drop-on-upgrade is the only user-visible side effect — documented above.
- The release deploy is a separate step (cut a release tag); the merge alone doesn't ship to companion-pp. Worth calling this out in the PR completion report.
