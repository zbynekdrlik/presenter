// The guest page's speech player (#999 L3): an AudioWorklet processor, the one
// piece the platform requires in JavaScript (a worklet module cannot be WASM
// from our bundle). The page posts Float32Array chunks at the context's rate;
// this plays them in order from a small jitter buffer. Latency is THE
// requirement, so the buffer stays short: playback starts once TARGET seconds
// are queued, and a queue longer than MAX (a burst after a stall) drops its
// oldest audio back to TARGET. Every ~250 ms it reports what it played.
const TARGET = 0.15;
const MAX = 1.0;

class GuestPlayer extends AudioWorkletProcessor {
  constructor() {
    super();
    this.chunks = [];
    this.offset = 0;
    this.queued = 0;
    this.started = false;
    this.played = 0;
    this.dropped = 0;
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
      this.chunks.push(data);
      this.queued += data.length;
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
      this.port.postMessage({ played: this.played, queued: this.queued, dropped: this.dropped, underruns: this.underruns });
    }
    return true;
  }
}

registerProcessor("guest-player", GuestPlayer);
