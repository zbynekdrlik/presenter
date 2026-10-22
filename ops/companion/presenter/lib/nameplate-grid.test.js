// #870: the nameplate GRID through the REAL `index.js` adapter (stubbed host,
// `lib/test-support.js`). A slot is a position in Presenter's plate list; the
// server fills the slot label variables and resolves the press from that one
// list. Covered here: the slot action is registered (no output option) and
// sends exactly the slot the button names, always for `stream`, with the plate
// id its label shows (`expect_id`); the slot feedback lights from the
// variables the server sends; the slot variables are accepted (also past 11
// once the list has grown) and a removed slot's cached value is forgotten; the
// grid presets come first, song first.
const { test, describe } = require("node:test");
const assert = require("node:assert/strict");
const { WS_OPEN, loadPresenterModule } = require("./test-support");
const { NAMEPLATE_GRID_CATEGORY } = require("./stream");

const presenter = loadPresenterModule();

const PLATES = [
  { id: 9, name: "Ján Novák", role: "pastor" },
  { id: 4, name: "Eva Malá", role: "" },
];

function nameplatesMessage(plates) {
  return { type: "nameplates", output: "stream", plates };
}

function variablesMessage(values) {
  return {
    type: "variables",
    values: Object.entries(values).map(([name, value]) => ({ name, value })),
  };
}

// A fresh instance that has received the plate list (as on connect).
function instanceWith(plates) {
  const instance = new presenter.factory({});
  instance._handleMessage(nameplatesMessage(plates));
  return instance;
}

// Press an action the way Companion does; returns every sent message.
function press(instance, actionId, options) {
  const sent = [];
  instance.ws = { readyState: WS_OPEN, send: (raw) => sent.push(JSON.parse(raw)) };
  instance.actionDefinitions[actionId].callback({ options });
  return sent;
}

