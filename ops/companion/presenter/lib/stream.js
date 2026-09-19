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

// --------------------------------------------------------------------------- //
// Feedback evaluators (issue #780).
//
// The server publishes two stream variables (`crates/presenter-server/src/
// companion/stream.rs`): `stream_overlays` is a comma-joined list of the ACTIVE
// overlay NAMES (`overlay_names.join(", ")`), and `stream_scene` is the single
// active base-scene NAME; both are the placeholder `"-"` when nothing is active.
//
// Scenes/overlays are addressed BY NAME and matched case-insensitively (the same
// contract the actions use server-side), so the feedback evaluators mirror that:
// trim + lowercase, exact membership/equality (NOT substring — `verse` must not
// light for an overlay named `verses`). Pure + dependency-free, unit-tested with
// `node --test` (`lib/stream.test.js`); `index.js` is a thin adapter.
// --------------------------------------------------------------------------- //

const PLACEHOLDER = "-";

/**
 * Split the server's comma-joined `stream_overlays` value into trimmed active
 * overlay names. Empty segments and the `"-"` placeholder are dropped. A
 * non-string input yields an empty list.
 *
 * @param {unknown} value The `stream_overlays` variable value.
 * @returns {string[]} Active overlay names (never null/undefined entries).
 */
function parseOverlayList(value) {
  if (typeof value !== "string") return [];
  return value
    .split(",")
    .map((entry) => entry.trim())
    .filter((entry) => entry !== "" && entry !== PLACEHOLDER);
}

/**
 * True when `name` is an active overlay (case-insensitive, trimmed, exact
 * membership) in the joined `stream_overlays` value. An empty/non-string name
 * is never active.
 *
 * @param {unknown} value The `stream_overlays` variable value.
 * @param {unknown} name The overlay scene name to test.
 * @returns {boolean}
 */
function isOverlayActive(value, name) {
  if (typeof name !== "string") return false;
  const target = name.trim().toLowerCase();
  if (target === "") return false;
  return parseOverlayList(value).some((entry) => entry.toLowerCase() === target);
}

/**
 * True when `name` is the active base scene (case-insensitive, trimmed, exact
 * equality) in the `stream_scene` value. The `"-"` placeholder, an empty value,
 * and an empty/non-string name are never active. Substring names do not match.
 *
 * @param {unknown} value The `stream_scene` variable value.
 * @param {unknown} name The base scene name to test.
 * @returns {boolean}
 */
function isSceneActive(value, name) {
  if (typeof name !== "string") return false;
  const target = name.trim().toLowerCase();
  if (target === "") return false;
  const current = typeof value === "string" ? value.trim().toLowerCase() : "";
  if (current === "" || current === PLACEHOLDER) return false;
  return current === target;
}

// --------------------------------------------------------------------------- //
// Lower-third nameplates (issue #779).
//
// The server sends a `nameplates` message ({output, plates:[{id,name,role}]}) on
// connect and on every list change; the plugin keeps that list in module state
// and derives from it: the show/toggle action DROPDOWN choices (incl. a virtual
// "Pieseň"/song entry), the DYNAMIC per-plate variable definitions
// (`nameplate_<id>_name`/`_role`), the boolean feedback `stream_nameplate_active`
// (lit while a plate is on air, matched against `nameplate_active_id`), and the
// PRESETS whose button text is the plate's variables. All pure + dependency-free
// so `lib/stream.test.js` covers them with `node --test`; `index.js` wires them.
// --------------------------------------------------------------------------- //

// The four nameplate command NAMES (all `stream_`-prefixed, so the server's
// parse_command delegation routes them to companion/stream.rs).
const NAMEPLATE_COMMAND_IDS = [
  "stream_nameplate_show",
  "stream_nameplate_song",
  "stream_nameplate_toggle",
  "stream_nameplate_hide",
];

// The dropdown value for the virtual song plate (also the `nameplate_active_id`
// value the server publishes while a song plate is on air).
const SONG_CHOICE_ID = "song";

// Static (non-per-plate) nameplate variable ids the plugin always defines.
const NAMEPLATE_STATIC_VARIABLE_IDS = [
  "nameplate_song_name",
  "nameplate_song_role",
  "nameplate_active_name",
  "nameplate_active_role",
  "nameplate_active_id",
];

function isNameplateCommand(commandId) {
  return NAMEPLATE_COMMAND_IDS.includes(commandId);
}

function resolveOutput(output) {
  const trimmed = typeof output === "string" ? output.trim() : "";
  return trimmed !== "" ? trimmed : DEFAULT_OUTPUT;
}

/**
 * Dropdown choices for a nameplate action: one per person plate plus (when
 * `includeSong`) the virtual song entry. Ids are STRINGS (Companion dropdown
 * ids). A non-array `plates` yields just the song entry / an empty list.
 *
 * @param {unknown} plates Array of `{id, name, role}` from the server.
 * @param {boolean} includeSong Append the "Pieseň" entry.
 * @returns {Array<{id: string, label: string}>}
 */
function nameplateChoices(plates, includeSong) {
  const list = (Array.isArray(plates) ? plates : []).map((p) => ({
    id: String(p.id),
    label: p.role ? `${p.name} — ${p.role}` : String(p.name),
  }));
  if (includeSong) {
    list.push({ id: SONG_CHOICE_ID, label: "Pieseň (aktuálna)" });
  }
  return list;
}

/**
 * Companion action option fields for a nameplate command. Show/toggle get a
 * plate dropdown (incl. song) + an output input; song/hide get only output.
 *
 * @param {string} commandId
 * @param {unknown} plates The current plate list.
 * @returns {Array<object>}
 */
