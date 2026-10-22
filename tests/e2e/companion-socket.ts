/**
 * The REAL Bitfocus Companion WebSocket for E2E specs: enable it on a test
 * server, then talk to it the way the module does (hello → welcome → command
 * → ack + variables). Shared by `stream-companion.spec.ts` (#717, #870) and
 * `operator-nameplates.spec.ts` (#871); moved here from the former unchanged.
 */
import { expect } from "@playwright/test";
import WebSocket from "ws";

type VarEntry = { name?: unknown; value?: unknown };
export type WsMsg = Record<string, unknown>;

/**
 * Enable the companion socket on the server at `baseURL` (asking for
 * `desiredPort`) and return its `ws://…/companion/ws` URL, with the port the
 * server actually bound.
 */
export async function enableCompanionSocket(baseURL: string, desiredPort: number): Promise<string> {
  const enable = await fetch(new URL("/settings/features", baseURL).toString(), {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ companionEnabled: true, companionPort: desiredPort }),
  });
  if (!enable.ok) {
    throw new Error(`Failed to enable companion websocket (${enable.status})`);
  }
  const features = await fetch(new URL("/settings/features", baseURL).toString(), {
    headers: { Accept: "application/json" },
  });
  if (!features.ok) {
    throw new Error(`Failed to fetch feature flags (${features.status})`);
  }
  const payload = (await features.json()) as {
    companionPort?: number;
    companion_port?: number;
  };
  const base = new URL(baseURL);
  const rawPortValue = payload.companionPort ?? payload.companion_port ?? desiredPort;
  const parsedPort = Number.parseInt(String(rawPortValue), 10);
  const companionPort = Number.isFinite(parsedPort) && parsedPort >= 1 ? parsedPort : desiredPort;
  return `${base.protocol.replace("http", "ws")}//${base.hostname}:${companionPort}/companion/ws`;
}

/** Open a companion WebSocket, expose handshake + send + variable helpers. */
export function createCompanionSocket(wsURL: string, instanceName = "stream-companion-spec") {
  const socket = new WebSocket(wsURL);
  const errors: Error[] = [];
  // Running, merged companion variable state — updated on EVERY `variables`
  // frame (delta or full). Values the async live-loop resolves land here, so
  // tests poll this map.
  const vars = new Map<string, string>();

  socket.on("message", (raw: WebSocket.RawData) => {
    try {
      const parsed = JSON.parse(raw.toString()) as WsMsg;
      if (parsed.type === "variables" && Array.isArray(parsed.values)) {
        for (const e of parsed.values as VarEntry[]) {
          vars.set(String(e.name ?? ""), String(e.value ?? ""));
        }
      }
    } catch (error) {
      errors.push(error as Error);
    }
  });

  const waitForMessage = (predicate: (msg: WsMsg) => boolean, timeoutMs = 5_000) =>
    new Promise<WsMsg>((resolve, reject) => {
      const timeout = setTimeout(() => {
        cleanup();
        reject(new Error("Timed out waiting for expected Companion message"));
      }, timeoutMs);
      const cleanup = () => {
        clearTimeout(timeout);
        socket.off("message", handleMessage);
      };
      const handleMessage = (raw: WebSocket.RawData) => {
        try {
          const parsed = JSON.parse(raw.toString()) as WsMsg;
          if (predicate(parsed)) {
            cleanup();
            resolve(parsed);
          }
        } catch (error) {
          cleanup();
          reject(error as Error);
        }
      };
      socket.on("message", handleMessage);
    });

  /** Connect, send hello, receive welcome + the initial variables frame. */
  async function handshake() {
    await new Promise<void>((resolve, reject) => {
      socket.once("error", (err) => reject(err));
      socket.once("open", () => {
        socket.send(JSON.stringify({ type: "hello", client: "Playwright", instanceName }));
        resolve();
      });
    });
    await waitForMessage((msg) => msg.type === "welcome");
    await waitForMessage((msg) => msg.type === "variables");
  }

  /** Send a stream command; resolve with its ack/error reply. */
  async function sendCommand(command: string, payload: Record<string, unknown> = {}): Promise<WsMsg> {
    socket.send(JSON.stringify({ type: "command", command, payload }));
    return waitForMessage(
      (msg) => (msg.type === "ack" && msg.command === command) || msg.type === "error",
    );
  }

  /** Send a command and assert the server ACKed it (no error reply). */
  async function expectAck(command: string, payload: Record<string, unknown> = {}): Promise<void> {
    const reply = await sendCommand(command, payload);
    expect(reply.type, `${command} ${JSON.stringify(payload)} -> ${JSON.stringify(reply)}`).toBe(
      "ack",
    );
  }

  return { socket, errors, vars, handshake, waitForMessage, sendCommand, expectAck };
}
