---
paths:
  - "ops/companion/presenter/**"
---

# Companion module: renaming an action option needs an upgrade script (#805)

Companion keeps every saved action's `options` in its config DB. If you **rename or retype an
action option** in `_commandOptionsFor` (`seconds` → `minutes`, a checkbox `enabled` → a dropdown
`state`), every button saved before the change still stores only the OLD key. The new handler
never reads that key. Nothing fails loudly: the press just sends the fallback.

Changing only an option's TYPE while keeping its key AND a string value — e.g. the #814
textinput → `dropdown` with `allowCustom: true` for `scene`/`output`/`code` — needs NO upgrade
script: the stored string is passed through unchanged. Prove it with a legacy-value press test
(`lib/catalog.test.js`).

Real case: commit `956af894` (#249 / #270) shipped two renames with no migration.
- Legacy preach-limit buttons sent the 45-min default (2700 s) instead of 7/5/60 min.
- Legacy "Live ON" buttons sent `enabled: false`.

**Every option rename/retype ships, in the same PR:**

1. **An upgrade script appended at the END of `UPGRADE_SCRIPTS`** in
   `ops/companion/presenter/lib/upgrade-scripts.js`. Build it with `optionRenameScript(actionId,
   legacyKey, readLegacy, toOptions)`: for each action of `actionId` where `readLegacy(options)`
   is non-null, it drops `legacyKey` and merges in `toOptions(value)`. `index.js` passes the list
   to `runEntrypoint(PresenterInstance, UPGRADE_SCRIPTS)`.
2. **A handler fallback** in `_sendCommand` that shares the script's legacy check (an exported
   `legacy*` reader; preach's handler reader also rounds to the whole seconds the server's u64
   expects). It covers a config the upgrade has not reached yet (an import, a not-yet-restarted
   rig).
3. Tests in `lib/upgrade-scripts.test.js`, plus an append-only pin: a connection at
   `lastUpgradeIndex` N still gets your new script.

**`UPGRADE_SCRIPTS` is APPEND-ONLY.** Companion stores each connection's `lastUpgradeIndex` and
runs only the scripts after it. An existing connection with no prior scripts is at -1, so it runs
them all once. A new connection starts at the end, so it runs none.
- Never reorder or delete an entry; already-upgraded connections would skip or re-run scripts.
- To retire a script, replace it IN PLACE with an inline no-op shaped like base's
  `EmptyUpgradeScript`: `() => ({ updatedConfig: null, updatedSecrets: null, updatedActions: [],
  updatedFeedbacks: [] })`. Do not import base into the dependency-free lib: under the test stub
  it resolves to `undefined`, and outside the stub CI has no base installed (MODULE_NOT_FOUND).

**Contract** (`@companion-module/base` 1.13/1.14, `internal/upgrade.js`):
- The script is `(context, {config, secrets, actions, feedbacks}) => {updatedConfig, updatedSecrets, updatedActions, updatedFeedbacks}`.
- `actions` are CLONES of `{id, controlId, actionId, options}`. Return only the CHANGED ones;
  the base applies them by `id`.
- Return `updatedConfig: null`, never `{}`, so the connection's host/port config is left alone.

To check against the real runner (base is not in the root devDependencies):
1. `npm install @companion-module/base@1` into a scratch dir.
2. Call `require("@companion-module/base/dist/internal/upgrade.js").runThroughUpgradeScripts(actions, [], lastUpgradeIndex, UPGRADE_SCRIPTS, config, null, false)`.

## Testing the real `index.js` adapter without the Companion runtime

`index.js` `require`s `@companion-module/base` and `ws` at load. Base is NOT installed for
`npm run test:companion` (CI runs only a root `npm ci`, and base is the module's own dependency).
`ws` is a root devDependency, but it is stubbed too, so no real socket is ever involved.
The shared test helper `lib/test-support.js` `loadPresenterModule()` (used by
`lib/upgrade-scripts.test.js` and `lib/catalog.test.js`) therefore stubs exactly those two
external modules through a `Module._load` hook, restored in `finally`, and then `require`s the
REAL `index.js`. It is not a `*.test.js` file, so it is never in the `test:companion` list.
- The fake `runEntrypoint` captures `(factory, upgradeScripts)`.
- The fake `InstanceBase` records `setActionDefinitions` / `setFeedbackDefinitions` /
  `setPresetDefinitions` / `setVariableDefinitions`, and counts the action + feedback
  registrations and `checkFeedbacks` calls in `instance.calls` (so a test can assert a message
  did / did not re-register definitions).
- A test then presses the registered action callback the way Companion does, with
  `instance.ws = {readyState: 1, send}`, and asserts on the sent JSON.

Use this for adapter-level behaviour (wiring, handler payloads). Keep pure logic in a
dependency-free `lib/*.js`.