function nameplateActionOptions(commandId, plates) {
  if (
    commandId === "stream_nameplate_show" ||
    commandId === "stream_nameplate_toggle"
  ) {
    return [
      {
        type: "dropdown",
        id: "plate",
        label: "Menovka",
        default: SONG_CHOICE_ID,
        choices: nameplateChoices(plates, true),
        allowCustom: false,
      },
      outputOption(),
    ];
  }
  return [outputOption()];
}

/**
 * Resolve a nameplate action's options into the wire `{command, payload}` to
 * send, or `{error}` when a required plate is not chosen. A show/toggle whose
 * plate is the song entry maps to the song command / a `{song:true}` toggle.
 * Never throws.
 *
 * @param {string} commandId The action id.
 * @param {object} [options] Companion action options (`plate`, `output`).
 * @returns {{command: string, payload: object} | {error: string}}
 */
function buildNameplateInvocation(commandId, options) {
  if (!isNameplateCommand(commandId)) {
    return { error: `not a nameplate command: ${commandId}` };
  }
  const opts = options || {};
  const output = resolveOutput(opts.output);

  if (commandId === "stream_nameplate_song") {
    return { command: "stream_nameplate_song", payload: { output } };
  }
  if (commandId === "stream_nameplate_hide") {
    return { command: "stream_nameplate_hide", payload: { output } };
  }

  // show / toggle: read the plate dropdown value.
  const value = opts.plate;
  if (value === SONG_CHOICE_ID) {
    if (commandId === "stream_nameplate_toggle") {
      return {
        command: "stream_nameplate_toggle",
        payload: { song: true, output },
      };
    }
    return { command: "stream_nameplate_song", payload: { output } };
  }
  const id = Number(value);
  if (!Number.isInteger(id) || id <= 0) {
    return { error: `${commandId}: choose a plate` };
  }
  return { command: commandId, payload: { id, output } };
}

/**
 * True when `target` (a plate id string or `"song"`) is the plate currently on
 * air, per the server's `nameplate_active_id` variable. Idle (`"-"`/empty) or a
 * non-string target is never active.
 *
 * @param {unknown} activeIdValue The `nameplate_active_id` variable value.
 * @param {unknown} target The plate id string / `"song"` a button watches.
 * @returns {boolean}
 */
function isNameplateActive(activeIdValue, target) {
  if (typeof target !== "string" || target.trim() === "") return false;
  const current = typeof activeIdValue === "string" ? activeIdValue.trim() : "";
  if (current === "" || current === PLACEHOLDER) return false;
  return current === target.trim();
}

/**
 * The full nameplate variable-definition id list for a plate list: the static
 * ids plus `nameplate_<id>_name`/`_role` per plate. The plugin unions this with
 * its base defs so `setVariableDefinitions` + the `applyVariablesMessage`
 * allowlist accept the per-plate values the server sends.
 *
 * @param {unknown} plates The current plate list.
 * @returns {string[]}
 */
function nameplateVariableIds(plates) {
  const ids = [...NAMEPLATE_STATIC_VARIABLE_IDS];
  for (const p of Array.isArray(plates) ? plates : []) {
    ids.push(`nameplate_${p.id}_name`, `nameplate_${p.id}_role`);
  }
  return ids;
}

/**
 * Preset definitions for the plate list: one button per person plate plus the
 * song button. Button text is the plate's variables (so renaming a plate in the
 * editor updates the button); the down action toggles the plate; the active
 * feedback lights it while on air. Shape = `CompanionButtonPresetDefinition`
 * (@companion-module/base ^1.13).
 *
 * @param {unknown} plates The current plate list.
 * @returns {Object<string, object>}
 */
function nameplatePresets(plates) {
  const presets = {};
  const preset = (target, nameVar, roleVar, label) => ({
    type: "button",
    category: "Menovky",
    name: label,
    style: {
      text: `$(presenter:${nameVar})\\n$(presenter:${roleVar})`,
      size: "auto",
      color: 0xffffff,
      bgcolor: 0x000000,
    },
    steps: [
      {
        down: [
          {
            actionId: "stream_nameplate_toggle",
            options: { plate: target, output: DEFAULT_OUTPUT },
          },
        ],
        up: [],
      },
    ],
    feedbacks: [
      {
        feedbackId: "stream_nameplate_active",
        options: { target },
        style: { bgcolor: 0x00aa00 },
      },
    ],
  });

  presets["nameplate_song"] = preset(
    SONG_CHOICE_ID,
    "nameplate_song_name",
    "nameplate_song_role",
    "Menovka: Pieseň",
  );
  for (const p of Array.isArray(plates) ? plates : []) {
    presets[`nameplate_${p.id}`] = preset(
      String(p.id),
      `nameplate_${p.id}_name`,
      `nameplate_${p.id}_role`,
      `Menovka: ${p.name}`,
    );
  }
  return presets;
}

module.exports = {
  DEFAULT_OUTPUT,
  STREAM_COMMAND_IDS,
  isStreamCommand,
  streamActionOptions,
  buildStreamPayload,
  parseOverlayList,
  isOverlayActive,
  isSceneActive,
  // #779 lower-third nameplates.
  NAMEPLATE_COMMAND_IDS,
  SONG_CHOICE_ID,
  isNameplateCommand,
  nameplateChoices,
  nameplateActionOptions,
  buildNameplateInvocation,
  isNameplateActive,
  nameplateVariableIds,
  nameplatePresets,
};
