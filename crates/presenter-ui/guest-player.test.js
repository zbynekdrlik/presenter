// #999 L3: the guest-player AudioWorklet processor under node:test — the
// worklet globals (AudioWorkletProcessor, sampleRate, registerProcessor) are
// stubbed and the module is run in a vm context, as a browser would.
const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

const RATE = 24000;

function loadProcessor() {
  let registered;
  class AudioWorkletProcessor {
    constructor() {
      this.port = { onmessage: null, sent: [], postMessage(m) { this.sent.push(m); } };
    }
  }
  const context = vm.createContext({
    AudioWorkletProcessor,
    sampleRate: RATE,
    registerProcessor: (name, cls) => {
      registered = { name, cls };
    },
    Float32Array,
    Math,
  });
  vm.runInContext(fs.readFileSync(path.join(__dirname, "guest-player.js"), "utf8"), context);
  assert.equal(registered.name, "guest-player");
  const player = new registered.cls();
  const post = (data) => player.port.onmessage({ data });
  const render = () => {
    const out = new Float32Array(128);
    player.process([], [[out]]);
    return out;
  };
  return { player, post, render };
}

const chunk = (seconds, value) => new Float32Array(Math.round(seconds * RATE)).fill(value);

test("playback starts only once 150 ms are queued, then plays in order", () => {
  const { player, post, render } = loadProcessor();
  post(chunk(0.1, 0.5));
  assert.equal(render()[0], 0, "100 ms queued: still silent");
  post(chunk(0.1, 0.25));
  assert.equal(render()[0], 0.5);
  let played = 128;
  while (played < 0.1 * RATE) {
    render();
    played += 128;
  }
  assert.equal(player.played, played);
  assert.ok(render().includes(0.25), "the second chunk follows the first");
});

test("a queue over 1 s drops back to about 150 ms, never the newest chunk", () => {
  const { player, post } = loadProcessor();
  for (let i = 0; i < 5; i += 1) post(chunk(0.25, i));
  // 1.25 s queued > 1 s: the oldest chunks go until < 150 ms + one chunk is left
  assert.equal(player.chunks.length, 1);
  assert.equal(player.chunks[0][0], 4, "the newest chunk stays");
  assert.equal(player.queued, 0.25 * RATE);
  assert.equal(player.dropped, RATE);
});

test("an underrun waits for 150 ms again and reset empties the queue", () => {
  const { player, post, render } = loadProcessor();
  post(chunk(0.2, 0.5));
  for (let i = 0; i < 40; i += 1) render();
  assert.equal(player.underruns, 1);
  assert.equal(player.started, false);
  post(chunk(0.05, 0.5));
  assert.equal(render()[0], 0, "50 ms after an underrun: silent");
  post("reset");
  assert.equal(player.queued, 0);
  assert.equal(player.chunks.length, 0);
});
