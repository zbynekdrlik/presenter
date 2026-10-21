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

// #814: `output` and `scene` are DROPDOWNS fed by the server catalog
// (`lib/catalog.js`), with `allowCustom: true` and the SAME option ids the old
// text inputs had — a saved button's stored string keeps resolving (even one
// not in the current catalog), so no upgrade script is needed.

/**
 * The `output` option: a dropdown of the catalog's outputs (slug ids), default
 * `"stream"`. Without choices (no catalog yet) it offers the default output.
 *
 * @param {Array<{id: string, label: string}>} [choices]
 * @returns {object}
 */
function outputOption(choices) {
  const list =
    Array.isArray(choices) && choices.length > 0
      ? choices
      : [{ id: DEFAULT_OUTPUT, label: DEFAULT_OUTPUT }];
  return {
    type: "dropdown",
    id: "output",
    label: "Output (default: stream)",
    default: DEFAULT_OUTPUT,
    choices: list,
    allowCustom: true,
  };
}

/**
 * The `scene` option: a dropdown of scene NAMES (the server matches them
 * case-insensitively), defaulting to the first choice. Empty before a catalog —
 * a typed (custom) name still works.
 *
 * @param {Array<{id: string, label: string}>} [choices]
 * @param {string} [label]
 * @returns {object}
 */
function sceneOption(choices, label) {
  const list = Array.isArray(choices) ? choices : [];
  return {
    type: "dropdown",
    id: "scene",
    label: label || "Scene (matched case-insensitively)",
    default: list.length > 0 ? list[0].id : "",
    choices: list,
    allowCustom: true,
  };
}

/**
 * Companion action option fields for a stream command.
 * Scene-required commands get a `scene` + `output` dropdown — base scenes for
 * `stream_scene_set`, overlay scenes for the overlay actions; the clear
 * commands get only an `output` dropdown.
 *
 * @param {string} commandId
 * @param {{outputs?: Array<object>, baseScenes?: Array<object>, overlayScenes?: Array<object>}} [choices]
 *   Dropdown choices from the catalog (`lib/catalog.js`); omitted → fallbacks.
 * @returns {Array<object>} option-field definitions (empty for non-stream ids)
 */
