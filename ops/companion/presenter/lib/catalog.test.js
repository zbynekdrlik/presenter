// #814: the module loads its dropdown choices (stage layouts, stream outputs,
// base/overlay scenes) from the server's `catalog` message instead of
// hardcoded lists. Two layers are covered:
//   1. the pure choice builders in `lib/catalog.js`;
//   2. the REAL `index.js` adapter (stubbed host, `lib/test-support.js`): a
//      catalog message rebuilds the action + feedback definitions, an
//      identical one does not, and buttons saved with the old static list /
//      free-text values still send exactly the same payload.
const { test, describe } = require("node:test");
const assert = require("node:assert/strict");
const {
  FALLBACK_STAGE_LAYOUT_CHOICES,
  normaliseCatalog,
  catalogEquals,
  stageLayoutChoices,
  streamOutputChoices,
  streamSceneChoices,
} = require("./catalog");
const { WS_OPEN, loadPresenterModule } = require("./test-support");

// A catalog shaped exactly like the server's `OutgoingMessage::Catalog`
// (`crates/presenter-server/src/companion/catalog.rs`), incl. a layout code the
// old hardcoded list never had (`future-layout`) to prove the list is dynamic.
const CATALOG_MESSAGE = {
  type: "catalog",
  layouts: [
    { code: "worship-snv", name: "WORSHIP SNV" },
    { code: "worship-pp", name: "WORSHIP PP" },
    { code: "api", name: "API" },
    { code: "api-ambient", name: "API + CG VIDEO" },
    { code: "future-layout", name: "FUTURE" },
  ],
  stream: [
    {
      slug: "stream",
      name: "Stream",
      scenes: [
        { name: "blank", kind: "base" },
        { name: "chvaly", kind: "base" },
        { name: "verse", kind: "overlay" },
        { name: "Logo", kind: "overlay" },
      ],
    },
    {
      slug: "moderator",
      name: "Moderátor",
      scenes: [
        { name: "Chvaly", kind: "base" }, // same name, other case → one entry
        { name: "Menovka", kind: "overlay" },
      ],
    },
  ],
};

const ids = (choices) => choices.map((choice) => choice.id);

// --------------------------------------------------------------------------- //
// Pure choice builders.
// --------------------------------------------------------------------------- //
describe("catalog — pure choice builders", () => {
  test("fallback layout list = the operator-selectable set incl. the API layouts", () => {
    assert.deepEqual(ids(FALLBACK_STAGE_LAYOUT_CHOICES), [
      "worship-snv",
      "worship-pp",
      "timer",
      "preach",
      "ndi-fullscreen",
      "bible",
      "fulltext",
      "api",
      "api-ambient",
    ]);
    const byId = Object.fromEntries(FALLBACK_STAGE_LAYOUT_CHOICES.map((c) => [c.id, c.label]));
    assert.equal(byId.api, "API");
    assert.equal(byId["api-ambient"], "API + CG VIDEO");
  });

  test("stageLayoutChoices: fallback before a catalog, the catalog's layouts after", () => {
    assert.deepEqual(stageLayoutChoices(null), FALLBACK_STAGE_LAYOUT_CHOICES);
    // A catalog with no layouts (malformed / older server) keeps the fallback.
    assert.deepEqual(stageLayoutChoices(normaliseCatalog({ layouts: [] })), FALLBACK_STAGE_LAYOUT_CHOICES);
    const choices = stageLayoutChoices(normaliseCatalog(CATALOG_MESSAGE));
    assert.deepEqual(choices, [
      { id: "worship-snv", label: "WORSHIP SNV" },
      { id: "worship-pp", label: "WORSHIP PP" },
      { id: "api", label: "API" },
      { id: "api-ambient", label: "API + CG VIDEO" },
      { id: "future-layout", label: "FUTURE" },
    ]);
  });

  test("stageLayoutChoices returns a copy — callers cannot corrupt the fallback", () => {
    stageLayoutChoices(null)[0].label = "MUTATED";
    assert.equal(FALLBACK_STAGE_LAYOUT_CHOICES[0].label, "WORSHIP SNV");
  });

  test("normaliseCatalog drops malformed entries and never throws", () => {
    const catalog = normaliseCatalog({
      layouts: [{ code: "  timer  ", name: "" }, { code: "" }, null, { name: "no code" }],
      stream: [
        { slug: "stream", name: "Stream", scenes: [{ name: " Verse ", kind: "overlay" }, { name: "x", kind: "bogus" }, { kind: "base" }] },
        { name: "no slug" },
        "junk",
      ],
    });
    assert.deepEqual(catalog, {
      layouts: [{ code: "timer", name: "timer" }],
      stream: [{ slug: "stream", name: "Stream", scenes: [{ name: "Verse", kind: "overlay" }] }],
    });
    assert.deepEqual(normaliseCatalog(undefined), { layouts: [], stream: [] });
    assert.deepEqual(normaliseCatalog("nope"), { layouts: [], stream: [] });
  });

  test("streamOutputChoices: default output before a catalog, slugs (name + slug label) after", () => {
    assert.deepEqual(streamOutputChoices(null, "stream"), [{ id: "stream", label: "stream" }]);
    assert.deepEqual(streamOutputChoices(normaliseCatalog(CATALOG_MESSAGE), "stream"), [
      { id: "stream", label: "Stream (stream)" },
      { id: "moderator", label: "Moderátor (moderator)" },
    ]);
    // An output whose name IS its slug is not labelled twice.
    const same = normaliseCatalog({ stream: [{ slug: "timer", name: "timer", scenes: [] }] });
    assert.deepEqual(streamOutputChoices(same, "stream"), [{ id: "timer", label: "timer" }]);
  });

  test("streamSceneChoices splits by kind, de-duplicates case-insensitively, labels outputs", () => {
    const catalog = normaliseCatalog(CATALOG_MESSAGE);
    assert.deepEqual(streamSceneChoices(catalog, "base"), [
      { id: "blank", label: "blank (Stream)" },
      { id: "chvaly", label: "chvaly (Stream, Moderátor)" },
    ]);
    assert.deepEqual(ids(streamSceneChoices(catalog, "overlay")), ["verse", "Logo", "Menovka"]);
    // Restricted to one output → its scenes only, no output label.
    assert.deepEqual(streamSceneChoices(catalog, "overlay", "stream"), [
      { id: "verse", label: "verse" },
      { id: "Logo", label: "Logo" },
    ]);
    assert.deepEqual(streamSceneChoices(null, "base"), []);
    assert.deepEqual(streamSceneChoices(catalog, "overlay", "missing"), []);
  });

  test("catalogEquals compares content", () => {
    const a = normaliseCatalog(CATALOG_MESSAGE);
    const b = normaliseCatalog(JSON.parse(JSON.stringify(CATALOG_MESSAGE)));
    assert.ok(catalogEquals(a, b));
    assert.ok(catalogEquals(null, null));
    assert.ok(!catalogEquals(null, a));
    const renamed = normaliseCatalog({ ...CATALOG_MESSAGE, layouts: CATALOG_MESSAGE.layouts.slice(1) });
    assert.ok(!catalogEquals(a, renamed));
  });
});

