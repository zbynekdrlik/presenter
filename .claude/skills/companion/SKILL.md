---
name: presenter-companion
description: >
  Bitfocus Companion plugin setup and gotchas for the presenter project — /etc/hosts override,
  setVariableValues batching, and version sync. Use when working on the Companion plugin or
  CI Deploy Companion Plugin job.
triggers:
  - Companion
  - companion plugin
  - companion-pp
  - setVariableValues
  - companion/ws
---

# Presenter Companion Skill

## companion-pp.lan Access (Tailscale)

The "Companion PP" box is NOT on the LAN anymore — it joins via Tailscale. CI's
`Deploy Companion Plugin` job runs on the self-hosted runner and SSHs to `companion-pp.lan`.

**Fix applied 2026-04-14:** `/etc/hosts` on dev machine maps the name:

```
100.101.72.101 companion-pp.lan companion-pp
```

`/etc/hosts` takes priority over DNS (nsswitch), so `ssh-keyscan` and `ssh` both resolve
the Tailscale IP. If `Deploy Companion Plugin` starts failing on `ssh-keyscan companion-pp.lan`:
1. Verify Tailscale IP: `tailscale status | grep companion-pp` or ask the user.
2. Update the `/etc/hosts` entry.
3. If the box is fully offline → machine-availability issue, not config.

## setVariableValues Batching (MUST)

`ops/companion/presenter/index.js` connects to `/companion/ws` and receives full-snapshot
`variables` messages. The plugin MUST collapse ONE server message into ONE `setVariableValues(batch)`
call. Per-variable calls trigger full Companion state propagation AND re-evaluation of every
advanced feedback per call — 6 timer variables × 1 Hz = 6× the feedback work per second.

**Why:** Bug #265 ("Companion is slow after some update"). Co-location (same machine) made it
worse (no network-level WS coalescing). PR #214 (April 2026) added 39 variables + 39
`text_<var>` advanced feedbacks, amplifying the cost.

**Implementation:** `ops/companion/presenter/lib/variable-batch.js` exports `computeVariableBatch`
+ `applyVariablesMessage`. The `case "variables":` arm in `index.js` delegates entirely to
`applyVariablesMessage`.

**Rules when touching Companion plugin message handling:**
- NEVER call `setVariableValues` inside a per-entry `forEach` / `for...of` loop.
- Always build ONE batch object from a server message, emit ONE call.
- Call `setVariableValues(batch)` BEFORE writing local cache — on throw, leave cache clean.
- Tests in `lib/variable-batch.test.js` enforce this with a behavior-based stub.
- Bump `package.json` `version` AND `companion/manifest.json` `version` atomically.

## Adding actions / variables / tests (gotchas, #712)

- **The "Companion Tests" CI job runs a HARDCODED test-file list.** Root
  `package.json` → `test:companion` names each `lib/*.test.js` explicitly
  (`node --test lib/time.test.js lib/commands.test.js ...`). A NEW test file is
  **not** discovered automatically — add its path to that script or CI silently
  never runs it. Run it locally the same way (`npm run test:companion`); it needs
  only `node` (built-in `node:test`/`node:assert`), no cargo, no npm install.
- **`lib/commands.test.js` asserts an EXACT command count** and text-parses the
  `COMMANDS` array from `index.js` source via regex. Adding a command means
  updating its `EXPECTED_COMMANDS` list too, or the parity test fails. Because it
  is a **source-text regex parse**, `COMMANDS` (and `VARIABLE_DEFINITIONS`, parsed
  the same way in `lib/stream.test.js`) MUST stay **inline array literals** — a
  `[...base, ...IMPORTED]` spread breaks the parse (the spread's ids are invisible
  to the regex).
- **Pure logic goes in `lib/<area>.js`, tested without `@companion-module/base`.**
  `index.js` `require`s the Companion runtime at load, so it can't be imported in
  `node --test`; keep new logic (payload builders, option fields) in a dependency-free
  `lib/` module (`lib/time.js`, `lib/variable-batch.js`, `lib/stream.js`) and let
  `index.js` be a thin adapter. Tests then import the lib module directly.
- **Stream command family (`lib/stream.js`).** Wire command names MUST start with
  `stream_` — the server (`companion/stream.rs`, #711) delegates by that prefix; the
  Companion action id IS the wire command name. Scenes are addressed BY NAME
  (server matches case-insensitively); `output` slug is optional, default `stream`.
  Unknown command on an older server → non-fatal `{type:"error"}` reply the plugin
  already logs (`case "error"`) → graceful degrade, no crash. Payload builders must
  never throw.
- **Profile: `ops/companion/generated/` is gitignored.** Edit + commit the SOURCE
  blueprint `ops/companion/presenter-companion-profile.json`; run
  `node ops/companion/generate-profile.mjs` only to VERIFY it still renders (the
  export under `generated/` is never committed).
