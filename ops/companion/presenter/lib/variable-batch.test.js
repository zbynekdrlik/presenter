const { test, describe } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("fs");
const path = require("path");

const {
  computeVariableBatch,
  applyVariablesMessage,
} = require("./variable-batch");

const indexSource = fs.readFileSync(
  path.resolve(__dirname, "..", "index.js"),
  "utf-8",
);

const VARS = [
  "timer_countdown_state",
  "timer_countdown_remaining_seconds",
  "timer_countdown_remaining_hms",
  "timer_countdown_remaining_mmss",
  "timer_countdown_remaining_hhmm",
  "timer_countdown_remaining_readable",
  "song_name",
];

function makeStub() {
  const stub = {
    variables: new Map(),
    setVariableValuesCalls: [],
    setVariableValues(batch) {
      this.setVariableValuesCalls.push({ ...batch });
    },
  };
  return stub;
}

describe("computeVariableBatch", () => {
  test("returns only changed known variables", () => {
    const current = new Map([
      ["timer_countdown_state", "running"],
      ["song_name", "Hymn"],
    ]);
    const values = [
      { name: "timer_countdown_state", value: "running" },
      { name: "timer_countdown_remaining_seconds", value: "12" },
      { name: "song_name", value: "Doxology" },
      { name: "unknown_variable", value: "drop" },
    ];

    const batch = computeVariableBatch(values, VARS, current);

    assert.deepEqual(batch, {
      timer_countdown_remaining_seconds: "12",
      song_name: "Doxology",
    });
  });

  test("includes first-seen variables (undefined → value transition)", () => {
    const current = new Map();
    const values = [
      { name: "song_name", value: "Hymn" },
      { name: "timer_countdown_state", value: "paused" },
    ];

    const batch = computeVariableBatch(values, VARS, current);

    assert.deepEqual(batch, {
      song_name: "Hymn",
      timer_countdown_state: "paused",
    });
  });

  test("treats null/undefined value as empty string", () => {
    const current = new Map([["song_name", "Hymn"]]);
    const values = [
      { name: "song_name", value: null },
      { name: "timer_countdown_state", value: undefined },
    ];

    const batch = computeVariableBatch(values, VARS, current);

    assert.equal(batch.song_name, "");
    assert.equal(batch.timer_countdown_state, "");
  });

  test("skips entries with non-string names", () => {
    const current = new Map();
    const values = [
      { name: null, value: "x" },
      { value: "y" },
      "not-an-object",
      null,
      { name: "song_name", value: "OK" },
    ];

    const batch = computeVariableBatch(values, VARS, current);

    assert.deepEqual(batch, { song_name: "OK" });
  });

  test("returns empty object when nothing changes", () => {
    const current = new Map([
      ["song_name", "Hymn"],
      ["timer_countdown_state", "paused"],
    ]);
    const values = [
      { name: "song_name", value: "Hymn" },
      { name: "timer_countdown_state", value: "paused" },
    ];

    const batch = computeVariableBatch(values, VARS, current);

    assert.deepEqual(batch, {});
  });
});

describe("applyVariablesMessage (behavior, #265)", () => {
  test("collapses a full timer-tick payload into ONE setVariableValues call", () => {
    const instance = makeStub();
    const msg = {
      type: "variables",
      values: [
        { name: "timer_countdown_state", value: "running" },
        { name: "timer_countdown_remaining_seconds", value: "299" },
        { name: "timer_countdown_remaining_hms", value: "00:04:59" },
        { name: "timer_countdown_remaining_mmss", value: "04:59" },
        { name: "timer_countdown_remaining_hhmm", value: "00:04" },
        { name: "timer_countdown_remaining_readable", value: "4m 59s" },
      ],
    };

    const calls = applyVariablesMessage(msg, VARS, instance);

    assert.equal(calls, 1, "exactly one setVariableValues call");
    assert.equal(instance.setVariableValuesCalls.length, 1);
    assert.equal(
      Object.keys(instance.setVariableValuesCalls[0]).length,
      6,
      "all 6 timer vars in the single call",
    );
  });

  test("skips setVariableValues entirely when nothing changed", () => {
    const instance = makeStub();
    instance.variables.set("song_name", "Hymn");

    const calls = applyVariablesMessage(
      { values: [{ name: "song_name", value: "Hymn" }] },
      VARS,
      instance,
    );

    assert.equal(calls, 0);
    assert.equal(instance.setVariableValuesCalls.length, 0);
  });

  test("updates local cache for every batched variable", () => {
    const instance = makeStub();
    applyVariablesMessage(
      {
        values: [
          { name: "song_name", value: "Doxology" },
          { name: "timer_countdown_state", value: "running" },
        ],
      },
      VARS,
      instance,
    );

    assert.equal(instance.variables.get("song_name"), "Doxology");
    assert.equal(instance.variables.get("timer_countdown_state"), "running");
  });

  test("on setVariableValues throw, cache stays clean for retry", () => {
    const instance = {
      variables: new Map(),
      setVariableValues() {
        throw new Error("simulated Companion failure");
      },
    };

    assert.throws(() =>
      applyVariablesMessage(
        { values: [{ name: "song_name", value: "Hymn" }] },
        VARS,
        instance,
      ),
    );
    assert.equal(
      instance.variables.has("song_name"),
      false,
      "cache must NOT be written when setVariableValues throws",
    );
  });

  test("tolerates malformed messages (no values, wrong shape)", () => {
    const instance = makeStub();
    assert.equal(applyVariablesMessage({}, VARS, instance), 0);
    assert.equal(applyVariablesMessage({ values: null }, VARS, instance), 0);
    assert.equal(
      applyVariablesMessage({ values: "not-array" }, VARS, instance),
      0,
    );
    assert.equal(instance.setVariableValuesCalls.length, 0);
  });

  test("filters unknown variables before counting changes", () => {
    const instance = makeStub();
    const calls = applyVariablesMessage(
      {
        values: [
          { name: "unknown_extra", value: "x" },
          { name: "song_name", value: "Hymn" },
        ],
      },
      VARS,
      instance,
    );

    assert.equal(calls, 1);
    assert.deepEqual(instance.setVariableValuesCalls[0], {
      song_name: "Hymn",
    });
    assert.equal(instance.variables.has("unknown_extra"), false);
  });
});

describe("index.js wires applyVariablesMessage (regression for #265)", () => {
  test("variables case delegates to applyVariablesMessage and contains NO inline setVariableValues call", () => {
    const m = indexSource.match(
      /case "variables":\s*([\s\S]*?)\n\s+(?:case "|default:|\})/,
    );
    assert.ok(m, "could not locate the 'variables' case body in index.js");
    const body = m[1];

    assert.ok(
      /applyVariablesMessage\s*\(/.test(body),
      "variables case must call applyVariablesMessage(...)",
    );

    const inlineCalls = body.match(/this\.setVariableValues\s*\(/g) || [];
    assert.equal(
      inlineCalls.length,
      0,
      `variables case must not call this.setVariableValues directly (found ${inlineCalls.length}); use applyVariablesMessage instead (#265)`,
    );

    assert.ok(
      !/forEach[\s\S]*_updateVariable/.test(body),
      "values.forEach calling _updateVariable produces N setVariableValues calls — must batch via applyVariablesMessage (regression for #265)",
    );
  });
});
