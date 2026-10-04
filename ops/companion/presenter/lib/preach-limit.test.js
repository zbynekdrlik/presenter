// #805: legacy `timer.set_preach_limit` actions (saved before #249 renamed the
// option `seconds` → `minutes`) must keep their real limit — via a Companion
// upgrade script that migrates the stored options, and a handler fallback for
// any config the upgrade has not reached.
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

// Mirror of @companion-module/base `runThroughUpgradeScripts` for a connection
// that has never run a script (lastUpgradeIndex -1): every script runs in
// order on CLONED actions, and returned `updatedActions` are applied by id.
function runUpgrades(actions) {
  const scripts = presenter.upgradeScripts;
  assert.ok(Array.isArray(scripts), "runEntrypoint needs an upgrade-scripts array");
  const byId = new Map(actions.map((a) => [a.id, clone(a)]));
  const changed = new Set();
  const results = [];
  for (const script of scripts) {
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
function pressPayload(options) {
  const instance = new presenter.factory({});
  const sent = [];
  instance.ws = {
    readyState: WS_OPEN,
    send: (raw) => sent.push(JSON.parse(raw)),
  };
  instance._setupActions();
  instance.actionDefinitions[ACTION_ID].callback({ options });
  assert.equal(sent.length, 1, "one command per press");
  assert.equal(sent[0].type, "command");
  assert.equal(sent[0].command, ACTION_ID);
  return sent[0].payload;
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

  test("a numeric string seconds value migrates too", () => {
    assert.deepEqual(upgradedOptions({ seconds: "420" }).options, { minutes: 7 });
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
