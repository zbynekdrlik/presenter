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
  NAMEPLATE_COMMAND_IDS,
  SONG_CHOICE_ID,
  isNameplateCommand,
  nameplateChoices,
  nameplateActionOptions,
  buildNameplateInvocation,
  isNameplateActive,
  nameplateVariableIds,
  nameplatePresets,
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

// --------------------------------------------------------------------------- //
// #779 — lower-third nameplates: pure logic.
// --------------------------------------------------------------------------- //
describe("nameplate pure logic (#779)", () => {
  const plates = [
    { id: 3, name: "Ján Novák", role: "pastor" },
    { id: 7, name: "Eva Malá", role: "" },
  ];

  test("the four nameplate command ids all start with 'stream_'", () => {
    assert.deepEqual(NAMEPLATE_COMMAND_IDS, [
      "stream_nameplate_show",
      "stream_nameplate_song",
      "stream_nameplate_toggle",
      "stream_nameplate_hide",
    ]);
    for (const id of NAMEPLATE_COMMAND_IDS) assert.ok(id.startsWith("stream_"), id);
    assert.ok(isNameplateCommand("stream_nameplate_show"));
    assert.ok(!isNameplateCommand("stream_scene_set"));
  });

  test("nameplateChoices builds string ids + labels, incl. the song entry", () => {
    const choices = nameplateChoices(plates, true);
    assert.deepEqual(choices[0], { id: "3", label: "Ján Novák — pastor" });
    assert.deepEqual(choices[1], { id: "7", label: "Eva Malá" }); // no role → name only
    assert.deepEqual(choices[2], { id: SONG_CHOICE_ID, label: "Pieseň (aktuálna)" });
    // Without song, and defensive against a non-array.
    assert.equal(nameplateChoices(plates, false).length, 2);
    assert.deepEqual(nameplateChoices(null, false), []);
  });

  test("nameplateActionOptions: show/toggle get a plate dropdown + output; song/hide only output", () => {
    for (const cmd of ["stream_nameplate_show", "stream_nameplate_toggle"]) {
      const opts = nameplateActionOptions(cmd, plates);
      assert.equal(opts.length, 2);
      assert.equal(opts[0].id, "plate");
      assert.equal(opts[0].type, "dropdown");
      assert.equal(opts[1].id, "output");
    }
    for (const cmd of ["stream_nameplate_song", "stream_nameplate_hide"]) {
      const opts = nameplateActionOptions(cmd, plates);
      assert.equal(opts.length, 1);
      assert.equal(opts[0].id, "output");
    }
  });

  test("buildNameplateInvocation maps person + song for show/toggle", () => {
    assert.deepEqual(
      buildNameplateInvocation("stream_nameplate_show", { plate: "3", output: "stream" }),
      { command: "stream_nameplate_show", payload: { id: 3, output: "stream" } },
    );
    // Show + song → the dedicated song command.
    assert.deepEqual(
      buildNameplateInvocation("stream_nameplate_show", { plate: SONG_CHOICE_ID }),
      { command: "stream_nameplate_song", payload: { output: "stream" } },
    );
    // Toggle + song → toggle with {song:true}.
    assert.deepEqual(
      buildNameplateInvocation("stream_nameplate_toggle", { plate: SONG_CHOICE_ID }),
      { command: "stream_nameplate_toggle", payload: { song: true, output: "stream" } },
    );
    assert.deepEqual(
      buildNameplateInvocation("stream_nameplate_toggle", { plate: "7", output: " lower-third " }),
      { command: "stream_nameplate_toggle", payload: { id: 7, output: "lower-third" } },
    );
    // Song + hide pass through.
    assert.deepEqual(buildNameplateInvocation("stream_nameplate_song", {}), {
      command: "stream_nameplate_song",
      payload: { output: "stream" },
    });
    assert.deepEqual(buildNameplateInvocation("stream_nameplate_hide", {}), {
      command: "stream_nameplate_hide",
      payload: { output: "stream" },
    });
  });

  test("buildNameplateInvocation errors on an unchosen/invalid plate", () => {
    assert.ok(buildNameplateInvocation("stream_nameplate_show", { plate: "" }).error);
    assert.ok(buildNameplateInvocation("stream_nameplate_show", {}).error);
    assert.ok(buildNameplateInvocation("stage.set", {}).error);
  });

  test("isNameplateActive matches an on-air id/song exactly, never idle/substring", () => {
    assert.ok(isNameplateActive("3", "3"));
    assert.ok(isNameplateActive("song", "song"));
    assert.ok(!isNameplateActive("-", "3")); // idle placeholder
    assert.ok(!isNameplateActive("", "3"));
    assert.ok(!isNameplateActive("33", "3")); // not substring
    assert.ok(!isNameplateActive("3", "")); // empty target
  });

  test("nameplateVariableIds = static ids + per-plate name/role", () => {
    const ids = nameplateVariableIds(plates);
    for (const id of [
      "nameplate_song_name",
      "nameplate_song_role",
      "nameplate_active_name",
      "nameplate_active_role",
      "nameplate_active_id",
      "nameplate_3_name",
      "nameplate_3_role",
      "nameplate_7_name",
      "nameplate_7_role",
    ]) {
      assert.ok(ids.includes(id), `missing ${id}`);
    }
  });

  test("nameplatePresets: one per plate + song, text = plate variables, toggle + active feedback", () => {
    const presets = nameplatePresets(plates);
    assert.ok(presets["nameplate_song"], "song preset present");
    assert.ok(presets["nameplate_3"], "per-plate preset present");
    const p = presets["nameplate_3"];
    assert.equal(p.type, "button");
    assert.match(p.style.text, /\$\(presenter:nameplate_3_name\)/);
    assert.match(p.style.text, /\$\(presenter:nameplate_3_role\)/);
    assert.equal(p.steps[0].down[0].actionId, "stream_nameplate_toggle");
    assert.equal(p.steps[0].down[0].options.plate, "3");
    assert.equal(p.feedbacks[0].feedbackId, "stream_nameplate_active");
    assert.equal(p.feedbacks[0].options.target, "3");
    // The song preset toggles the song entry.
    assert.equal(presets["nameplate_song"].steps[0].down[0].options.plate, SONG_CHOICE_ID);
  });
});

