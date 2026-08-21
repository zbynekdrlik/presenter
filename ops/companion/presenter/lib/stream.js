"use strict";

/**
 * Stream-graphics Companion command family (issue #712, EPIC #718).
 *
 * The plugin sends `{ type: "command", command: "stream_*", payload }` over
 * `/companion/ws`; the server (issue #711 / arch comment #718 §7) parses any
 * command whose NAME starts with `stream_`. Scenes are addressed BY NAME
 * (the server matches case-insensitively); `output` is an optional slug that
 * defaults to `"stream"` (the seeded default output).
 *
 * Wire contract (arch #718 §7 — this is FIXED, do not re-litigate):
 *   stream_scene_set        { scene, output? }   exclusive base activate
 *   stream_scene_clear      { output? }          base → none (transparent)
 *   stream_overlay_on       { scene, output? }   overlay on
 *   stream_overlay_off      { scene, output? }   overlay off
 *   stream_overlay_toggle   { scene, output? }   overlay toggle
 *   stream_clear            { output? }          base clear + ALL overlays off
 *
 * Graceful degrade: an older server that predates #711 replies with a
 * non-fatal `{ type: "error", message: "unknown command: stream_*" }`, which
 * the plugin logs via its existing `case "error"` handler in `_handleMessage`
 * — no crash, no disconnect. `buildStreamPayload` therefore never throws.
 *
 * This module has NO dependency on `@companion-module/base`, so it is unit
 * testable with `node --test` alone (same idiom as `lib/time.js` and
 * `lib/variable-batch.js`); `index.js` is a thin adapter that delegates here.
 */

const DEFAULT_OUTPUT = "stream";

// Companion action IDs === the wire command names (all start with "stream_",
// which is exactly the prefix the server's parse_command delegation keys on).
const STREAM_COMMAND_IDS = [
  "stream_scene_set",
  "stream_scene_clear",
  "stream_overlay_on",
  "stream_overlay_off",
  "stream_overlay_toggle",
  "stream_clear",
];

// Commands that require a scene NAME (base activate + overlay on/off/toggle).
// The clear commands (stream_scene_clear, stream_clear) carry no scene.
const SCENE_REQUIRED = new Set([
  "stream_scene_set",
  "stream_overlay_on",
  "stream_overlay_off",
  "stream_overlay_toggle",
]);

function isStreamCommand(commandId) {
  return STREAM_COMMAND_IDS.includes(commandId);
}

function outputOption() {
  return {
    type: "textinput",
    id: "output",
    label: "Output slug (default: stream)",
    default: DEFAULT_OUTPUT,
    placeholder: DEFAULT_OUTPUT,
  };
}

function sceneOption() {
  return {
    type: "textinput",
    id: "scene",
    label: "Scene name (matched case-insensitively)",
    default: "",
  };
}

/**
 * Companion action option fields for a stream command.
 * Scene-required commands get a `scene` + `output` input; the clear commands
 * get only an `output` input.
 *
 * @param {string} commandId
 * @returns {Array<object>} option-field definitions (empty for non-stream ids)
 */
function streamActionOptions(commandId) {
  if (!isStreamCommand(commandId)) return [];
  if (SCENE_REQUIRED.has(commandId)) {
    return [sceneOption(), outputOption()];
  }
  return [outputOption()];
}

/**
 * Build the wire payload for a stream command from Companion action options.
 *
 * Returns `{ payload }` on success, or `{ error }` when a required scene is
 * empty/missing (the caller logs the error and does NOT send — same guard
 * idiom as `timer.set_countdown_target`). Never throws.
 *
 * `output` defaults to `"stream"` when omitted or blank; `scene` and `output`
 * are trimmed so a stray leading/trailing space never breaks the server-side
 * name match.
 *
 * @param {string} commandId
 * @param {object} [options] Companion action options.
 * @returns {{payload: object} | {error: string}}
 */
function buildStreamPayload(commandId, options) {
  if (!isStreamCommand(commandId)) {
    return { error: `not a stream command: ${commandId}` };
  }
  const opts = options || {};
  const rawOutput = typeof opts.output === "string" ? opts.output.trim() : "";
  const output = rawOutput !== "" ? rawOutput : DEFAULT_OUTPUT;

  if (SCENE_REQUIRED.has(commandId)) {
    const scene = typeof opts.scene === "string" ? opts.scene.trim() : "";
    if (scene === "") {
      return { error: `${commandId}: scene name is required` };
    }
    return { payload: { scene, output } };
  }
  return { payload: { output } };
}

module.exports = {
  DEFAULT_OUTPUT,
  STREAM_COMMAND_IDS,
  isStreamCommand,
  streamActionOptions,
  buildStreamPayload,
};
