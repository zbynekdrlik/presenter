const { test, describe } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("fs");
const path = require("path");

const { computeVariableBatch } = require("./variable-batch");

const indexSource = fs.readFileSync(
  path.resolve(__dirname, "..", "index.js"),
  "utf-8",
);

describe("computeVariableBatch", () => {
  const VARS = [
    "timer_countdown_state",
    "timer_countdown_remaining_seconds",
    "timer_countdown_remaining_hms",
    "timer_countdown_remaining_mmss",
    "timer_countdown_remaining_hhmm",
    "timer_countdown_remaining_readable",
    "song_name",
  ];

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

  test("collapses a full timer-tick payload into a single batch (#265)", () => {
    const current = new Map();
    const values = [
      { name: "timer_countdown_state", value: "running" },
      { name: "timer_countdown_remaining_seconds", value: "299" },
      { name: "timer_countdown_remaining_hms", value: "00:04:59" },
      { name: "timer_countdown_remaining_mmss", value: "04:59" },
      { name: "timer_countdown_remaining_hhmm", value: "00:04" },
      { name: "timer_countdown_remaining_readable", value: "4m 59s" },
    ];

    const batch = computeVariableBatch(values, VARS, current);

    assert.equal(
      Object.keys(batch).length,
      6,
      "all 6 changed timer variables must be in ONE batch object",
    );
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

describe("index.js variables-case regression (#265)", () => {
  test("must not call _updateVariable inside forEach in the variables case", () => {
    const m = indexSource.match(/case "variables":\s*([\s\S]*?)\n\s+case "/);
    assert.ok(m, "could not locate the 'variables' case body in index.js");
    const body = m[1];

    assert.ok(
      !/forEach[\s\S]*_updateVariable/.test(body),
      "values.forEach calling _updateVariable produces N setVariableValues calls — must batch instead (regression for #265)",
    );
  });

  test("variables case must use computeVariableBatch + single setVariableValues call", () => {
    const m = indexSource.match(/case "variables":\s*([\s\S]*?)\n\s+case "/);
    assert.ok(m, "could not locate the 'variables' case body in index.js");
    const body = m[1];

    assert.ok(
      /computeVariableBatch/.test(body),
      "variables case must use computeVariableBatch helper from lib/variable-batch",
    );

    const calls = body.match(/setVariableValues/g) || [];
    assert.ok(
      calls.length <= 1,
      `variables case must call setVariableValues at most once per message (found ${calls.length})`,
    );
  });
});
