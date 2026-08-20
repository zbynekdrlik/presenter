const { test, describe } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("fs");
const path = require("path");

const {
  DEFAULT_OUTPUT,
  STREAM_COMMAND_IDS,
  isStreamCommand,
  streamActionOptions,
  buildStreamPayload,
} = require("./stream");

// --------------------------------------------------------------------------- //
// buildStreamPayload — the WIRE CONTRACT (arch #718 §7 / #711). Each assertion
// is the exact payload the server's parse_stream_command must accept.
// --------------------------------------------------------------------------- //
describe("buildStreamPayload — wire contract", () => {
  test("stream_scene_set sends { scene, output } with scene trimmed", () => {
    assert.deepEqual(
      buildStreamPayload("stream_scene_set", { scene: "  ytfast  ", output: "stream" }),
      { payload: { scene: "ytfast", output: "stream" } },
    );
  });

  test("output defaults to 'stream' when omitted or blank", () => {
    assert.deepEqual(buildStreamPayload("stream_scene_set", { scene: "chvaly" }), {
      payload: { scene: "chvaly", output: "stream" },
    });
    assert.deepEqual(buildStreamPayload("stream_scene_set", { scene: "chvaly", output: "   " }), {
      payload: { scene: "chvaly", output: "stream" },
    });
  });

  test("custom output slug passes through trimmed", () => {
    assert.deepEqual(
      buildStreamPayload("stream_overlay_on", { scene: "verse", output: " lower-third " }),
      { payload: { scene: "verse", output: "lower-third" } },
    );
  });

  test("overlay on/off/toggle all send { scene, output }", () => {
    for (const id of ["stream_overlay_on", "stream_overlay_off", "stream_overlay_toggle"]) {
      assert.deepEqual(
        buildStreamPayload(id, { scene: "verse" }),
        { payload: { scene: "verse", output: "stream" } },
        id,
      );
    }
  });

  test("stream_scene_clear / stream_clear send { output } only (no scene key)", () => {
    assert.deepEqual(buildStreamPayload("stream_scene_clear", {}), {
      payload: { output: "stream" },
    });
    assert.deepEqual(buildStreamPayload("stream_clear", { output: "wall" }), {
      payload: { output: "wall" },
    });
    // clear commands ignore any stray scene option
    assert.deepEqual(buildStreamPayload("stream_clear", { scene: "verse" }), {
      payload: { output: "stream" },
    });
  });

  test("scene-required commands reject empty/missing scene with an error (no send)", () => {
    for (const id of ["stream_scene_set", "stream_overlay_on", "stream_overlay_off", "stream_overlay_toggle"]) {
      const blank = buildStreamPayload(id, { scene: "   " });
      assert.ok(blank.error && !blank.payload, `${id} should error on blank scene`);
      const missing = buildStreamPayload(id, {});
      assert.ok(missing.error && !missing.payload, `${id} should error on missing scene`);
    }
  });

  test("never throws on undefined/null options (graceful-degrade contract)", () => {
    assert.doesNotThrow(() => buildStreamPayload("stream_clear"));
    assert.doesNotThrow(() => buildStreamPayload("stream_scene_set"));
    assert.doesNotThrow(() => buildStreamPayload("stream_scene_set", null));
  });

  test("non-stream command id returns an error, not a payload", () => {
    const r = buildStreamPayload("timer.start_countdown", {});
    assert.ok(r.error && !r.payload);
  });
});

// --------------------------------------------------------------------------- //
// streamActionOptions — Companion action option fields.
// --------------------------------------------------------------------------- //
describe("streamActionOptions — option fields", () => {
  test("scene-required commands expose 'scene' + 'output' inputs in order", () => {
    for (const id of ["stream_scene_set", "stream_overlay_on", "stream_overlay_off", "stream_overlay_toggle"]) {
      assert.deepEqual(streamActionOptions(id).map((o) => o.id), ["scene", "output"], id);
    }
  });

  test("clear commands expose only an 'output' input", () => {
    for (const id of ["stream_scene_clear", "stream_clear"]) {
      assert.deepEqual(streamActionOptions(id).map((o) => o.id), ["output"], id);
    }
  });

  test("output input defaults to 'stream'", () => {
    const out = streamActionOptions("stream_clear")[0];
    assert.equal(out.default, DEFAULT_OUTPUT);
    assert.equal(DEFAULT_OUTPUT, "stream");
  });

  test("non-stream command yields no options", () => {
    assert.deepEqual(streamActionOptions("timer.start_countdown"), []);
  });
});

describe("isStreamCommand", () => {
  test("true for every stream command id, false otherwise", () => {
    for (const id of STREAM_COMMAND_IDS) assert.ok(isStreamCommand(id), id);
    assert.ok(!isStreamCommand("stage.layout"));
    assert.ok(!isStreamCommand("stream")); // exact-list, not a prefix check
  });
});

// --------------------------------------------------------------------------- //
// Parity: index.js wires the commands + variables. Text-parsed from source so
// the test needs no @companion-module/base (mirrors commands.test.js).
// --------------------------------------------------------------------------- //
const indexSource = fs.readFileSync(path.resolve(__dirname, "..", "index.js"), "utf-8");

describe("index.js wires stream commands + variables", () => {
  const commandsBlock = indexSource.match(/const COMMANDS\s*=\s*\[([\s\S]*?)\];/);
  const varsBlock = indexSource.match(/const VARIABLE_DEFINITIONS\s*=\s*\[([\s\S]*?)\];/);

  test("COMMANDS contains all 6 stream command IDs", () => {
    assert.ok(commandsBlock, "Could not find COMMANDS array in index.js");
    for (const id of STREAM_COMMAND_IDS) {
      assert.ok(commandsBlock[1].includes(`"${id}"`), `COMMANDS missing ${id}`);
    }
  });

  test("VARIABLE_DEFINITIONS exposes stream_scene + stream_overlays", () => {
    assert.ok(varsBlock, "Could not find VARIABLE_DEFINITIONS array in index.js");
    assert.match(varsBlock[1], /"stream_scene"/);
    assert.match(varsBlock[1], /"stream_overlays"/);
  });

  test("every stream command id starts with 'stream_' (server delegation prefix)", () => {
    for (const id of STREAM_COMMAND_IDS) assert.ok(id.startsWith("stream_"), id);
  });
});
