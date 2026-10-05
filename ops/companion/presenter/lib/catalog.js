"use strict";

/**
 * Presenter catalog (issue #814) — the CHOICES the module's dropdowns offer.
 *
 * The server pushes `{type:"catalog", layouts:[{code,name}], stream:[{slug,
 * name, scenes:[{name, kind}]}]}` over `/companion/ws` on connect and again
 * whenever a stream config change alters it (`crates/presenter-server/src/
 * companion/catalog.rs`). The module keeps the last catalog and rebuilds its
 * action + feedback definitions from it (the same model as the #779 nameplate
 * list), so a layout / output / scene added in presenter shows up in Companion
 * without a module release or a Companion restart.
 *
 * Every dropdown built from here is `allowCustom: true` and keeps the SAME
 * option ids as the old static list / text inputs, so a stored value (a string)
 * keeps resolving even when it is not in the current catalog — no upgrade
 * script is needed (`.claude/rules/companion-upgrade-scripts.md`).
 *
 * Until the first catalog arrives (or against an older server that never sends
 * one) the static fallbacks below apply. Pure + dependency-free, unit-tested
 * with `node --test` (`lib/catalog.test.js`); `index.js` is a thin adapter.
 */

const SCENE_KINDS = ["base", "overlay"];

// Pre-catalog fallback for the `stage.layout` dropdown: the operator-selectable
// set `StageDisplayLayout::operator_selectable()` served when this was written.
const FALLBACK_STAGE_LAYOUT_CHOICES = [
  { id: "worship-snv", label: "WORSHIP SNV" },
  { id: "worship-pp", label: "WORSHIP PP" },
  { id: "timer", label: "TIMER" },
  { id: "preach", label: "PREACH" },
  { id: "ndi-fullscreen", label: "NDI FULLSCREEN" },
  { id: "bible", label: "BIBLE" },
  { id: "fulltext", label: "FULL TEXT" },
  { id: "api", label: "API" },
  { id: "api-ambient", label: "API + CG VIDEO" },
];

function cleanString(value) {
  return typeof value === "string" ? value.trim() : "";
}

function normaliseLayouts(raw) {
  const layouts = [];
  for (const entry of Array.isArray(raw) ? raw : []) {
    const code = cleanString(entry && entry.code);
    if (code === "") continue;
    layouts.push({ code, name: cleanString(entry.name) || code });
  }
  return layouts;
}

function normaliseScenes(raw) {
  const scenes = [];
  for (const entry of Array.isArray(raw) ? raw : []) {
    const name = cleanString(entry && entry.name);
    if (name === "" || !SCENE_KINDS.includes(entry.kind)) continue;
    scenes.push({ name, kind: entry.kind });
  }
  return scenes;
}

function normaliseOutputs(raw) {
  const outputs = [];
  for (const entry of Array.isArray(raw) ? raw : []) {
    const slug = cleanString(entry && entry.slug);
    if (slug === "") continue;
    outputs.push({
      slug,
      name: cleanString(entry.name) || slug,
      scenes: normaliseScenes(entry.scenes),
    });
  }
  return outputs;
}

/**
 * Normalise a server `catalog` message into `{layouts, stream}`. Malformed
 * entries (missing code/slug/name, unknown scene kind) are dropped; a
 * non-object message yields empty lists. Never throws.
 *
 * @param {unknown} msg The parsed `catalog` message.
 * @returns {{layouts: Array<{code: string, name: string}>, stream: Array<{slug: string, name: string, scenes: Array<{name: string, kind: string}>}>}}
 */
function normaliseCatalog(msg) {
  const source = msg && typeof msg === "object" ? msg : {};
  return {
    layouts: normaliseLayouts(source.layouts),
    stream: normaliseOutputs(source.stream),
  };
}

/**
 * True when two normalised catalogs carry the same content (`null` = no
 * catalog yet). Used to skip re-registering definitions for an unchanged one.
 *
 * @param {object|null} a
 * @param {object|null} b
 * @returns {boolean}
 */
function catalogEquals(a, b) {
  return JSON.stringify(a ?? null) === JSON.stringify(b ?? null);
}

/**
 * `stage.layout` dropdown choices: the catalog's layouts, or the static
 * fallback when there is no catalog yet / it carries no layouts.
 *
 * @param {object|null} catalog A normalised catalog.
 * @returns {Array<{id: string, label: string}>}
 */
function stageLayoutChoices(catalog) {
  const layouts = catalog && Array.isArray(catalog.layouts) ? catalog.layouts : [];
  if (layouts.length === 0) {
    return FALLBACK_STAGE_LAYOUT_CHOICES.map((choice) => ({ ...choice }));
  }
  return layouts.map((layout) => ({ id: layout.code, label: layout.name }));
}

/**
 * Stream output dropdown choices (id = the slug the commands send). Before a
 * catalog — or when it lists no outputs — only the default output.
 *
 * @param {object|null} catalog A normalised catalog.
 * @param {string} defaultOutput The default output slug (`"stream"`).
 * @returns {Array<{id: string, label: string}>}
 */
function streamOutputChoices(catalog, defaultOutput) {
  const outputs = catalog && Array.isArray(catalog.stream) ? catalog.stream : [];
  if (outputs.length === 0) {
    return [{ id: defaultOutput, label: defaultOutput }];
  }
  return outputs.map((output) => ({
    id: output.slug,
    label: output.name === output.slug ? output.slug : `${output.name} (${output.slug})`,
  }));
}

/**
 * Scene dropdown choices of one `kind` ("base" for the scene actions, "overlay"
 * for the overlay ones). Id = the scene NAME the commands send (the server
 * matches it case-insensitively, so names are de-duplicated case-insensitively,
 * first spelling wins). Scenes of every output are offered unless `onlyOutput`
 * names one slug; with more than one output in scope a label names the
 * output(s) the scene belongs to. Empty before a catalog (allowCustom keeps a
 * typed value working).
 *
 * @param {object|null} catalog A normalised catalog.
 * @param {"base"|"overlay"} kind
 * @param {string} [onlyOutput] Restrict to this output slug.
 * @returns {Array<{id: string, label: string}>}
 */
function streamSceneChoices(catalog, kind, onlyOutput) {
  const all = catalog && Array.isArray(catalog.stream) ? catalog.stream : [];
  const outputs = onlyOutput ? all.filter((output) => output.slug === onlyOutput) : all;
  const byKey = new Map();
  for (const output of outputs) {
    for (const scene of output.scenes) {
      if (scene.kind !== kind) continue;
      const key = scene.name.toLowerCase();
      if (!byKey.has(key)) byKey.set(key, { name: scene.name, outputs: [] });
      const entry = byKey.get(key);
      if (!entry.outputs.includes(output.name)) entry.outputs.push(output.name);
    }
  }
  const multiOutput = outputs.length > 1;
  return [...byKey.values()].map((entry) => ({
    id: entry.name,
    label: multiOutput ? `${entry.name} (${entry.outputs.join(", ")})` : entry.name,
  }));
}

module.exports = {
  FALLBACK_STAGE_LAYOUT_CHOICES,
  normaliseCatalog,
  catalogEquals,
  stageLayoutChoices,
  streamOutputChoices,
  streamSceneChoices,
};
