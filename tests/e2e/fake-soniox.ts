// #852 C3-9: a scripted stand-in for Soniox's real-time WebSocket
// (`wss://stt-rt.soniox.com/transcribe-websocket`), for the E2E only — no spec
// ever reaches the real service. The test server is pointed at it through
// `PRESENTER_LISTEN_SONIOX_URL` (ws:// on loopback, port 0).
//
// What it records, per the plan's Task 9.4: the upgrade's `authorization` and
// `user-agent`, the first TEXT frame (the config JSON), the audio bytes (→ audio
// ms at 32 bytes per ms, 16 kHz s16 mono), every control frame, and how many
// WebSocket pings the client sent (Soniox does not answer them, so it must be 0).
//
// What it answers: the fixture's responses (`{"atAudioMs": n, "response": {...}}`
// per line) as the audio arrives. The fixture is replayed in a LOOP over the
// audio received since the fake (re)started, so a long send keeps producing new
// sentences. Token times are set by the fake to the connection's own audio ms (a
// real Soniox stream's times are per connection). `{"type":"finalize"}` turns the
// pending non-final tokens final and adds `<fin>`; an empty TEXT frame ends the
// stream with `{"finished": true}` and a close. `kill()` drops every socket and
// stops listening (an outage: connects are refused); `restart()` listens on the
// same port again.

import { readFileSync } from "node:fs";
import { createServer, type Server } from "node:http";
import type { AddressInfo } from "node:net";
import { WebSocketServer, type WebSocket, type RawData } from "ws";

type Token = { text: string; is_final?: boolean };
type Line = { atAudioMs: number; response: { tokens: Token[] } };

/** Audio bytes per ms of 16 kHz s16le mono. */
const BYTES_PER_MS = 32;

export type FakeSoniox = {
  /** `ws://127.0.0.1:<port>` for `PRESENTER_LISTEN_SONIOX_URL`. */
  url: string;
  authorizations: string[];
  userAgents: string[];
  /** The first TEXT frame of each connection, parsed. */
  configs: Record<string, unknown>[];
  /** Every later TEXT frame (controls); `""` = the end-of-stream frame. */
  controls: string[];
  /** Pings the CLIENT sent (must stay 0). */
  pings: number;
  connections: number;
  /** Audio received since the last (re)start, in ms. */
  audioMs(): number;
  kill(): Promise<void>;
  restart(): Promise<void>;
  stop(): Promise<void>;
};

type Connection = {
  socket: WebSocket;
  configured: boolean;
  audioBytes: number;
  pending: Token[];
};

export async function startFakeSoniox(fixtureFile: string): Promise<FakeSoniox> {
  const lines: Line[] = readFileSync(fixtureFile, "utf8")
    .split("\n")
    .filter((line) => line.trim() !== "")
    .map((line) => JSON.parse(line) as Line);
  if (lines.length === 0) throw new Error(`empty fixture ${fixtureFile}`);
  // One loop of the fixture lasts until its last line's audio ms, plus a gap.
  const period = lines[lines.length - 1].atAudioMs + 200;

  let http: Server = createServer();
  let wss: WebSocketServer | undefined;
  let port = 0;
  let audioBytesTotal = 0;
  let nextLine = 0;
  const live = new Set<Connection>();

  const fake: FakeSoniox = {
    url: "",
    authorizations: [],
    userAgents: [],
    configs: [],
    controls: [],
    pings: 0,
    connections: 0,
    audioMs: () => Math.floor(audioBytesTotal / BYTES_PER_MS),
    kill,
    restart,
    stop,
  };

  /** Fixture line `n` (counting over every loop) is due at this audio ms. */
  const dueAt = (n: number) =>
    Math.floor(n / lines.length) * period + lines[n % lines.length].atAudioMs;

  function send(conn: Connection, tokens: Token[], extra: Record<string, unknown> = {}): void {
    const ms = Math.floor(conn.audioBytes / BYTES_PER_MS);
    const timed = tokens.map((token) => ({
      text: token.text,
      start_ms: ms,
      end_ms: ms,
      confidence: 0.98,
      is_final: token.is_final === true,
      language: "en",
    }));
    conn.socket.send(
      JSON.stringify({ tokens: timed, final_audio_proc_ms: ms, total_audio_proc_ms: ms, ...extra }),
    );
  }

  function onAudio(conn: Connection, bytes: number): void {
    conn.audioBytes += bytes;
    audioBytesTotal += bytes;
    while (fake.audioMs() >= dueAt(nextLine)) {
      const tokens = lines[nextLine % lines.length].response.tokens;
      nextLine += 1;
      conn.pending = tokens.filter((token) => token.is_final !== true);
      send(conn, tokens);
    }
  }

  function onText(conn: Connection, text: string): void {
    if (!conn.configured) {
      conn.configured = true;
      fake.configs.push(JSON.parse(text) as Record<string, unknown>);
      return;
    }
    fake.controls.push(text);
    if (text === "") {
      send(conn, [], { finished: true });
      conn.socket.close(1000);
      return;
    }
    const control = JSON.parse(text) as { type?: string };
    if (control.type === "finalize") {
      const finals = conn.pending.map((token) => ({ text: token.text, is_final: true }));
      conn.pending = [];
      send(conn, [...finals, { text: "<fin>", is_final: true }]);
    }
  }

  function attach(): void {
    wss = new WebSocketServer({ server: http });
    wss.on("connection", (socket, request) => {
      fake.connections += 1;
      fake.authorizations.push(String(request.headers.authorization ?? ""));
      fake.userAgents.push(String(request.headers["user-agent"] ?? ""));
      const conn: Connection = { socket, configured: false, audioBytes: 0, pending: [] };
      live.add(conn);
      socket.on("ping", () => {
        fake.pings += 1;
      });
      socket.on("message", (data: RawData, isBinary: boolean) => {
        const buffer = Array.isArray(data) ? Buffer.concat(data) : Buffer.from(data as Buffer);
        if (isBinary) onAudio(conn, buffer.length);
        else onText(conn, buffer.toString("utf8"));
      });
      socket.on("close", () => live.delete(conn));
    });
  }

  async function listen(): Promise<void> {
    await new Promise<void>((resolve, reject) => {
      http.once("error", reject);
      http.listen(port, "127.0.0.1", () => {
        http.off("error", reject);
        resolve();
      });
    });
    port = (http.address() as AddressInfo).port;
    fake.url = `ws://127.0.0.1:${port}`;
    attach();
  }

  async function shutDown(): Promise<void> {
    for (const conn of live) conn.socket.terminate();
    live.clear();
    await new Promise<void>((resolve) => (wss ? wss.close(() => resolve()) : resolve()));
    await new Promise<void>((resolve) => http.close(() => resolve()));
  }

  async function kill(): Promise<void> {
    await shutDown();
  }

  async function restart(): Promise<void> {
    http = createServer();
    audioBytesTotal = 0;
    nextLine = 0;
    await listen();
  }

  async function stop(): Promise<void> {
    if (http.listening) await shutDown();
  }

  await listen();
  return fake;
}
