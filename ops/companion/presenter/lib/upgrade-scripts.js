// #805: Companion upgrade scripts — the module's migrations of stored action
// options. Companion persists every action's options; renaming an option
// without a migration leaves already-saved actions carrying only the OLD key,
// which the new handler never reads. Commit 956af894 (#249 / #270) renamed two
// options that way:
//   - `timer.set_preach_limit`: `seconds` → `minutes` (legacy presses sent the
//     45-min default instead of the configured limit);
//   - `broadcast.set_live`: checkbox `enabled` → dropdown `state` "on"/"off"
//     (a legacy "Live ON" `{enabled: true}` button sent `enabled: false`).
// For each: an upgrade script migrates the stored options, and a `legacy*`
// reader lets the action handler honour an action not yet upgraded.
// Dependency-free so it is tested without the Companion runtime.

const PREACH_LIMIT_ACTION_ID = "timer.set_preach_limit";
const LIVE_ACTION_ID = "broadcast.set_live";

function isSet(value) {
  return value !== undefined && value !== null && value !== "";
}

// The raw legacy `seconds` of a pre-#249 preach-limit action, or null when the
// options are not legacy-shaped: `minutes` set (it wins), `seconds` absent, or
// `seconds` not a positive number.
function legacyRawSeconds(options) {
  const opts = options || {};
  if (isSet(opts.minutes) || !isSet(opts.seconds)) {
    return null;
  }
  const seconds = Number(opts.seconds);
  return Number.isFinite(seconds) && seconds > 0 ? seconds : null;
}

/**
 * The limit a legacy (pre-#249) `timer.set_preach_limit` action stores, in
 * whole seconds (≥ 1, the server parses the payload as u64) — or null when the
 * options are not legacy-shaped.
 */
function legacyPreachLimitSeconds(options) {
  const seconds = legacyRawSeconds(options);
  return seconds === null ? null : Math.max(1, Math.round(seconds));
}

/**
 * The live state a legacy (pre-#270) `broadcast.set_live` action stores — its
 * old `enabled` checkbox, read exactly as the old handler did
 * (`Boolean(options.enabled)`) — or null when `state` is set (it wins) or the
 * action never stored `enabled`.
 */
function legacyLiveEnabled(options) {
  const opts = options || {};
  if (isSet(opts.state) || opts.enabled === undefined) {
    return null;
  }
  return Boolean(opts.enabled);
}

// Shared shape of one option-rename migration: for each action of `actionId`
// whose `readLegacy(options)` is non-null, drop `legacyKey` and merge in
// `toOptions(legacyValue)`. Only changed actions are returned; config and
// feedbacks are never touched (@companion-module/base
// CompanionStaticUpgradeScript contract).
function optionRenameScript(actionId, legacyKey, readLegacy, toOptions) {
  return function (_context, props) {
    const updatedActions = [];
    for (const action of props.actions) {
      if (action.actionId !== actionId) {
        continue;
      }
      const legacy = readLegacy(action.options);
      if (legacy === null) {
        continue;
      }
      const rest = { ...action.options };
      delete rest[legacyKey];
      action.options = { ...rest, ...toOptions(legacy) };
      updatedActions.push(action);
    }
    return {
      updatedConfig: null,
      updatedSecrets: null,
      updatedActions,
      updatedFeedbacks: [],
    };
  };
}

// `{seconds: N}` → `{minutes: max(1, round(N / 60))}`.
const upgradeLegacyPreachLimitSeconds = optionRenameScript(
  PREACH_LIMIT_ACTION_ID,
  "seconds",
  legacyRawSeconds,
  (seconds) => ({ minutes: Math.max(1, Math.round(seconds / 60)) }),
);

// `{enabled: B}` → `{state: Boolean(B) ? "on" : "off"}`.
const upgradeLegacyLiveEnabled = optionRenameScript(
  LIVE_ACTION_ID,
  "enabled",
  legacyLiveEnabled,
  (enabled) => ({ state: enabled ? "on" : "off" }),
);

// Handed to `runEntrypoint`. APPEND-ONLY: Companion records, per connection,
// the index of the last script it ran (`lastUpgradeIndex`) and runs only the
// scripts after it. Never reorder or delete an entry — retire one by
// replacing it IN PLACE with an inline no-op of the same shape as base's
// `EmptyUpgradeScript` (this lib stays dependency-free, so do not import it):
// `() => ({ updatedConfig: null, updatedSecrets: null, updatedActions: [],
// updatedFeedbacks: [] })`. A future option rename adds its migration at the
// END of this list.
const UPGRADE_SCRIPTS = [upgradeLegacyPreachLimitSeconds, upgradeLegacyLiveEnabled];

module.exports = {
  legacyPreachLimitSeconds,
  legacyLiveEnabled,
  UPGRADE_SCRIPTS,
};
