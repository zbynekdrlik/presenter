/**
 * Compute a single batched update object from a server "variables" payload.
 *
 * Semantics:
 *   - Entries with non-string `name` are skipped (defensive against malformed input).
 *   - Entries whose `name` is not in `variableDefinitions` are skipped (unknown vars).
 *   - `null` / `undefined` values are coerced to empty string to mirror what
 *     Companion expects.
 *   - Only CHANGED values (compared against `currentMap`) are included in the
 *     returned object; unchanged values are filtered out.
 *
 * The function does not mutate `currentMap`. Caller is responsible for writing
 * the batch back to the cache *after* the Companion `setVariableValues` call
 * succeeds (so a thrown call leaves the cache clean for retry on the next
 * message — see `applyVariablesMessage`).
 *
 * @param {unknown} values Server-side array of `{name, value}` entries.
 * @param {string[]} variableDefinitions Whitelist of accepted variable names.
 * @param {Map<string,string>} currentMap Current cached values per variable.
 * @returns {Object<string,string>} Diff object suitable for setVariableValues.
 */
function computeVariableBatch(values, variableDefinitions, currentMap) {
  const batch = {};
  if (!Array.isArray(values)) return batch;

  for (const entry of values) {
    if (!entry || typeof entry.name !== "string") continue;
    if (!variableDefinitions.includes(entry.name)) continue;

    const next = entry.value ?? "";
    if (currentMap.get(entry.name) !== next) {
      batch[entry.name] = next;
    }
  }

  return batch;
}

/**
 * Apply a "variables" message to a Companion plugin instance.
 *
 * Calls `instance.setVariableValues(batch)` AT MOST ONCE per server broadcast
 * (the fix for #265: previously the plugin made N separate calls — one per
 * changed variable — which caused N rounds of Companion feedback re-evaluation
 * per timer tick).
 *
 * Ordering: setVariableValues is called BEFORE the local cache is updated. If
 * setVariableValues throws, the cache stays clean, so the next message will
 * re-detect the change and retry.
 *
 * @param {{values?: unknown}} msg Parsed server "variables" message.
 * @param {string[]} variableDefinitions Whitelist of accepted variable names.
 * @param {{variables: Map<string,string>, setVariableValues: Function}} instance
 *   Plugin instance (or stub) exposing the cache map + Companion setter.
 * @returns {number} Number of setVariableValues calls actually made (0 or 1).
 */
function applyVariablesMessage(msg, variableDefinitions, instance) {
  const batch = computeVariableBatch(
    msg && msg.values,
    variableDefinitions,
    instance.variables,
  );
  const names = Object.keys(batch);
  if (names.length === 0) return 0;

  instance.setVariableValues(batch);
  for (const name of names) {
    instance.variables.set(name, batch[name]);
  }
  return 1;
}

/**
 * Drop cached values of variables that are no longer defined (#870).
 *
 * The cache is the diff base of `applyVariablesMessage`. When a variable
 * leaves the definitions (a slot past 11 after a plate was removed, a deleted
 * plate's `nameplate_<id>_*`), its last value must go too: otherwise, when the
 * variable is defined again later with the same value, the diff calls it
 * "unchanged" and Companion — which dropped the value with the definition —
 * is never sent it.
 *
 * @param {Map<string,string>} cache The plugin's variable value cache.
 * @param {string[]} variableDefinitions The variable ids defined now.
 * @returns {string[]} The ids whose cached value was dropped.
 */
function forgetUndefinedVariables(cache, variableDefinitions) {
  const defined = new Set(variableDefinitions);
  const dropped = [];
  for (const name of [...cache.keys()]) {
    if (!defined.has(name)) {
      cache.delete(name);
      dropped.push(name);
    }
  }
  return dropped;
}

module.exports = { computeVariableBatch, applyVariablesMessage, forgetUndefinedVariables };