describe("index.js adapter — nameplate grid (#870)", () => {
  test("the slot action is registered with slot / mode options, no output", () => {
    const instance = instanceWith(PLATES);
    const action = instance.actionDefinitions.stream_nameplate_slot;
    assert.ok(action, "stream_nameplate_slot action registered");
    assert.deepEqual(
      action.options.map((o) => o.id),
      ["slot", "mode"],
    );
    const slot = action.options[0];
    assert.equal(slot.allowCustom, true);
    assert.equal(slot.choices.length, 11);
    assert.equal(slot.choices[0].label, "Pozícia 1 — Ján Novák");
    assert.equal(slot.choices[2].label, "Pozícia 3 — (prázdna)");
  });

  test("a press sends the slot the button names; an invalid slot sends nothing", () => {
    const instance = instanceWith(PLATES);
    assert.deepEqual(
      press(instance, "stream_nameplate_slot", { slot: "2", mode: "toggle", output: "stream" }),
      [
        {
          type: "command",
          command: "stream_nameplate_slot",
          payload: { slot: 2, mode: "toggle", output: "stream" },
        },
      ],
    );
    assert.deepEqual(
      press(instance, "stream_nameplate_slot", { slot: "5", mode: "show" })[0].payload,
      { slot: 5, mode: "show", output: "stream" },
    );
    // A stored output (a hand-configured button) never redirects a slot.
    assert.equal(
      press(instance, "stream_nameplate_slot", { slot: "1", output: "moderator" })[0].payload
        .output,
      "stream",
    );
    assert.deepEqual(press(instance, "stream_nameplate_slot", { slot: "0" }), []);
    assert.deepEqual(press(instance, "stream_nameplate_slot", { slot: "" }), []);
  });

  test("a press carries the plate id the slot's label shows; an empty slot sends none", () => {
    const instance = instanceWith(PLATES);
    instance._handleMessage(
      variablesMessage({
        nameplate_slot_1_name: "Ján Novák",
        nameplate_slot_1_id: "9",
        nameplate_slot_2_name: "Eva Malá",
        nameplate_slot_2_id: "4",
        nameplate_slot_3_name: "",
        nameplate_slot_3_id: "",
      }),
    );
    assert.deepEqual(press(instance, "stream_nameplate_slot", { slot: "2" })[0].payload, {
      slot: 2,
      mode: "toggle",
      output: "stream",
      expect_id: 4,
    });
    const empty = press(instance, "stream_nameplate_slot", { slot: "3" })[0].payload;
    assert.equal("expect_id" in empty, false);
    // The label shows slot 3 empty → the press says so, and the server refuses
    // a plate that filled it before the new label arrived.
    assert.equal(empty.expect_empty, true);

    // The label moves (a reorder in Presenter) → the press follows the label.
    instance._handleMessage(variablesMessage({ nameplate_slot_2_id: "9" }));
    assert.equal(press(instance, "stream_nameplate_slot", { slot: "2" })[0].payload.expect_id, 9);
  });

  test("every grid preset's action, pressed, fires its own slot", () => {
    const instance = instanceWith(PLATES);
    for (let n = 1; n <= 11; n++) {
      const preset = instance.presetDefinitions[`nameplate_grid_slot_${n}`];
      const action = preset.steps[0].down[0];
      const sent = press(instance, action.actionId, action.options);
      assert.equal(sent.length, 1);
      assert.equal(sent[0].command, "stream_nameplate_slot");
      assert.equal(sent[0].payload.slot, n);
    }
  });

  test("the slot feedback lights from the variables the server sends", () => {
    const instance = instanceWith(PLATES);
    const feedback = instance.feedbackDefinitions.stream_nameplate_slot_active;
    assert.ok(feedback, "stream_nameplate_slot_active feedback registered");
    assert.equal(feedback.type, "boolean", "a boolean feedback (#780), never advanced");
    assert.equal(feedback.defaultStyle.bgcolor, 0x00aa00);
    assert.equal(feedback.options[0].id, "slot");

    const checksBefore = instance.calls.checkFeedbacks;
    instance._handleMessage(
      variablesMessage({
        nameplate_slot_1_id: "9",
        nameplate_slot_2_id: "4",
        nameplate_slot_3_id: "",
        nameplate_active_id: "4",
      }),
    );
    assert.equal(instance.calls.checkFeedbacks, checksBefore + 1, "re-evaluated once");
    assert.equal(feedback.callback({ options: { slot: "2" } }), true);
    assert.equal(feedback.callback({ options: { slot: "1" } }), false);
    assert.equal(feedback.callback({ options: { slot: "3" } }), false);

    // A reorder in Presenter: slot 1 now holds the on-air plate.
    instance._handleMessage(
      variablesMessage({ nameplate_slot_1_id: "4", nameplate_slot_2_id: "9" }),
    );
    assert.equal(feedback.callback({ options: { slot: "1" } }), true);
    assert.equal(feedback.callback({ options: { slot: "2" } }), false);

    // Off air → nothing lit.
    instance._handleMessage(variablesMessage({ nameplate_active_id: "-" }));
    assert.equal(feedback.callback({ options: { slot: "1" } }), false);
  });

  test("slot variables are defined for 11 slots, and for more once the list grows", () => {
    const instance = instanceWith(PLATES);
    const defined = instance.variableDefinitions.map((d) => d.variableId);
    assert.ok(defined.includes("nameplate_slot_11_name"));
    assert.ok(!defined.includes("nameplate_slot_12_name"));

    // Slot 12 is not defined yet → its value is dropped …
    instance._handleMessage(
      variablesMessage({ nameplate_slot_11_name: "Jedenásty", nameplate_slot_12_name: "Osoba 12" }),
    );
    assert.equal(instance.variables.get("nameplate_slot_11_name"), "Jedenásty");
    assert.equal(instance.variables.get("nameplate_slot_12_name"), undefined);

    // … which is why the server sends the plate list BEFORE the variables:
    // with 12 plates the module defines slot 12 first and accepts its value.
    const twelve = Array.from({ length: 12 }, (_, i) => ({
      id: 200 + i,
      name: `Osoba ${i + 1}`,
      role: "",
    }));
    instance._handleMessage(nameplatesMessage(twelve));
    assert.ok(
      instance.variableDefinitions.map((d) => d.variableId).includes("nameplate_slot_12_id"),
    );
    instance._handleMessage(variablesMessage({ nameplate_slot_12_name: "Osoba 12" }));
    assert.equal(instance.variables.get("nameplate_slot_12_name"), "Osoba 12");
    assert.equal(instance.actionDefinitions.stream_nameplate_slot.options[0].choices.length, 12);
  });

  test("a slot that leaves the list forgets its cached value", () => {
    const twelve = Array.from({ length: 12 }, (_, i) => ({
      id: 300 + i,
      name: `Osoba ${i + 1}`,
      role: "",
    }));
    const instance = instanceWith(twelve);
    const sent = [];
    instance.setVariableValues = (batch) => sent.push({ ...batch });
    instance._handleMessage(variablesMessage({ nameplate_slot_12_name: "Osoba 12" }));
    assert.equal(instance.variables.get("nameplate_slot_12_name"), "Osoba 12");

    // A plate is removed: slot 12 is no longer defined, and its value goes.
    instance._handleMessage(nameplatesMessage(twelve.slice(0, 11)));
    assert.equal(instance.variables.has("nameplate_slot_12_name"), false);

    // Re-added with the SAME text: Companion is sent the value again.
    instance._handleMessage(nameplatesMessage(twelve));
    const before = sent.length;
    instance._handleMessage(variablesMessage({ nameplate_slot_12_name: "Osoba 12" }));
    assert.equal(sent.length, before + 1, "not diffed away as unchanged");
    assert.deepEqual(sent[before], { nameplate_slot_12_name: "Osoba 12" });
  });

  test("presets: the grid first (song, then slots 1..11), then the per-plate presets", () => {
    const instance = instanceWith(PLATES);
    const keys = Object.keys(instance.presetDefinitions);
    assert.deepEqual(keys.slice(0, 12), [
      "nameplate_grid_song",
      ...Array.from({ length: 11 }, (_, i) => `nameplate_grid_slot_${i + 1}`),
    ]);
    for (const key of keys.slice(0, 12)) {
      assert.equal(instance.presetDefinitions[key].category, NAMEPLATE_GRID_CATEGORY);
    }
    // The #779 per-plate presets are kept.
    assert.ok(instance.presetDefinitions.nameplate_song);
    assert.ok(instance.presetDefinitions.nameplate_9);
    assert.ok(instance.presetDefinitions.nameplate_4);
    assert.equal(instance.presetDefinitions.nameplate_9.category, "Menovky");
  });
});