// --------------------------------------------------------------------------- //
// The real index.js adapter.
// --------------------------------------------------------------------------- //
const presenter = loadPresenterModule();

function newInstance() {
  const instance = new presenter.factory({});
  instance._setupActions();
  instance._setupFeedbacks();
  return instance;
}

function option(definitions, definitionId, optionId) {
  const found = definitions[definitionId].options.find((o) => o.id === optionId);
  assert.ok(found, `${definitionId} must have an option "${optionId}"`);
  return found;
}

// Press a button the way Companion does and return the sent {command, payload}.
function press(instance, actionId, options) {
  const sent = [];
  instance.ws = { readyState: WS_OPEN, send: (raw) => sent.push(JSON.parse(raw)) };
  instance.actionDefinitions[actionId].callback({ options });
  assert.equal(sent.length, 1, "one command per press");
  return sent[0];
}

describe("index.js adapter — catalog-driven dropdowns", () => {
  test("before a catalog: the static fallbacks (incl. api + api-ambient)", () => {
    const instance = newInstance();
    const layout = option(instance.actionDefinitions, "stage.layout", "code");
    assert.equal(layout.type, "dropdown");
    assert.equal(layout.allowCustom, true);
    assert.deepEqual(layout.choices, FALLBACK_STAGE_LAYOUT_CHOICES);
    const output = option(instance.actionDefinitions, "stream_scene_set", "output");
    assert.equal(output.type, "dropdown");
    assert.deepEqual(output.choices, [{ id: "stream", label: "stream" }]);
    const scene = option(instance.actionDefinitions, "stream_scene_set", "scene");
    assert.equal(scene.type, "dropdown");
    assert.equal(scene.allowCustom, true);
    assert.deepEqual(scene.choices, []);
  });

  test("a catalog message rebuilds every layout / output / scene dropdown", () => {
    const instance = newInstance();
    instance._handleMessage(CATALOG_MESSAGE);
    const actions = instance.actionDefinitions;

    assert.deepEqual(ids(option(actions, "stage.layout", "code").choices), [
      "worship-snv",
      "worship-pp",
      "api",
      "api-ambient",
      "future-layout",
    ]);
    // Base scenes for the scene action, overlay scenes for the overlay actions.
    assert.deepEqual(ids(option(actions, "stream_scene_set", "scene").choices), ["blank", "chvaly"]);
    for (const id of ["stream_overlay_on", "stream_overlay_off", "stream_overlay_toggle"]) {
      assert.deepEqual(ids(option(actions, id, "scene").choices), ["verse", "Logo", "Menovka"], id);
    }
    // Every output-taking action (stream + nameplate) gets the output list.
    for (const id of [
      "stream_scene_set",
      "stream_scene_clear",
      "stream_overlay_toggle",
      "stream_clear",
      "stream_nameplate_toggle",
      "stream_nameplate_hide",
    ]) {
      const output = option(actions, id, "output");
      assert.deepEqual(ids(output.choices), ["stream", "moderator"], id);
      assert.equal(output.allowCustom, true, id);
      assert.equal(output.default, "stream", id);
    }
    // Option ids are unchanged, so saved buttons keep their keys.
    assert.deepEqual(actions.stream_scene_set.options.map((o) => o.id), ["scene", "output"]);
    assert.deepEqual(actions.stream_clear.options.map((o) => o.id), ["output"]);
    assert.deepEqual(actions["stage.layout"].options.map((o) => o.id), ["code"]);

    // Feedbacks offer the DEFAULT output's scenes (the only one they track).
    const feedbacks = instance.feedbackDefinitions;
    const overlayFb = option(feedbacks, "stream_overlay_active", "scene");
    assert.equal(overlayFb.type, "dropdown");
    assert.equal(overlayFb.allowCustom, true);
    assert.deepEqual(ids(overlayFb.choices), ["verse", "Logo"]);
    assert.deepEqual(ids(option(feedbacks, "stream_scene_active", "scene").choices), ["blank", "chvaly"]);
  });

  test("re-defines on a changed catalog only — an identical one is skipped", () => {
    const instance = newInstance();
    instance._handleMessage(CATALOG_MESSAGE);
    const after = { ...instance.calls };

    // Same content again (e.g. a reconnect) → no re-registration.
    instance._handleMessage(JSON.parse(JSON.stringify(CATALOG_MESSAGE)));
    assert.deepEqual(instance.calls, after);

    // A new overlay scene → actions + feedbacks re-registered with it.
    const changed = JSON.parse(JSON.stringify(CATALOG_MESSAGE));
    changed.stream[0].scenes.push({ name: "Verš s prekladom", kind: "overlay" });
    instance._handleMessage(changed);
    assert.equal(instance.calls.setActionDefinitions, after.setActionDefinitions + 1);
    assert.equal(instance.calls.setFeedbackDefinitions, after.setFeedbackDefinitions + 1);
    assert.equal(instance.calls.checkFeedbacks, after.checkFeedbacks + 1);
    assert.ok(
      ids(option(instance.actionDefinitions, "stream_overlay_toggle", "scene").choices).includes(
        "Verš s prekladom",
      ),
    );
  });

  test("legacy stored values still resolve to the same payloads", () => {
    const instance = newInstance();
    instance._handleMessage(CATALOG_MESSAGE);

    // A layout from the OLD static list that this catalog does not list, and a
    // free-typed custom code: both are sent verbatim (allowCustom).
    assert.deepEqual(press(instance, "stage.layout", { code: "fulltext" }), {
      type: "command",
      command: "stage.layout",
      payload: { code: "fulltext" },
    });
    assert.deepEqual(press(instance, "stage.layout", { code: "worship-pp" }).payload, {
      code: "worship-pp",
    });

    // A button saved when `scene`/`output` were text inputs: same keys, string
    // values — sent unchanged (trimmed), even for a name not in the catalog.
    assert.deepEqual(press(instance, "stream_scene_set", { scene: "Chvaly", output: "stream" }).payload, {
      scene: "Chvaly",
      output: "stream",
    });
    assert.deepEqual(
      press(instance, "stream_overlay_toggle", { scene: " old overlay ", output: "" }).payload,
      { scene: "old overlay", output: "stream" },
    );
    assert.deepEqual(press(instance, "stream_clear", { output: "moderator" }).payload, {
      output: "moderator",
    });
  });

  test("a legacy free-text feedback scene still evaluates", () => {
    const instance = newInstance();
    instance._handleMessage(CATALOG_MESSAGE);
    instance.variables.set("stream_overlays", "verse, Logo");
    instance.variables.set("stream_scene", "chvaly");
    const overlay = instance.feedbackDefinitions.stream_overlay_active;
    const scene = instance.feedbackDefinitions.stream_scene_active;
    assert.equal(overlay.callback({ options: { scene: "Verse" } }), true);
    assert.equal(overlay.callback({ options: { scene: "Menovka" } }), false);
    assert.equal(scene.callback({ options: { scene: "CHVALY" } }), true);
  });
});
