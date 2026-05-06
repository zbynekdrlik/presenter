const { test, describe } = require("node:test");
const assert = require("node:assert/strict");

// Extract COMMANDS from the main module source to keep the test independent
// of the Companion runtime (which requires @companion-module/base).
const fs = require("fs");
const path = require("path");

const indexSource = fs.readFileSync(
  path.resolve(__dirname, "..", "index.js"),
  "utf-8",
);

// Parse the COMMANDS array from the source text
const commandsMatch = indexSource.match(/const COMMANDS\s*=\s*\[([\s\S]*?)\];/);
assert.ok(commandsMatch, "Could not find COMMANDS array in index.js");

const idMatches = [...commandsMatch[1].matchAll(/id:\s*["']([^"']+)["']/g)];
const commandIds = idMatches.map((m) => m[1]);

describe("Companion COMMANDS parity", () => {
  const EXPECTED_COMMANDS = [
    "timer.start_countdown",
    "timer.pause_countdown",
    "timer.reset_countdown",
    "timer.set_countdown_target",
    "timer.start_preach",
    "timer.pause_preach",
    "timer.reset_preach",
    "timer.set_preach_limit",
    "timer.clear_preach_limit",
    "stage.layout",
    "stage.set",
    "bible.trigger",
    "bible.clear",
    "broadcast.set_live",
  ];

  test("COMMANDS array contains all expected command IDs", () => {
    for (const expected of EXPECTED_COMMANDS) {
      assert.ok(commandIds.includes(expected), `Missing command: ${expected}`);
    }
  });

  test("COMMANDS array has exactly the expected number of entries", () => {
    assert.equal(
      commandIds.length,
      EXPECTED_COMMANDS.length,
      `Expected ${EXPECTED_COMMANDS.length} commands but found ${commandIds.length}: ${JSON.stringify(commandIds)}`,
    );
  });

  test("no duplicate command IDs", () => {
    const seen = new Set();
    for (const id of commandIds) {
      assert.ok(!seen.has(id), `Duplicate command ID: ${id}`);
      seen.add(id);
    }
  });
});

describe("Companion action UX (#270 #249)", () => {
  test("timer.set_preach_limit input uses 'minutes' field with default 45", () => {
    // The action options switch case for timer.set_preach_limit must
    // expose a 'minutes' input with default 45 (not 'seconds' / 2700).
    const preachOptionsRegion = indexSource.match(
      /case ["']timer\.set_preach_limit["']:\s*return\s*\[([\s\S]*?)\];/,
    );
    assert.ok(
      preachOptionsRegion,
      "Could not find timer.set_preach_limit options block",
    );
    const optionsText = preachOptionsRegion[1];
    assert.match(
      optionsText,
      /id:\s*["']minutes["']/,
      "preach-limit input id should be 'minutes'",
    );
    assert.match(
      optionsText,
      /label:\s*["']Limit \(minutes\)["']/,
      "preach-limit label should say (minutes)",
    );
    assert.match(
      optionsText,
      /default:\s*45\b/,
      "preach-limit default should be 45",
    );
  });

  test("timer.set_preach_limit handler multiplies minutes by 60", () => {
    const handlerRegion = indexSource.match(
      /case ["']timer\.set_preach_limit["']:\s*\{([\s\S]*?)\}/,
    );
    assert.ok(handlerRegion, "Could not find timer.set_preach_limit handler");
    const handlerText = handlerRegion[1];
    assert.match(
      handlerText,
      /options\.minutes/,
      "handler should read options.minutes",
    );
    assert.match(
      handlerText,
      /\*\s*60\b/,
      "handler should multiply by 60 to convert minutes → seconds",
    );
  });

  test("broadcast.set_live input is a dropdown with 'state' field", () => {
    const liveOptionsRegion = indexSource.match(
      /case ["']broadcast\.set_live["']:\s*return\s*\[([\s\S]*?)\];/,
    );
    assert.ok(
      liveOptionsRegion,
      "Could not find broadcast.set_live options block",
    );
    const optionsText = liveOptionsRegion[1];
    assert.match(
      optionsText,
      /type:\s*["']dropdown["']/,
      "broadcast.set_live input should be a dropdown",
    );
    assert.match(
      optionsText,
      /id:\s*["']state["']/,
      "broadcast.set_live id should be 'state'",
    );
    assert.match(
      optionsText,
      /id:\s*["']on["']/,
      "dropdown should have an 'on' choice",
    );
    assert.match(
      optionsText,
      /id:\s*["']off["']/,
      "dropdown should have an 'off' choice",
    );
  });

  test("broadcast.set_live handler maps state==='on' to enabled boolean", () => {
    const handlerRegion = indexSource.match(
      /case ["']broadcast\.set_live["']:\s*\{([\s\S]*?)\}/,
    );
    assert.ok(handlerRegion, "Could not find broadcast.set_live handler");
    const handlerText = handlerRegion[1];
    assert.match(
      handlerText,
      /options\.state\s*===\s*["']on["']/,
      "handler should compare options.state === 'on'",
    );
  });

  test("action label for timer.set_preach_limit says (minutes)", () => {
    // The action registration entry in COMMANDS must show "(minutes)" so
    // the operator sees the unit when picking the action.
    assert.match(
      indexSource,
      /id:\s*["']timer\.set_preach_limit["'],\s*label:\s*["'][^"']*\(minutes\)/,
      "timer.set_preach_limit label should contain '(minutes)'",
    );
  });
});
