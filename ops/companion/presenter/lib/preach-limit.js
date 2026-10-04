// #805: `timer.set_preach_limit` stored its limit as `options.seconds` until
// #249 switched the option to `options.minutes` — without an upgrade script,
// so every action saved before that change silently sent the 45-min default.
// This module holds the dependency-free logic for both halves of the fix:
// the Companion upgrade script that migrates stored actions, and the legacy
// read the action handler falls back on for any config not yet upgraded.

const PREACH_LIMIT_ACTION_ID = "timer.set_preach_limit";

function isSet(value) {
  return value !== undefined && value !== null && value !== "";
}

/**
 * The limit a legacy (pre-#249) `timer.set_preach_limit` action stores, in
 * whole seconds — or null when the options are not legacy-shaped: `minutes`
 * set (it wins), `seconds` absent, or `seconds` not a positive number.
 * Whole seconds because the server parses the payload as u64.
 */
function legacyPreachLimitSeconds(options) {
  const opts = options || {};
  if (isSet(opts.minutes) || !isSet(opts.seconds)) {
    return null;
  }
  const seconds = Math.round(Number(opts.seconds));
  return Number.isFinite(seconds) && seconds > 0 ? seconds : null;
}

/**
 * Companion upgrade script (@companion-module/base CompanionStaticUpgradeScript):
 * rewrite each legacy `{seconds: N}` preach-limit action to
 * `{minutes: max(1, round(N / 60))}`, dropping `seconds`. Only changed actions
 * are returned; config and feedbacks are never touched.
 */
function upgradeLegacyPreachLimitSeconds(_context, props) {
  const updatedActions = [];
  for (const action of props.actions) {
    if (action.actionId !== PREACH_LIMIT_ACTION_ID) {
      continue;
    }
    const seconds = legacyPreachLimitSeconds(action.options);
    if (seconds === null) {
      continue;
    }
    const { seconds: _legacySeconds, ...rest } = action.options;
    action.options = {
      ...rest,
      minutes: Math.max(1, Math.round(seconds / 60)),
    };
    updatedActions.push(action);
  }
  return {
    updatedConfig: null,
    updatedSecrets: null,
    updatedActions,
    updatedFeedbacks: [],
  };
}

// Handed to `runEntrypoint`. APPEND-ONLY: Companion records, per connection,
// the index of the last script it ran (`lastUpgradeIndex`) and runs only the
// scripts after it. Never reorder or delete an entry — retire one by
// replacing it with base's `EmptyUpgradeScript`.
const UPGRADE_SCRIPTS = [upgradeLegacyPreachLimitSeconds];

module.exports = {
  PREACH_LIMIT_ACTION_ID,
  legacyPreachLimitSeconds,
  upgradeLegacyPreachLimitSeconds,
  UPGRADE_SCRIPTS,
};
