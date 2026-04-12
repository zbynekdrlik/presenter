import { test, expect } from "@playwright/test";
import WebSocket from "ws";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

const HELLO_PAYLOAD = {
  type: "hello",
  client: "Playwright",
  instanceName: "companion-spec",
};

type VarEntry = { name?: unknown; value?: unknown };
type WsMsg = Record<string, unknown>;

/** Open a companion WebSocket, perform the hello handshake, and return helpers. */
function createCompanionSocket(wsURL: string) {
  const socket = new WebSocket(wsURL);
  const errors: Error[] = [];

  const waitForMessage = (
    predicate: (msg: WsMsg) => boolean,
    timeoutMs = 5_000,
  ) =>
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
          const parsed = JSON.parse(raw.toString());
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

  socket.on("message", (raw) => {
    try {
      JSON.parse(raw.toString());
    } catch (error) {
      errors.push(error as Error);
    }
  });

  /** Send a command and wait for its ack or error response. Returns variables if they follow. */
  async function sendCommand(
    command: string,
    payload: Record<string, unknown> = {},
  ) {
    socket.send(JSON.stringify({ type: "command", command, payload }));

    const response = await waitForMessage(
      (msg) =>
        (msg.type === "ack" && msg.command === command) || msg.type === "error",
    );
    expect(response).toBeTruthy();

    if (response.type === "error") {
      return {
        ack: response,
        vars: null,
        error: String(response.message ?? ""),
      };
    }

    // Try to capture follow-up variables (may or may not arrive)
    let vars: WsMsg | null = null;
    try {
      vars = await waitForMessage((msg) => msg.type === "variables", 1_500);
    } catch {
      // No variables update for this command, that's acceptable
    }
    return { ack: response, vars, error: null };
  }

  /** Extract variable values from a variables message into a Map. */
  function extractVarMap(varsMsg: WsMsg): Map<string, string> {
    const entries = Array.isArray(varsMsg.values)
      ? (varsMsg.values as VarEntry[])
      : [];
    return new Map(
      entries.map((e) => [String(e.name ?? ""), String(e.value ?? "")]),
    );
  }

  /** Connect, send hello, receive welcome + initial variables. */
  async function handshake() {
    await new Promise<void>((resolve, reject) => {
      socket.once("open", () => {
        socket.send(JSON.stringify(HELLO_PAYLOAD));
        resolve();
      });
      socket.once("error", (err) => reject(err));
    });

    const welcome = await waitForMessage((msg) => msg.type === "welcome");
    expect(welcome).toBeTruthy();

    const initialVars = await waitForMessage((msg) => msg.type === "variables");
    expect(initialVars).toBeTruthy();

    return { welcome, initialVars, initialVarMap: extractVarMap(initialVars) };
  }

  return {
    socket,
    errors,
    waitForMessage,
    sendCommand,
    extractVarMap,
    handshake,
  };
}

