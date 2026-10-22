---
name: presenter-companion
description: >
  Bitfocus Companion plugin setup and gotchas for the presenter project — /etc/hosts override,
  setVariableValues batching, and version sync. Use when working on the Companion plugin or
  the "Deploy Companion module" step of deploy.yml / release.yml.
triggers:
  - Companion
  - companion plugin
  - companion-pp
  - setVariableValues
  - companion/ws
---

# Presenter Companion Skill

## companion-pp.lan Access (Tailscale)

The "Companion PP" box is NOT on the LAN anymore — it joins via Tailscale. release.yml's
`Deploy Companion module` step runs on the self-hosted runner and SSHs to `companion-pp.lan`.

**The module ships with the RELEASE, never with a dev push (#850).** Companion runs only on the
production hosts. `scripts/deploy/deploy_companion_module.sh <alias> <label>` runs as the
"Deploy Companion module" step of deploy.yml (SNV, merge to main) and release.yml (PP, GitHub
Release), after the server is up. pipeline.yml used to deploy it on every dev push, which
restarted the operators' Companion on both hosts ~50 min after any push (2026-10-14: a PP event at
18:00) and ran unreleased plugin code next to the released server. `check_deploy_workflows.py`
invariant 5 blocks a Companion deploy / `systemctl … companion` in any dev-push workflow. To try a
plugin change before a release, use the `node --test` suites and `lib/test-support.js`.

**Fix applied 2026-04-14:** `/etc/hosts` on dev machine maps the name:

```
100.101.72.101 companion-pp.lan companion-pp
```

