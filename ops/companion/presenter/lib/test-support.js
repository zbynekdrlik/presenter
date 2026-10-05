"use strict";

// Test-only helper (not a `*.test.js` file, so `node --test` never runs it
// directly). Loads the REAL `index.js` adapter with the Companion host runtime
// (`@companion-module/base`) and the socket library (`ws`) replaced by minimal
// stand-ins, so a test drives exactly what Companion drives: the upgrade
// scripts handed to `runEntrypoint`, the registered action / feedback
// definitions, and `_handleMessage` (#805 idiom,
// `.claude/rules/companion-upgrade-scripts.md`). Base is NOT installed for
// `npm run test:companion`, so it must be stubbed; `ws` is stubbed too so no
// real socket is ever involved.

const assert = require("node:assert/strict");
const Module = require("node:module");
const path = require("node:path");

const WS_OPEN = 1;

// Stand-in for the Companion host: records every definition the module
// registers (and how often), swallows logs.
class FakeInstanceBase {
  constructor(_internal) {
    this.actionDefinitions = {};
    this.feedbackDefinitions = {};
    this.presetDefinitions = {};
    this.variableDefinitions = [];
    this.calls = {
      setActionDefinitions: 0,
      setFeedbackDefinitions: 0,
      checkFeedbacks: 0,
    };
  }
  log() {}
  updateStatus() {}
  setActionDefinitions(defs) {
    this.actionDefinitions = defs;
    this.calls.setActionDefinitions += 1;
  }
  setFeedbackDefinitions(defs) {
    this.feedbackDefinitions = defs;
    this.calls.setFeedbackDefinitions += 1;
  }
  setPresetDefinitions(defs) {
    this.presetDefinitions = defs;
  }
  setVariableDefinitions(defs) {
    this.variableDefinitions = defs;
  }
  setVariableValues() {}
  checkFeedbacks() {
    this.calls.checkFeedbacks += 1;
  }
}

/**
 * Require the real `index.js` under the stubs and return what it handed to
 * `runEntrypoint`: `{ factory, upgradeScripts }` (`factory` = the instance
 * class). The `Module._load` hook is restored in `finally`.
 */
function loadPresenterModule() {
  let entry = null;
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

module.exports = { WS_OPEN, loadPresenterModule };