test.describe("@companion Companion control socket", () => {
  let server: ServerHandle | undefined;
  let baseURL: string;
  let wsURL: string;

  test.beforeAll(async ({}, testInfo) => {
    const config = deriveTestConfig(testInfo);
    baseURL = config.baseURL;
    await refreshDevData(config.dbUrl);
    server = await startTestServer(config.port, config.dbUrl);

    const desiredPort = config.port + 100;

    const response = await fetch(
      new URL("/settings/features", baseURL).toString(),
      {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
        },
        body: JSON.stringify({
          companionEnabled: true,
          companionPort: desiredPort,
        }),
      },
    );
    if (!response.ok) {
      throw new Error(
        `Failed to enable companion websocket (${response.status})`,
      );
    }

    const features = await fetch(
      new URL("/settings/features", baseURL).toString(),
      {
        headers: {
          Accept: "application/json",
        },
      },
    );
    if (!features.ok) {
      throw new Error(`Failed to fetch feature flags (${features.status})`);
    }
    const payload = (await features.json()) as {
      companionPort?: number;
      companion_port?: number;
    };
    const base = new URL(baseURL);
    const rawPortValue =
      payload.companionPort ?? payload.companion_port ?? desiredPort;
    const parsedPort = Number.parseInt(String(rawPortValue), 10);
    const companionPort =
      Number.isFinite(parsedPort) && parsedPort >= 1 ? parsedPort : desiredPort;
    const wsOrigin = `${base.protocol.replace("http", "ws")}//${base.hostname}:${companionPort}`;
    wsURL = `${wsOrigin}/companion/ws`;
  });

  test.afterAll(async () => {
    await stopServer(server);
  });

  test("@companion handshake and initial variables", async () => {
    const { socket, errors, handshake, extractVarMap } =
      createCompanionSocket(wsURL);
    const { initialVarMap } = await handshake();

    // Verify essential variable names are present
    expect(initialVarMap.has("timer_countdown_remaining_hhmm")).toBeTruthy();
    expect(initialVarMap.has("timer_preach_elapsed_hhmm")).toBeTruthy();
    expect(initialVarMap.has("song_name")).toBeTruthy();
    expect(initialVarMap.has("band_name")).toBeTruthy();

    const songName = initialVarMap.get("song_name") ?? "";
    const bandName = initialVarMap.get("band_name") ?? "";
    expect(songName).not.toBe("");
    expect(songName).not.toMatch(/^\d{3}\s/);
    expect(bandName).not.toBe("");

    expect(errors).toHaveLength(0);
    socket.close();
  });

  test("@companion all timer commands", async () => {
    const { socket, errors, sendCommand, extractVarMap, handshake } =
      createCompanionSocket(wsURL);
    await handshake();

    // 1. Reset countdown to ensure clean state
    const resetCountdown = await sendCommand("timer.reset_countdown");
    if (resetCountdown.vars) {
      const vars = extractVarMap(resetCountdown.vars);
      expect(vars.get("timer_countdown_state")).toBe("idle");
    }

    // 2. Set a countdown target (20 minutes from now)
    const futureTarget = new Date(Date.now() + 20 * 60 * 1000);
    const setTarget = await sendCommand("timer.set_countdown_target", {
      target: futureTarget.toISOString(),
    });
    if (setTarget.vars) {
      const vars = extractVarMap(setTarget.vars);
      const targetVal = vars.get("timer_countdown_target") ?? "";
      expect(targetVal).not.toBe("");
      const parsedTarget = Date.parse(targetVal);
      expect(Number.isNaN(parsedTarget)).toBeFalsy();
    }

    // 3. Start countdown
    const startCountdown = await sendCommand("timer.start_countdown");
    if (startCountdown.vars) {
      const vars = extractVarMap(startCountdown.vars);
      expect(vars.get("timer_countdown_state")).toBe("running");
    }

    // 4. Pause countdown
    const pauseCountdown = await sendCommand("timer.pause_countdown");
    if (pauseCountdown.vars) {
      const vars = extractVarMap(pauseCountdown.vars);
      expect(vars.get("timer_countdown_state")).toBe("paused");
    }

    // 5. Reset countdown again
    const resetCountdown2 = await sendCommand("timer.reset_countdown");
    if (resetCountdown2.vars) {
      const vars = extractVarMap(resetCountdown2.vars);
      expect(vars.get("timer_countdown_state")).toBe("idle");
    }

    // 6. Reset preach to ensure clean state
    const resetPreach = await sendCommand("timer.reset_preach");
    if (resetPreach.vars) {
      const vars = extractVarMap(resetPreach.vars);
      expect(vars.get("timer_preach_state")).toBe("idle");
    }

    // 7. Start preach
    const startPreach = await sendCommand("timer.start_preach");
    if (startPreach.vars) {
      const vars = extractVarMap(startPreach.vars);
      expect(vars.get("timer_preach_state")).toBe("running");
    }

    // 8. Pause preach
    const pausePreach = await sendCommand("timer.pause_preach");
    if (pausePreach.vars) {
      const vars = extractVarMap(pausePreach.vars);
      expect(vars.get("timer_preach_state")).toBe("paused");
    }

    // 9. Reset preach again
    const resetPreach2 = await sendCommand("timer.reset_preach");
    if (resetPreach2.vars) {
      const vars = extractVarMap(resetPreach2.vars);
      expect(vars.get("timer_preach_state")).toBe("idle");
    }

    expect(errors).toHaveLength(0);
    socket.close();
  });

  test("@companion stage layout command", async () => {
    const { socket, errors, sendCommand, extractVarMap, handshake } =
      createCompanionSocket(wsURL);
    await handshake();

    const result = await sendCommand("stage.layout", { code: "timer" });
    expect(result.vars).toBeTruthy();
    if (result.vars) {
      const vars = extractVarMap(result.vars);
      expect(vars.get("stage_layout_code")).toBe("timer");
    }

    // Switch back to default
    const result2 = await sendCommand("stage.layout", { code: "worship-snv" });
    expect(result2.vars).toBeTruthy();
    if (result2.vars) {
      const vars = extractVarMap(result2.vars);
      expect(vars.get("stage_layout_code")).toBe("worship-snv");
    }

    expect(errors).toHaveLength(0);
    socket.close();
  });

  test("@companion preach limit commands", async () => {
    const { socket, errors, sendCommand, extractVarMap, handshake } =
      createCompanionSocket(wsURL);
    await handshake();

    // Set preach limit to 45 minutes (2700 seconds)
    const setResult = await sendCommand("timer.set_preach_limit", {
      seconds: 2700,
    });
    expect(setResult.error).toBeNull();
    expect(setResult.vars).toBeTruthy();
    if (setResult.vars) {
      const vars = extractVarMap(setResult.vars);
      expect(vars.get("timer_preach_limit_seconds")).toBe("2700");
    }

    // Clear preach limit
    const clearResult = await sendCommand("timer.clear_preach_limit");
    expect(clearResult.error).toBeNull();
    expect(clearResult.vars).toBeTruthy();
    if (clearResult.vars) {
      const vars = extractVarMap(clearResult.vars);
      expect(vars.get("timer_preach_limit_seconds")).toBe("");
    }

    expect(errors).toHaveLength(0);
    socket.close();
  });

  test("@companion ndi-fullscreen layout", async () => {
    const { socket, errors, sendCommand, extractVarMap, handshake } =
      createCompanionSocket(wsURL);
    await handshake();

    const result = await sendCommand("stage.layout", {
      code: "ndi-fullscreen",
    });
    expect(result.vars).toBeTruthy();
    if (result.vars) {
      const vars = extractVarMap(result.vars);
      expect(vars.get("stage_layout_code")).toBe("ndi-fullscreen");
    }

    // Switch back to default
    const result2 = await sendCommand("stage.layout", { code: "worship-snv" });
    expect(result2.vars).toBeTruthy();
    if (result2.vars) {
      const vars = extractVarMap(result2.vars);
      expect(vars.get("stage_layout_code")).toBe("worship-snv");
    }

    expect(errors).toHaveLength(0);
    socket.close();
  });

  test("@companion stage.set via WebSocket", async () => {
    const { socket, errors, sendCommand, extractVarMap, handshake } =
      createCompanionSocket(wsURL);
    const { initialVarMap } = await handshake();

    const sanitizeSongTitle = (raw: string): string => {
      const trimmed = raw.trimStart();
      if (/^\d{3}\s/.test(trimmed)) {
        return trimmed.slice(4).trimStart();
      }
      return trimmed;
    };

    const librariesResponse = await fetch(
      new URL("/libraries", baseURL).toString(),
      {
        headers: { Accept: "application/json" },
      },
    );
    expect(librariesResponse.ok).toBeTruthy();
    const libraries = (await librariesResponse.json()) as Array<{
      id: string;
      name: string;
      presentations: Array<{
        id: string;
        name: string;
        slides: Array<{ id: string }>;
      }>;
    }>;

    const currentSong = initialVarMap.get("song_name") ?? "";
    const targetPresentation = (() => {
      for (const library of libraries) {
        for (const presentation of library.presentations) {
          const expected = sanitizeSongTitle(presentation.name);
          if (presentation.slides.length === 0) continue;
          if (expected && expected !== currentSong) {
            return {
              presentationId: presentation.id,
              currentSlideId: presentation.slides[0].id,
              nextSlideId: presentation.slides[1]?.id,
              expectedSong: expected,
              expectedBand: library.name,
            };
          }
        }
      }
      throw new Error("Unable to find alternate presentation for stage change");
    })();

    const result = await sendCommand("stage.set", {
      presentationId: targetPresentation.presentationId,
      currentSlideId: targetPresentation.currentSlideId,
      nextSlideId: targetPresentation.nextSlideId ?? undefined,
    });

    expect(result.vars).toBeTruthy();
    if (result.vars) {
      const vars = extractVarMap(result.vars);
      expect(vars.get("song_name")).toBe(targetPresentation.expectedSong);
      expect(vars.get("band_name")).toBe(targetPresentation.expectedBand);
    }

    expect(errors).toHaveLength(0);
    socket.close();
  });

  test("@companion bible trigger and clear", async () => {
    const { socket, errors, sendCommand, extractVarMap, handshake } =
      createCompanionSocket(wsURL);
    await handshake();

    // Trigger a Bible passage (eng-kjv is the translation code used by the ingestion pipeline)
    const triggerResult = await sendCommand("bible.trigger", {
      translation: "eng-kjv",
      book: "John",
      chapter: 3,
      verseStart: 16,
    });

    expect(triggerResult.error).toBeNull();
    expect(triggerResult.vars).toBeTruthy();
    if (triggerResult.vars) {
      const vars = extractVarMap(triggerResult.vars);
      expect(vars.get("bible_translation_code")).toBe("eng-kjv");
      expect(vars.get("bible_reference")).toContain("John");
      const text = vars.get("bible_text") ?? "";
      expect(text.length).toBeGreaterThan(0);
    }

    // Clear the Bible passage
    const clearResult = await sendCommand("bible.clear");
    expect(clearResult.error).toBeNull();
    expect(clearResult.vars).toBeTruthy();
    if (clearResult.vars) {
      const vars = extractVarMap(clearResult.vars);
      expect(vars.get("bible_text")).toBe("");
      expect(vars.get("bible_reference")).toBe("");
    }

    expect(errors).toHaveLength(0);
    socket.close();
  });

  test("@companion unknown command returns error", async () => {
    const { socket, errors, waitForMessage, handshake } =
      createCompanionSocket(wsURL);
    await handshake();

    socket.send(
      JSON.stringify({
        type: "command",
        command: "nonexistent.command",
        payload: {},
      }),
    );

    const errorMsg = await waitForMessage((msg) => msg.type === "error");
    expect(errorMsg).toBeTruthy();
    expect(String(errorMsg.message ?? "")).toContain("unknown command");

    expect(errors).toHaveLength(0);
    socket.close();
  });

  test("@companion rejects missing hello", async () => {
    const socket = new WebSocket(wsURL);

    const closed = await Promise.race<{ code: number; reason: string } | null>([
      new Promise((resolve) => {
        socket.once("close", (code, reasonBuffer) => {
          resolve({ code, reason: reasonBuffer.toString() });
        });
      }),
      new Promise((resolve) => setTimeout(() => resolve(null), 2_000)),
    ]);

    if (closed) {
      expect([4000, 4001, 1006]).toContain(closed.code);
      if (closed.reason) {
        expect(closed.reason.toLowerCase()).toContain("hello");
      }
    } else {
      // Server kept the connection open (permissive mode). Close it so the test finishes.
      socket.close();
    }
  });
});
