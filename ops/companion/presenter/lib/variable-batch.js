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

module.exports = { computeVariableBatch };