function streamActionOptions(commandId, choices) {
  if (!isStreamCommand(commandId)) return [];
  const c = choices || {};
  if (commandId === "stream_scene_set") {
    return [sceneOption(c.baseScenes, "Base scene"), outputOption(c.outputs)];
  }
  if (SCENE_REQUIRED.has(commandId)) {
    return [sceneOption(c.overlayScenes, "Overlay scene"), outputOption(c.outputs)];
  }
  return [outputOption(c.outputs)];
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

// The nameplate command NAMES (all `stream_`-prefixed, so the server's
// parse_command delegation routes them to companion/stream.rs). #870 added the
// grid's `stream_nameplate_slot`.
const NAMEPLATE_COMMAND_IDS = [
  "stream_nameplate_show",
  "stream_nameplate_song",
  "stream_nameplate_toggle",
  "stream_nameplate_hide",
  "stream_nameplate_slot",
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

// --------------------------------------------------------------------------- //
// Nameplate GRID (issue #870).
//
// A slot is a 1-based POSITION in the output's person-plate list (the editor
// order). The server resolves BOTH the button label (`nameplate_slot_<n>_name`
// / `_role` / `_id` variables) and the press (`stream_nameplate_slot {slot}`,
// read at press time) from that one list, so a grid button fires exactly the
// plate its label shows — a rename or reorder in Presenter updates the label,
// and a new person fills the next slot without touching Companion. Slots
// 1..11 always exist (the 4×3 grid = the song + 11 people); more plates add
// more slots.
// --------------------------------------------------------------------------- //

// Slots that always exist — mirrors the server's NAMEPLATE_MIN_SLOTS.
const NAMEPLATE_MIN_SLOTS = 11;

// The preset category of the grid (song first, then slots 1..11).
const NAMEPLATE_GRID_CATEGORY = "Menovky — mriežka";

/**
 * How many slots exist for a plate list: max(11, plates.length).
 *
 * @param {unknown} plates The current plate list.
 * @returns {number}
 */
function nameplateSlotCount(plates) {
  const count = Array.isArray(plates) ? plates.length : 0;
  return Math.max(NAMEPLATE_MIN_SLOTS, count);
}

/**
 * Parse a slot option value (a dropdown string id, a typed custom value or a
 * number) into a whole number >= 1, or `null` for anything else (0, negative,
 * fractional, empty, non-numeric).
 *
 * @param {unknown} value
 * @returns {number | null}
 */
function parseSlot(value) {
  let text = "";
  if (typeof value === "number") text = String(value);
  else if (typeof value === "string") text = value.trim();
  if (!/^[0-9]+$/.test(text)) return null;
  const slot = Number(text);
  return Number.isSafeInteger(slot) && slot >= 1 ? slot : null;
}

/**
 * Dropdown choices for a slot option: one per slot, labelled with the name of
 * the plate the slot holds NOW (or "(prázdna)" for an empty slot). Ids are
 * strings (Companion dropdown ids).
 *
 * @param {unknown} plates The current plate list.
 * @returns {Array<{id: string, label: string}>}
 */
function nameplateSlotChoices(plates) {
  const list = Array.isArray(plates) ? plates : [];
  const choices = [];
  for (let n = 1; n <= nameplateSlotCount(list); n++) {
    const plate = list[n - 1];
    const name = plate ? String(plate.name) : "(prázdna)";
    choices.push({ id: String(n), label: `Pozícia ${n} — ${name}` });
  }
  return choices;
}

/**
 * The `slot` option of the grid action + feedback. `allowCustom` so a stored
 * slot number always passes, even one past the current list.
 *
 * @param {unknown} plates The current plate list.
 * @returns {object}
 */
function nameplateSlotOption(plates) {
  return {
    type: "dropdown",
    id: "slot",
    label: "Pozícia v zozname menoviek",
    default: "1",
    choices: nameplateSlotChoices(plates),
    allowCustom: true,
  };
}

// The grid action's `mode` choices (the server's SlotMode).
const SLOT_MODE_CHOICES = [
  { id: "toggle", label: "Prepnúť (zobraziť / skryť)" },
  { id: "show", label: "Zobraziť" },
];

/**
 * True when slot `slot` holds a plate and that plate is the one on air: its
 * `nameplate_slot_<n>_id` variable is non-empty and equals
 * `nameplate_active_id`. An empty slot, an idle plate (`"-"`), the song plate
 * and an invalid slot never light.
 *
 * @param {{get: Function}} variables The module's variable cache (a Map).
 * @param {unknown} slot The feedback's slot option value.
 * @returns {boolean}
 */
function isNameplateSlotActive(variables, slot) {
  const n = parseSlot(slot);
  if (n === null || !variables || typeof variables.get !== "function") return false;
  const slotId = variables.get(`nameplate_slot_${n}_id`);
  const id = typeof slotId === "string" ? slotId.trim() : "";
  if (id === "") return false;
  return isNameplateActive(variables.get("nameplate_active_id"), id);
}

/**
 * Companion action option fields for a nameplate command. Show/toggle get a
 * plate dropdown (incl. song) + an output dropdown; song/hide get only output;
 * the #870 grid slot gets a slot dropdown + a mode + an output dropdown.
 *
 * @param {string} commandId
 * @param {unknown} plates The current plate list.
 * @param {Array<{id: string, label: string}>} [outputChoices] Catalog outputs (#814).
 * @returns {Array<object>}
 */
function nameplateActionOptions(commandId, plates, outputChoices) {
  if (commandId === "stream_nameplate_slot") {
    return [
      nameplateSlotOption(plates),
      {
        type: "dropdown",
        id: "mode",
        label: "Režim",
        default: "toggle",
        choices: SLOT_MODE_CHOICES,
        allowCustom: false,
      },
      outputOption(outputChoices),
    ];
  }
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
      outputOption(outputChoices),
    ];
  }
  return [outputOption(outputChoices)];
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
  if (commandId === "stream_nameplate_slot") {
    const slot = parseSlot(opts.slot);
    if (slot === null) {
      return { error: `${commandId}: choose a slot (a position 1 or higher)` };
    }
    const mode = opts.mode === "show" ? "show" : "toggle";
    return { command: commandId, payload: { slot, mode, output } };
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
 * ids, `nameplate_<id>_name`/`_role` per plate, and (#870)
 * `nameplate_slot_<n>_name`/`_role`/`_id` for n = 1..max(11, plates). The
 * plugin unions this with its base defs so `setVariableDefinitions` + the
 * `applyVariablesMessage` allowlist accept the values the server sends.
 *
 * @param {unknown} plates The current plate list.
 * @returns {string[]}
 */
function nameplateVariableIds(plates) {
  const ids = [...NAMEPLATE_STATIC_VARIABLE_IDS];
  for (const p of Array.isArray(plates) ? plates : []) {
    ids.push(`nameplate_${p.id}_name`, `nameplate_${p.id}_role`);
  }
  for (let n = 1; n <= nameplateSlotCount(plates); n++) {
    ids.push(
      `nameplate_slot_${n}_name`,
      `nameplate_slot_${n}_role`,
      `nameplate_slot_${n}_id`,
    );
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
// One nameplate button preset: two-line text from the name + role variables,
// one down action, one green "on air" feedback.
function nameplateButtonPreset(category, label, nameVar, roleVar, action, feedback) {
  return {
    type: "button",
    category,
    name: label,
    style: {
      text: `$(presenter:${nameVar})\\n$(presenter:${roleVar})`,
      size: "auto",
      color: 0xffffff,
      bgcolor: 0x000000,
    },
    steps: [{ down: [action], up: [] }],
    feedbacks: [{ ...feedback, style: { bgcolor: 0x00aa00 } }],
  };
}

// The song plate's button (toggles the song entry, lit while the song is on air).
function songPreset(category, label) {
  return nameplateButtonPreset(
    category,
    label,
    "nameplate_song_name",
    "nameplate_song_role",
    {
      actionId: "stream_nameplate_toggle",
      options: { plate: SONG_CHOICE_ID, output: DEFAULT_OUTPUT },
    },
    { feedbackId: "stream_nameplate_active", options: { target: SONG_CHOICE_ID } },
  );
}

function nameplatePresets(plates) {
  const presets = {};
  const preset = (target, nameVar, roleVar, label) =>
    nameplateButtonPreset(
      "Menovky",
      label,
      nameVar,
      roleVar,
      {
        actionId: "stream_nameplate_toggle",
        options: { plate: target, output: DEFAULT_OUTPUT },
      },
      { feedbackId: "stream_nameplate_active", options: { target } },
    );

  presets["nameplate_song"] = songPreset("Menovky", "Menovka: Pieseň");
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

/**
 * The #870 grid presets, in grid order: the song button first, then slots
 * 1..11. Each slot button takes its text from `nameplate_slot_<n>_*`, fires
 * `stream_nameplate_slot {slot: n}` and lights on `stream_nameplate_slot_active
 * {slot: n}` — the SAME n for all three, from one loop, so a button cannot show
 * one slot and fire another. Independent of the plate list (the server fills
 * the slots), so the presets never change as people are added.
 *
 * @returns {Object<string, object>}
 */
function nameplateGridPresets() {
  const presets = {
    nameplate_grid_song: songPreset(NAMEPLATE_GRID_CATEGORY, "Mriežka: Pieseň"),
  };
  for (let n = 1; n <= NAMEPLATE_MIN_SLOTS; n++) {
    const slot = String(n);
    presets[`nameplate_grid_slot_${n}`] = nameplateButtonPreset(
      NAMEPLATE_GRID_CATEGORY,
      `Mriežka: pozícia ${n}`,
      `nameplate_slot_${n}_name`,
      `nameplate_slot_${n}_role`,
      {
        actionId: "stream_nameplate_slot",
        options: { slot, mode: "toggle", output: DEFAULT_OUTPUT },
      },
      { feedbackId: "stream_nameplate_slot_active", options: { slot } },
    );
  }
  return presets;
}

module.exports = {
  DEFAULT_OUTPUT,
  STREAM_COMMAND_IDS,
  isStreamCommand,
  streamActionOptions,
  sceneOption,
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
  // #870 nameplate grid.
  NAMEPLATE_MIN_SLOTS,
  NAMEPLATE_GRID_CATEGORY,
  nameplateSlotCount,
  parseSlot,
  nameplateSlotChoices,
  nameplateSlotOption,
  isNameplateSlotActive,
  nameplateGridPresets,
};
