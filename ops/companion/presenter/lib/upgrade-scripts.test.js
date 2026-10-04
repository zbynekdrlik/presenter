// #805: commit 956af894 (#249 / #270) renamed two action options without a
// Companion upgrade script, so actions saved before it silently misbehave:
//   - `timer.set_preach_limit`: `seconds` → `minutes` (legacy presses sent the
//     45-min default instead of the configured limit);
//   - `broadcast.set_live`: checkbox `enabled` → dropdown `state` "on"/"off"
//     (a legacy "Live ON" `{enabled: true}` button sent `enabled: false`).
// Each must keep its real behaviour — via an upgrade script that migrates the
// stored options, and a handler fallback for any config not yet upgraded.
//
// The real `index.js` adapter is loaded here with the Companion host runtime
// (`@companion-module/base`) and the socket library (`ws`) replaced by minimal
// stand-ins, so the test drives exactly what Companion drives: the upgrade
// scripts handed to `runEntrypoint`, and the registered action callback.
const { test, describe } = require("node:test");
const assert = require("node:assert/strict");
const Module = require("node:module");
const path = require("node:path");

const ACTION_ID = "timer.set_preach_limit";
const LIVE_ACTION_ID = "broadcast.set_live";
const WS_OPEN = 1;

function loadPresenterModule() {
  let entry = null;

  // Stand-in for the Companion host: records action definitions and swallows
  // logs, like the real InstanceBase does for a connection.
  class FakeInstanceBase {
    constructor(_internal) {
      this.actionDefinitions = {};
    }
    log() {}
    setActionDefinitions(defs) {
      this.actionDefinitions = defs;
    }
  }

  const fakeBase = {
    InstanceBase: FakeInstanceBase,
    InstanceStatus: {},
    runEntrypoint: (factory, upgradeScripts) => {
      entry = { factory, upgradeScripts };
    },
  };
  const fakeWs = { OPEN: WS_OPEN };

  const originalLoad = Module._load;
  Module._load = function (request, parent, isMain) {
    if (request === "@companion-module/base") return fakeBase;
    if (request === "ws") return fakeWs;
    return originalLoad.call(this, request, parent, isMain);
  };
  const indexPath = path.resolve(__dirname, "..", "index.js");
  try {
    delete require.cache[indexPath];
    require(indexPath);
  } finally {
    Module._load = originalLoad;
  }

  assert.ok(entry, "index.js must call runEntrypoint");
  return entry;
}

const presenter = loadPresenterModule();

function clone(value) {
  return JSON.parse(JSON.stringify(value));
}

// Mirror of @companion-module/base `runThroughUpgradeScripts`: a connection
// whose `lastUpgradeIndex` is N runs only the scripts after N, in order, on
// CLONED actions, and returned `updatedActions` are applied by id. Default -1
// = a connection that has never run a script.
function runUpgrades(actions, lastUpgradeIndex = -1) {
  const scripts = presenter.upgradeScripts;
  assert.ok(Array.isArray(scripts), "runEntrypoint needs an upgrade-scripts array");
  const byId = new Map(actions.map((a) => [a.id, clone(a)]));
  const changed = new Set();
  const results = [];
  for (const script of scripts.slice(lastUpgradeIndex + 1)) {
    const result = script(
      { currentConfig: {} },
      {
        config: null,
        secrets: null,
        actions: [...byId.values()].map(clone),
        feedbacks: [],
      },
    );
    results.push(result);
    for (const updated of result.updatedActions) {
      const current = byId.get(updated.id);
      current.actionId = updated.actionId;
      current.options = updated.options;
      changed.add(updated.id);
    }
  }
  return { actions: byId, changed, results };
}

function legacyAction(id, options, actionId = ACTION_ID) {
  return { id, controlId: `bank:${id}`, actionId, options };
}

function upgradedOptions(options, actionId = ACTION_ID) {
  const { actions, changed } = runUpgrades([legacyAction("a1", options, actionId)]);
  return { options: actions.get("a1").options, changed: changed.has("a1") };
}

// Press the button: the registered action callback, exactly as Companion runs it.
function pressPayload(options, actionId = ACTION_ID) {
  const instance = new presenter.factory({});
  const sent = [];
  instance.ws = {
    readyState: WS_OPEN,
    send: (raw) => sent.push(JSON.parse(raw)),
  };
  instance._setupActions();
  instance.actionDefinitions[actionId].callback({ options });
  assert.equal(sent.length, 1, "one command per press");
  assert.equal(sent[0].type, "command");
  assert.equal(sent[0].command, actionId);
  return sent[0].payload;
}

function livePressPayload(options) {
  return pressPayload(options, LIVE_ACTION_ID);
}

function upgradedLiveOptions(options) {
  return upgradedOptions(options, LIVE_ACTION_ID);
}