// --------------------------------------------------------------------------- //
// #779 — index.js wires the nameplate surface (text-parsed from source).
// --------------------------------------------------------------------------- //
describe("index.js wires nameplates (#779)", () => {
  test("COMMANDS contains all four nameplate command ids", () => {
    const commandsBlock = indexSource.match(/const COMMANDS\s*=\s*\[([\s\S]*?)\];/);
    assert.ok(commandsBlock);
    for (const id of NAMEPLATE_COMMAND_IDS) {
      assert.ok(commandsBlock[1].includes(`"${id}"`), `COMMANDS missing ${id}`);
    }
  });

  test("a nameplates message arm rebuilds defs/actions/feedbacks/presets", () => {
    assert.match(indexSource, /case "nameplates":/);
    assert.match(indexSource, /this\.nameplates\s*=/);
    assert.match(indexSource, /_setupPresets\(/);
  });

  test("presets are set from nameplatePresets + the active feedback is wired", () => {
    assert.match(indexSource, /setPresetDefinitions\(/);
    assert.match(indexSource, /nameplatePresets\(/);
    assert.match(indexSource, /feedbacks\[["']stream_nameplate_active["']\]/);
    assert.match(indexSource, /isNameplateActive\(/);
  });

  test("variable defs + allowlist are dynamic (_variableIds unions per-plate ids)", () => {
    assert.match(indexSource, /_variableIds\(/);
    assert.match(indexSource, /nameplateVariableIds\(/);
    // applyVariablesMessage no longer uses the static const directly.
    assert.match(indexSource, /applyVariablesMessage\(msg, this\._variableIds\(\), this\)/);
  });

  test("nameplate actions send via buildNameplateInvocation (command may remap)", () => {
    assert.match(indexSource, /buildNameplateInvocation\(/);
  });
});
