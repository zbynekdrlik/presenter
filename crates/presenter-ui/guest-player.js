// The guest page's speech player (#999 L3): an AudioWorklet processor, the one
// piece the platform requires in JavaScript (a worklet module cannot be WASM
// from our bundle). The page posts Float32Array chunks at the context's rate;
// this plays them in order from a small jitter buffer. Latency is THE
// requirement, so the buffer stays short: playback starts once TARGET seconds
// are queued, and a queue longer than MAX (a burst after a stall) drops its
// oldest audio back to TARGET. Every ~250 ms it reports what it played.
//
// #999 L6b: the model streams audio all the time, its pauses included, so the
// burst it sends when a session starts stays queued for the whole session
// (0.5–0.7 s on the R2 runs, up to MAX). While playing, a near-silent chunk
// (RMS at or below −45 dBFS, the model's pause) is cut so the queue holds at
// most TRIM: the standing delay is shed in the pauses and never in speech.
// TRIM 0.65 s = the lowest that added no gap in speech when the R2 output
// arrival times were replayed through these rules, also with ±80 ms of
// mobile-data jitter (0.55 s: 3–5 gaps in 5 min, 0.45 s: 24).
const TARGET = 0.15;
const MAX = 1.0;
const TRIM = 0.65;
// −45 dBFS as mean square: 10^(−45/10).
const SILENT_MEAN_SQUARE = Math.pow(10, -4.5);

function silent(data) {
  let energy = 0;
  for (let i = 0; i < data.length; i += 1) energy += data[i] * data[i];
  return data.length > 0 && energy / data.length <= SILENT_MEAN_SQUARE;
}

class GuestPlayer extends AudioWorkletProcessor {
  constructor() {
    super();
    this.chunks = [];
    this.offset = 0;
    this.queued = 0;
    this.started = false;
    this.played = 0;
    this.dropped = 0;
    this.trimmed = 0;
    this.underruns = 0;
    this.sinceReport = 0;
    this.port.onmessage = (event) => {
      const data = event.data;
      if (data === "reset") {
        this.chunks = [];
        this.offset = 0;
        this.queued = 0;
        this.started = false;
        return;
      }
      let chunk = data;
      if (this.started && silent(chunk)) {
        const room = Math.max(0, Math.round(TRIM * sampleRate) - this.queued);
        if (room < chunk.length) {
          this.trimmed += chunk.length - room;
          if (room === 0) return;
          chunk = chunk.subarray(0, room);
        }
      }
      this.chunks.push(chunk);
      this.queued += chunk.length;
      if (this.queued > MAX * sampleRate) {
        // Behind by more than MAX: drop whole chunks from the front until
        // about TARGET is left (never the newest chunk).
        while (this.chunks.length > 1 && this.queued - (this.chunks[0].length - this.offset) >= TARGET * sampleRate) {
          const first = this.chunks.shift();
          this.queued -= first.length - this.offset;
          this.dropped += first.length - this.offset;
          this.offset = 0;
        }
      }
    };
  }

  process(_inputs, outputs) {
    const out = outputs[0][0];
    if (!this.started && this.queued >= TARGET * sampleRate) {
      this.started = true;
    }
    let i = 0;
    if (this.started) {
      while (i < out.length && this.chunks.length > 0) {
        const chunk = this.chunks[0];
        const n = Math.min(out.length - i, chunk.length - this.offset);
        out.set(chunk.subarray(this.offset, this.offset + n), i);
        i += n;
        this.offset += n;
        this.queued -= n;
        this.played += n;
        if (this.offset >= chunk.length) {
          this.chunks.shift();
          this.offset = 0;
        }
      }
      if (i < out.length) {
        this.started = false;
        this.underruns += 1;
      }
    }
    out.fill(0, i);
    for (let c = 1; c < outputs[0].length; c++) {
      outputs[0][c].set(out);
    }
    this.sinceReport += out.length;
    if (this.sinceReport >= sampleRate / 4) {
      this.sinceReport = 0;
      this.port.postMessage({ played: this.played, queued: this.queued, dropped: this.dropped, trimmed: this.trimmed, underruns: this.underruns });
    }
    return true;
  }
}

registerProcessor("guest-player", GuestPlayer);