describe("#805 Companion upgrade script: legacy preach-limit seconds → minutes", () => {
  test("runEntrypoint receives a non-empty array of upgrade-script functions", () => {
    const scripts = presenter.upgradeScripts;
    assert.ok(Array.isArray(scripts), "upgrade scripts must be an array");
    assert.ok(scripts.length >= 1, "at least the #805 migration script");
    for (const script of scripts) {
      assert.equal(typeof script, "function");
    }
  });

  test("the rig buttons (420/300/3600/2700 s) migrate to 7/5/60/45 minutes, seconds dropped", () => {
    for (const [seconds, minutes] of [
      [420, 7],
      [300, 5],
      [3600, 60],
      [2700, 45],
    ]) {
      const { options, changed } = upgradedOptions({ seconds });
      assert.ok(changed, `seconds=${seconds} must be reported as updated`);
      assert.deepEqual(options, { minutes }, `seconds=${seconds}`);
    }
  });

  test("minutes are rounded to the nearest whole minute, never below 1", () => {
    assert.deepEqual(upgradedOptions({ seconds: 450 }).options, { minutes: 8 });
    assert.deepEqual(upgradedOptions({ seconds: 89 }).options, { minutes: 1 });
    assert.deepEqual(upgradedOptions({ seconds: 20 }).options, { minutes: 1 });
  });

  test("minutes follow the design formula round(seconds / 60) on the raw seconds", () => {
    // 89.5 s = 1.49 min → 1 (rounding seconds first would give 90 s → 2).
    assert.deepEqual(upgradedOptions({ seconds: 89.5 }).options, { minutes: 1 });
  });

  test("a numeric string seconds value migrates too", () => {
    assert.deepEqual(upgradedOptions({ seconds: "420" }).options, { minutes: 7 });
  });

  test("a cleared minutes value (empty / null) counts as unset — seconds migrates", () => {
    assert.deepEqual(upgradedOptions({ minutes: null, seconds: 300 }).options, {
      minutes: 5,
    });
    assert.deepEqual(upgradedOptions({ minutes: "", seconds: 300 }).options, {
      minutes: 5,
    });
  });

  test("unrelated option keys on a legacy action are preserved", () => {
    assert.deepEqual(upgradedOptions({ seconds: 300, note: "oznamy" }).options, {
      minutes: 5,
      note: "oznamy",
    });
  });

  test("an action that already has minutes is left untouched", () => {
    const { options, changed } = upgradedOptions({ minutes: 7, seconds: 3600 });
    assert.equal(changed, false);
    assert.deepEqual(options, { minutes: 7, seconds: 3600 });
  });

  test("an action with neither seconds nor minutes is left untouched", () => {
    const { options, changed } = upgradedOptions({});
    assert.equal(changed, false);
    assert.deepEqual(options, {});
  });

  test("an unusable legacy seconds value (non-numeric, zero, negative) is left untouched", () => {
    for (const seconds of ["abc", 0, -60, ""]) {
      const { options, changed } = upgradedOptions({ seconds });
      assert.equal(changed, false, `seconds=${JSON.stringify(seconds)}`);
      assert.deepEqual(options, { seconds });
    }
  });

  test("other actions that happen to carry a seconds option are not touched", () => {
    const { options, changed } = upgradedOptions(
      { seconds: 420 },
      "timer.set_countdown_target",
    );
    assert.equal(changed, false);
    assert.deepEqual(options, { seconds: 420 });
  });

  test("only actions are migrated — config and feedbacks are never reported changed", () => {
    const { results } = runUpgrades([legacyAction("a1", { seconds: 420 })]);
    for (const result of results) {
      assert.equal(result.updatedConfig, null);
      assert.deepEqual(result.updatedFeedbacks, []);
    }
  });

  test("a mixed bank: only the legacy preach-limit actions are reported changed", () => {
    const { actions, changed } = runUpgrades([
      legacyAction("legacy", { seconds: 300 }),
      legacyAction("modern", { minutes: 2 }),
      legacyAction("other", { code: "preach" }, "stage.layout"),
    ]);
    assert.deepEqual([...changed], ["legacy"]);
    assert.deepEqual(actions.get("legacy").options, { minutes: 5 });
    assert.deepEqual(actions.get("modern").options, { minutes: 2 });
    assert.deepEqual(actions.get("other").options, { code: "preach" });
  });

  test("an upgraded action pressed afterwards sends its real limit", () => {
    const { options } = upgradedOptions({ seconds: 420 });
    assert.deepEqual(pressPayload(options), { seconds: 420 });
  });
});

