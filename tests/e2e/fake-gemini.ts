// #999: a scripted stand-in for Gemini Live Translate's WebSocket
// (`BidiGenerateContent`), for the E2E only — no spec ever reaches Google. The
// test server is pointed at it through `PRESENTER_LISTEN_GEMINI_URL` (ws:// on
// loopback, port 0).
//
// What it records: the upgrade's `x-goog-api-key`, the setup JSON of every
// connection and the number of `realtimeInput` audio messages. What it
// answers, like the real service does (every frame BINARY, holding JSON):
// `setupComplete` after the setup; for every audio message 100 ms of 24 kHz
// s16le speech (a 440 Hz tone), the text fragment `Word<n> ` and a new
// resumption handle; every 5th message also an `inputTranscription`
// (`slovo<n> `, the Slovak original).

import { createServer, type Server } from "node:http";
import type { AddressInfo } from "node:net";
import { WebSocketServer, type RawData } from "ws";

export type FakeGemini = {
  /** `ws://127.0.0.1:<port>` for `PRESENTER_LISTEN_GEMINI_URL`. */
  url: string;
  keys: string[];
  setups: Record<string, unknown>[];
  audioMessages: number;
  stop(): Promise<void>;
};

/** 100 ms of a 440 Hz tone at 24 kHz, s16le, base64. */
function speechChunk(): string {
  const samples = 2_400;
  const pcm = Buffer.alloc(samples * 2);
  for (let i = 0; i < samples; i += 1) {
    pcm.writeInt16LE(Math.round(8_000 * Math.sin((2 * Math.PI * 440 * i) / 24_000)), i * 2);
  }
  return pcm.toString("base64");
}

export async function startFakeGemini(): Promise<FakeGemini> {
  const http: Server = createServer();
  const wss = new WebSocketServer({ server: http });
  const chunk = speechChunk();
  const fake: FakeGemini = { url: "", keys: [], setups: [], audioMessages: 0, stop };

  wss.on("connection", (socket, request) => {
    fake.keys.push(String(request.headers["x-goog-api-key"] ?? ""));
    let configured = false;
    let n = 0;
    const send = (value: unknown) => socket.send(Buffer.from(JSON.stringify(value)), { binary: true });
    socket.on("message", (data: RawData) => {
      const text = (Array.isArray(data) ? Buffer.concat(data) : Buffer.from(data as Buffer)).toString("utf8");
      const message = JSON.parse(text) as Record<string, unknown>;
      if (!configured) {
        configured = true;
        fake.setups.push(message);
        send({ setupComplete: {} });
        return;
      }
      if (!("realtimeInput" in message)) return;
      n += 1;
      fake.audioMessages += 1;
      send({ serverContent: { modelTurn: { parts: [{ inlineData: { mimeType: "audio/pcm;rate=24000", data: chunk } }] } } });
      send({ serverContent: { outputTranscription: { text: `Word${n} ` } } });
      if (n % 5 === 0) send({ serverContent: { inputTranscription: { text: `slovo${n} ` } } });
      send({ sessionResumptionUpdate: { newHandle: `h-${n}`, resumable: true } });
    });
  });

  await new Promise<void>((resolve, reject) => {
    http.once("error", reject);
    http.listen(0, "127.0.0.1", () => {
      http.off("error", reject);
      resolve();
    });
  });
  fake.url = `ws://127.0.0.1:${(http.address() as AddressInfo).port}`;

  async function stop(): Promise<void> {
    for (const client of wss.clients) client.terminate();
    await new Promise<void>((resolve) => wss.close(() => resolve()));
    await new Promise<void>((resolve) => http.close(() => resolve()));
  }

  return fake;
}
