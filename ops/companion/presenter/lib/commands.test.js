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
