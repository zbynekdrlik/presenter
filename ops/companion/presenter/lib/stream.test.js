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
  parseOverlayList,
  isOverlayActive,
  isSceneActive,
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
// #780 — pure evaluators for the stream_overlay_active / stream_scene_active
// boolean feedbacks. `stream_overlays` is a comma-joined list of active overlay
// NAMES (server: `overlay_names.join(", ")`), `"-"` when none; `stream_scene`
// is a single base-scene name, `"-"` when none. Membership + equality are
// case-insensitive and trimmed. Substring names must NOT match.
// --------------------------------------------------------------------------- //
describe("parseOverlayList — split the joined overlay list", () => {
  test("splits a comma-joined list, trims each entry", () => {
    assert.deepEqual(parseOverlayList("verse, ucet dole"), ["verse", "ucet dole"]);
  });

  test("the '-' placeholder yields an empty list", () => {
    assert.deepEqual(parseOverlayList("-"), []);
    assert.deepEqual(parseOverlayList(" - "), []);
  });

  test("empty / whitespace / non-string yields an empty list", () => {
    assert.deepEqual(parseOverlayList(""), []);
    assert.deepEqual(parseOverlayList("   "), []);
    assert.deepEqual(parseOverlayList(undefined), []);
    assert.deepEqual(parseOverlayList(null), []);
  });

  test("drops empty segments from stray commas", () => {
    assert.deepEqual(parseOverlayList("verse, , chvaly,"), ["verse", "chvaly"]);
  });
});

describe("isOverlayActive — case-insensitive membership", () => {
  test("true when the overlay is in the active list (two overlays active)", () => {
    assert.ok(isOverlayActive("verse, ucet dole", "verse"));
    assert.ok(isOverlayActive("verse, ucet dole", "ucet dole"));
  });

  test("case-insensitive + trims the queried name", () => {
    assert.ok(isOverlayActive("Verse, Ucet Dole", "verse"));
    assert.ok(isOverlayActive("verse", "  VERSE  "));
  });

  test("false when not present, or list empty / placeholder", () => {
    assert.ok(!isOverlayActive("chvaly, ucet dole", "verse"));
    assert.ok(!isOverlayActive("-", "verse"));
    assert.ok(!isOverlayActive("", "verse"));
  });

  test("substring name must NOT match (exact membership, not contains)", () => {
    assert.ok(!isOverlayActive("verses", "verse"));
    assert.ok(!isOverlayActive("verse intro, chvaly", "verse"));
  });

  test("empty / non-string queried name is never active", () => {
    assert.ok(!isOverlayActive("verse", ""));
    assert.ok(!isOverlayActive("verse", "   "));
    assert.ok(!isOverlayActive("verse", undefined));
  });
});

describe("isSceneActive — case-insensitive base-scene equality", () => {
  test("true only on exact (trimmed, case-insensitive) match", () => {
    assert.ok(isSceneActive("ytfast", "ytfast"));
    assert.ok(isSceneActive("YtFast", "ytfast"));
    assert.ok(isSceneActive("  ytfast  ", "ytfast"));
  });

  test("false on mismatch, placeholder, empty, or substring", () => {
    assert.ok(!isSceneActive("ytfast", "chvaly"));
    assert.ok(!isSceneActive("-", "ytfast"));
    assert.ok(!isSceneActive("", "ytfast"));
    assert.ok(!isSceneActive("ytfast-live", "ytfast")); // substring must not match
  });

  test("empty / non-string queried name is never active", () => {
    assert.ok(!isSceneActive("ytfast", ""));
    assert.ok(!isSceneActive("ytfast", undefined));
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

// --------------------------------------------------------------------------- //
// #780 — feedback wiring in index.js. Text-parsed from source (no runtime).
// The bug: text_* feedbacks were `type:"advanced"` returning a boolean (ignored
// by Companion), and checkFeedbacks was never called so nothing re-evaluated.
// --------------------------------------------------------------------------- //
describe("index.js feedback wiring (#780)", () => {
  test("no feedback is declared type: \"advanced\" (a boolean callback needs a boolean feedback)", () => {
    assert.ok(
      !/type:\s*["']advanced["']/.test(indexSource),
      "advanced feedbacks returning a boolean must be type:\"boolean\" (#780)",
    );
  });

  test("the generic text_<name> feedback is boolean with a defaultStyle", () => {
    const block = indexSource.match(/feedbacks\[`text_\$\{name\}`\]\s*=\s*{[\s\S]*?};/);
    assert.ok(block, "could not find the generic text_<name> feedback block");
    assert.match(block[0], /type:\s*["']boolean["']/);
    assert.match(block[0], /defaultStyle/);
  });

  test("checkFeedbacks() is re-evaluated on the variables path AND on connection-state change", () => {
    const calls = indexSource.match(/checkFeedbacks\(/g) || [];
    assert.ok(
      calls.length >= 2,
      `expected checkFeedbacks() on both the variables path and status change, found ${calls.length}`,
    );
  });

  test("the variables message arm re-checks feedbacks when a value changed", () => {
    const armStart = indexSource.indexOf('case "variables":');
    const armEnd = indexSource.indexOf('case "ack":');
    assert.ok(armStart !== -1 && armEnd > armStart, "could not isolate the variables arm");
    const arm = indexSource.slice(armStart, armEnd);
    assert.match(arm, /applyVariablesMessage/);
    assert.match(arm, /checkFeedbacks/);
  });

  test("dedicated stream_overlay_active + stream_scene_active boolean feedbacks exist", () => {
    assert.match(indexSource, /feedbacks\[["']stream_overlay_active["']\]/);
    assert.match(indexSource, /feedbacks\[["']stream_scene_active["']\]/);
  });

  test("the new stream feedbacks use the lib/stream evaluators", () => {
    assert.match(indexSource, /isOverlayActive\(/);
    assert.match(indexSource, /isSceneActive\(/);
  });
});