describe("#805 set_preach_limit handler: legacy seconds fallback", () => {
  test("a legacy action with only seconds sends that limit, not the 45-min default", () => {
    assert.deepEqual(pressPayload({ seconds: 420 }), { seconds: 420 });
    assert.deepEqual(pressPayload({ seconds: 3600 }), { seconds: 3600 });
  });

  test("a numeric string legacy seconds value is sent as an integer", () => {
    assert.deepEqual(pressPayload({ seconds: "300" }), { seconds: 300 });
  });

  test("a cleared minutes value (empty / null) does not mask the legacy seconds", () => {
    assert.deepEqual(pressPayload({ minutes: "", seconds: 420 }), { seconds: 420 });
    assert.deepEqual(pressPayload({ minutes: null, seconds: 420 }), { seconds: 420 });
  });

  test("the sent seconds is a whole number (the server parses u64)", () => {
    assert.deepEqual(pressPayload({ seconds: 419.6 }), { seconds: 420 });
  });

  test("minutes wins over a stale seconds value", () => {
    assert.deepEqual(pressPayload({ minutes: 7, seconds: 3600 }), { seconds: 420 });
  });

  test("minutes alone is still minutes × 60", () => {
    assert.deepEqual(pressPayload({ minutes: 2 }), { seconds: 120 });
  });

  test("with neither option the default stays 45 minutes", () => {
    assert.deepEqual(pressPayload({}), { seconds: 2700 });
  });

  test("an unusable legacy seconds value falls back to the 45-min default", () => {
    assert.deepEqual(pressPayload({ seconds: "abc" }), { seconds: 2700 });
    assert.deepEqual(pressPayload({ seconds: 0 }), { seconds: 2700 });
  });
});

describe("#805 Companion upgrade script: legacy live checkbox `enabled` → `state`", () => {
  test("a legacy Live ON button {enabled: true} migrates to state on, enabled dropped", () => {
    const { options, changed } = upgradedLiveOptions({ enabled: true });
    assert.ok(changed);
    assert.deepEqual(options, { state: "on" });
  });

  test("a legacy Live OFF button {enabled: false} migrates to state off", () => {
    const { options, changed } = upgradedLiveOptions({ enabled: false });
    assert.ok(changed);
    assert.deepEqual(options, { state: "off" });
  });

  test("legacy values are read exactly as the old handler did — Boolean(enabled)", () => {
    assert.deepEqual(upgradedLiveOptions({ enabled: "true" }).options, { state: "on" });
    assert.deepEqual(upgradedLiveOptions({ enabled: 1 }).options, { state: "on" });
    assert.deepEqual(upgradedLiveOptions({ enabled: 0 }).options, { state: "off" });
  });

  test("an action that already has a state is left untouched", () => {
    const { options, changed } = upgradedLiveOptions({ state: "off", enabled: true });
    assert.equal(changed, false);
    assert.deepEqual(options, { state: "off", enabled: true });
  });

  test("an action with neither enabled nor state is left untouched", () => {
    const { options, changed } = upgradedLiveOptions({});
    assert.equal(changed, false);
    assert.deepEqual(options, {});
  });

  test("other actions that happen to carry an enabled option are not touched", () => {
    const { options, changed } = upgradedOptions({ enabled: true }, "stage.layout");
    assert.equal(changed, false);
    assert.deepEqual(options, { enabled: true });
  });

  test("an upgraded Live ON button pressed afterwards sends enabled: true", () => {
    const { options } = upgradedLiveOptions({ enabled: true });
    assert.deepEqual(livePressPayload(options), { enabled: true });
  });
});

describe("#805 broadcast.set_live handler: legacy enabled fallback", () => {
  test("a legacy Live ON button {enabled: true} sends enabled: true", () => {
    assert.deepEqual(livePressPayload({ enabled: true }), { enabled: true });
  });

  test("a legacy Live OFF button {enabled: false} sends enabled: false", () => {
    assert.deepEqual(livePressPayload({ enabled: false }), { enabled: false });
  });

  test("state wins over a stale enabled value", () => {
    assert.deepEqual(livePressPayload({ state: "off", enabled: true }), {
      enabled: false,
    });
    assert.deepEqual(livePressPayload({ state: "on", enabled: false }), {
      enabled: true,
    });
  });

  test("state alone maps on → true, off → false", () => {
    assert.deepEqual(livePressPayload({ state: "on" }), { enabled: true });
    assert.deepEqual(livePressPayload({ state: "off" }), { enabled: false });
  });
});

describe("#805 upgrade scripts are append-only", () => {
  test("script 0 is the preach-limit migration (the first one shipped)", () => {
    const first = presenter.upgradeScripts[0];
    const result = first(
      { currentConfig: {} },
      {
        config: null,
        secrets: null,
        actions: [legacyAction("p", { seconds: 420 })],
        feedbacks: [],
      },
    );
    assert.deepEqual(
      result.updatedActions.map((a) => a.options),
      [{ minutes: 7 }],
    );
  });

  test("a connection that already ran script 0 still gets the live-state migration", () => {
    const { actions, changed } = runUpgrades(
      [legacyAction("live", { enabled: true }, LIVE_ACTION_ID)],
      0,
    );
    assert.ok(changed.has("live"));
    assert.deepEqual(actions.get("live").options, { state: "on" });
  });

  test("a fresh connection runs every script: both legacy shapes migrate", () => {
    const { actions, changed } = runUpgrades([
      legacyAction("preach", { seconds: 3600 }),
      legacyAction("live", { enabled: true }, LIVE_ACTION_ID),
    ]);
    assert.deepEqual([...changed].sort(), ["live", "preach"]);
    assert.deepEqual(actions.get("preach").options, { minutes: 60 });
    assert.deepEqual(actions.get("live").options, { state: "on" });
  });
});