`/etc/hosts` takes priority over DNS (nsswitch), so `ssh-keyscan` and `ssh` both resolve
the Tailscale IP. If release.yml's SSH setup starts failing on `ssh-keyscan companion-pp.lan`:
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
  `index.js` `require`s the Companion runtime at load, so a plain `require` fails in
  `node --test`; keep new logic (payload builders, option fields) in a dependency-free
  `lib/` module (`lib/time.js`, `lib/variable-batch.js`, `lib/stream.js`) and let
  `index.js` be a thin adapter. Tests then import the lib module directly. To test the
  REAL adapter (wiring, handler payloads, `_handleMessage`), use the shared
  `loadPresenterModule()` in `lib/test-support.js` (stubs `@companion-module/base` + `ws`
  via a `Module._load` hook; #805/#814, see `.claude/rules/companion-upgrade-scripts.md`).
- **Renaming an action option, or retyping it so the stored value's shape changes ⇒
  append an upgrade script** to `lib/upgrade-scripts.js` `UPGRADE_SCRIPTS` (append-only) + a
  handler fallback, or every saved button silently keeps the old key (#805) — a type-only change
  that keeps the key and a string value (textinput → `allowCustom` dropdown, #814) needs none —
  `.claude/rules/companion-upgrade-scripts.md`.
- **Stream command family (`lib/stream.js`).** Wire command names MUST start with
  `stream_` — the server (`companion/stream.rs`, #711) delegates by that prefix; the
  Companion action id IS the wire command name. Scenes are addressed BY NAME
  (server matches case-insensitively); `output` slug is optional, default `stream`.
  Unknown command on an older server → non-fatal `{type:"error"}` reply the plugin
  already logs (`case "error"`) → graceful degrade, no crash. Payload builders must
  never throw.
- **Nameplate grid buttons address a SLOT (position in the plate list), not a plate
  id (#870).** `stream_nameplate_slot {slot, mode?, expect_id?, expect_empty?}` (always the `stream`
  output; no output option) + label variables `nameplate_slot_<n>_name/_role/_id` +
  feedback `stream_nameplate_slot_active` + presets "Menovky — mriežka" (song, then slots
  1..11). The server resolves label and press from one list, and refuses a press whose
  `expect_id` (the label's plate id, from the module cache) no longer matches the slot;
  details in `.claude/rules/stream-graphics.md` (#779 section).
- **Profile: `ops/companion/generated/` is gitignored.** Edit + commit the SOURCE
  blueprint `ops/companion/presenter-companion-profile.json`; run
  `node ops/companion/generate-profile.mjs` only to VERIFY it still renders (the
  export under `generated/` is never committed).

## Dropdown choices come from the server `catalog` push (#814)

Layouts / outputs / scenes / overlays are NEVER a hardcoded list — the server pushes a
`catalog` message and the module rebuilds its dropdowns from it. Full pattern (server
`companion/catalog.rs`, module `lib/catalog.js`, legacy-value compatibility, tests):
`.claude/rules/companion-catalog.md` (auto-loads on the module + server companion files).

## Feedbacks — boolean type + checkFeedbacks (#780)

The 2026-09 "feedbacks never light" bug. Three rules when touching feedback
definitions in `index.js` `_setupFeedbacks()`:

- **A boolean condition needs `type:"boolean"` + `defaultStyle`, NEVER
  `type:"advanced"`.** `@companion-module/base` ^1.13 ignores a boolean returned
  from an `advanced` callback (an advanced callback must return a *style object*),
  so the button silently never lights and Companion shows it as an empty "Layered
  Styles Overrides" feedback. `countdown_running` is the correct reference shape.
- **`checkFeedbacks()` MUST be called to re-evaluate.** Companion evaluates a
  feedback at creation and then only when the module asks. Call it once per
  applied `variables` message that changed ≥1 value — reuse
  `applyVariablesMessage`'s return (0/1 changed-count, `lib/variable-batch.js`),
  do NOT loop-call per variable (that reintroduces the #265 fan-out) — and on
  connection-state change (`_updateVariable` on `live_ws_connected` is a
  single-call, non-loop path, safe to fire `checkFeedbacks()` there).
- **Match a variable's real shape.** `stream_overlays` is a comma-joined list of
  active overlay names (`"-"` when none), so an exact `=== "verse"` match breaks
  the moment a 2nd overlay is on. Use MEMBERSHIP (`isOverlayActive` in
  `lib/stream.js`), case-insensitive/trimmed, exact (not substring) — not
  whole-string equality. Pure evaluators live in `lib/stream.js` + `node --test`.

## Copying buttons between the SNV and PP Companions (2026-10-11)

There is no shared profile: each rig's buttons live only in its own `v5.0/db.sqlite`
(`pages` holds `{name, controls:{row:{col:controlId}}}`, `controls` holds the button JSON).
Pages match by number on both rigs (page 30 = SNV "strih-obs" / PP "STRIH 2ME").

- **Read first, read-only:** open the DB with `?mode=ro`. Labels are in
  `style.layers[type=text].text.value`. Actions are in `steps.<n>.action_sets.down[]`,
  with nested `children` for `logic_if`. Feedbacks are in `feedbacks[]`.
- **Connection ids differ per rig and some labels too.** For example SNV `strih-obs` is PP
  `2ME_obs`, and `presenter` is `presenter` on both. Remap every `connectionId` by label and
  abort on an unmapped id. Regenerate every action/feedback `id` and `overrideId`.
- **Module versions can differ.** SNV obs-studio is 3.15.3 (stores `upgradeIndex` 8), PP is
  3.13.1 (index 7). Clamp `upgradeIndex` to the target rig's existing values for that
  connection; never hand a module an index it does not know.
- **Write with Companion STOPPED, as the `companion` user** (root-owned `-wal`/`-shm` files
  would break it). Back up `db.sqlite` (+wal/shm) to `backups/pre-<what>-<ts>/` first,
  `pragma wal_checkpoint(TRUNCATE)`, then start it again. The script used on 2026-10-11 is
  kept outside the repo (work-products `companion_copy_page30_buttons.py`).
- **Before replacing, check what the target button already does.** PP page 30 row 1 still
  fired Resolume CG clips over OSC (`cg_resolume send_blank /composition/groups/2/columns/N/connect`).
  A button whose connection is down shows Companion's red warning triangle, and the owner reads
  that as "error". Moving a CG button to Presenter therefore needs:
  - every overlay toggle with its own `stream_overlay_active` feedback. SNV "ucet dole" had none,
    so it never lit;
  - a "Clear" made of `stream_overlay_off` per overlay. `stream_clear` also clears the BASE scene,
    which the old Resolume Clear never did.
  - Presenter stream scenes have no video element, so a Resolume "CG VIDEO" clip has no
    Presenter equivalent.
- **Test without the UI:** the Companion HTTP API on `:8000`:
  - press a button: `POST /api/location/<page>/<row>/<col>/press`
  - read a variable: `GET /api/variable/<connection label>/<var>/value`, e.g. `presenter/stream_overlays`
  - Press twice to restore. Read the full action list before pressing (#805).
- **OBS state without the Windows MCP:** Companion bundles node 26 (global `WebSocket`) at
  `/opt/companion/node-runtimes/main/bin/node`. obs-websocket v5 on `strih.lan:4455` has no
  password: send `Identify` (op 1), then `GetSourceActive` / `GetSceneItemList`. The CG
  button's "Show/Hide on CG" hotkeys change whether `CG-presenter` is ACTIVE in program
  (`videoActive`), not its `sceneItemEnabled` inside "CG bridge".

## Module version pinning — connection must survive a deploy (#733)

Companion stores each connection's module version as `moduleVersionId` in the `instances`
table of its config SQLite DB (`~/.config/companion-nodejs/v<major>.<minor>/db.sqlite` — the
active dir matches the running Companion version, e.g. `v5.0` for 5.0.3). `moduleVersionId`
is one of: a **concrete version** ("0.9.0"), **null** (= latest), or the special stable id
**"dev"** for a module loaded from `--extra-module-path`.

**The trap:** a connection pinned to a CONCRETE version breaks with a "missing version"
error the moment a deploy bumps the plugin version — the old version id no longer exists, and
a human must re-pin it in the UI. This is what bit SNV (it pinned "0.9.0", installed under the
store dir `~/.config/companion-nodejs/modules/presenter`).

**The fix (both hosts, unified):** deploy the presenter module into
`--extra-module-path /opt/companion-module-dev/presenter` (a DEV module — stable id "dev",
auto-reloads on file change, takes precedence over any store copy) AND pin the connection to
`moduleVersionId="dev"`. Then a version bump never breaks the connection. Both hosts already
launch Companion with `--extra-module-path /opt/companion-module-dev`.

- **Deploy target is `/opt/companion-module-dev/presenter` on BOTH hosts** (SNV = prod
  10.77.9.205, PP = companion-pp.lan). Never the store dir `…/modules/presenter` — remove any
  stale store copy so it can't compete with the dev module.
- **Idempotent re-pin: `ops/companion/repin-presenter-connection.py`.** Run it with Companion
  STOPPED (the deploy does stop → swap module → repin → start). It resolves the active DB from
  `/opt/companion/BUILD` (`v<major>.<minor>/db.sqlite`; fails loud if BUILD names a version dir
  that is missing — never silently edits a legacy DB), backs up the DB (+ any `-wal`/`-shm`),
  sets `moduleVersionId="dev"` for every `moduleId="presenter"` connection, and is a no-op if
  already "dev". The deploy passes **`--expect-connection`** so a mis-resolved/empty DB with no
  presenter connection fails the deploy (exit 2) instead of reporting a false green. The box has
  `python3` + stdlib `sqlite3` but **no `sqlite3` CLI** — always python. Its pure logic +
  DB write path are unit/integration-tested in `repin-presenter-connection.test.py`, run in the
  `companion-tests` CI job **by file path** (`python3 ops/companion/repin-presenter-connection.test.py`;
  the hyphenated stem can't be imported via `-m unittest`).
- **Inspect a live connection's pin (read-only). Resolve the DB dir from `/opt/companion/BUILD` — as of 2026-10-04 BOTH hosts run Companion 5.0.6 → `v5.0`; the old `v4.x` dirs are stale leftovers (reading one gives a false "upgrade didn't run" — #805):**
  `sudo python3 -c "import sqlite3,json; [print(json.loads(v).get('moduleVersionId')) for i,v in sqlite3.connect('file:/home/companion/.config/companion-nodejs/v5.0/db.sqlite?mode=ro',uri=True).execute('select id,value from instances') if json.loads(v).get('moduleId')=='presenter']"`
- **Never press a rig button to test without reading its FULL action list first (#805 incident).**
  The preach-limit buttons are composite: one press also switches the stage layout, resets +
  starts the preach timer, clears the Bible and sets the stream scene. To check what a module
  action sends, read the stored action options (DB above) or press only a single-action test
  button; if a composite button was pressed, restore from the last `Presenter command …` lines
  in `journalctl -u companion`.
- **Upgrade scripts (Companion 5):** stored options are wrapped `{isExpression,value}`; Companion
  unwraps them before calling the module's upgrade scripts (`Fu()` in main.js) and re-wraps the
  result, so scripts see plain values. After a deploy, verify `lastUpgradeIndex` on the
  presenter connection and `upgradeIndex` on its actions in the `v5.0` DB.
