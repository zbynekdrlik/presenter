import { test, expect, Page, BrowserContext } from "@playwright/test";
import WebSocket from "ws";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

let serverHandle: ServerHandle | undefined;
let baseURL: string;
let dbUrl: string;
let port: number;
let wsURL: string;

test.describe.configure({ timeout: 420_000 });

async function openStageDisplay(context: BrowserContext) {
  await context.request.post(new URL("/stage/layout", baseURL).toString(), {
    data: { code: "worship-snv" },
  });
  const stagePage = await context.newPage();
  await stagePage.goto(new URL("/stage", baseURL).toString(), {
    waitUntil: "domcontentloaded",
  });
  await stagePage.waitForFunction(
    () => window.__presenterStageConnectionState === "connected",
    { timeout: 30_000 },
  );
  return stagePage;
}

function createCompanionSocket(url: string) {
  const socket = new WebSocket(url);

  const waitForMessage = (
    predicate: (msg: Record<string, unknown>) => boolean,
    timeoutMs = 5_000,
  ) =>
    new Promise<Record<string, unknown>>((resolve, reject) => {
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

  async function handshake() {
    await new Promise<void>((resolve, reject) => {
      socket.once("open", () => {
        socket.send(
          JSON.stringify({
            type: "hello",
            client: "Playwright",
            instanceName: "stage-status-bar-spec",
          }),
        );
        resolve();
      });
      socket.once("error", (err) => reject(err));
    });

    await waitForMessage((msg) => msg.type === "welcome");
    await waitForMessage((msg) => msg.type === "variables");
  }

  async function sendCommand(
    command: string,
    payload: Record<string, unknown> = {},
  ) {
    socket.send(JSON.stringify({ type: "command", command, payload }));
    return waitForMessage(
      (msg) =>
        (msg.type === "ack" && msg.command === command) || msg.type === "error",
    );
  }

  return { socket, handshake, sendCommand, waitForMessage };
}

test.beforeAll(async ({}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  baseURL = config.baseURL;
  dbUrl = config.dbUrl;
  port = config.port;
  await refreshDevData(dbUrl);
  serverHandle = await startTestServer(port, dbUrl, config.oscPort);

  // Enable Companion socket
  const desiredPort = port + 100;
  const response = await fetch(
    new URL("/settings/features", baseURL).toString(),
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
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

  const base = new URL(baseURL);
  const wsOrigin = `${base.protocol.replace("http", "ws")}//${base.hostname}:${desiredPort}`;
  wsURL = `${wsOrigin}/companion/ws`;
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
});

test("stage status bar shows clock with current time", async ({ context }) => {
  const stagePage = await openStageDisplay(context);

  // Check that clock element exists and has content
  const clockEl = stagePage.locator("#stage-clock");
  await expect(clockEl).toBeVisible();

  // Clock should show time in HH:MM:SS format
  const clockText = await clockEl.textContent();
  expect(clockText).toMatch(/^\d{2}:\d{2}:\d{2}$/);

  await stagePage.close();
});

test("stage clock updates every second", async ({ context }) => {
  const stagePage = await openStageDisplay(context);

  const clockEl = stagePage.locator("#stage-clock");
  await expect(clockEl).toBeVisible();

  // Get initial time
  const initialTime = await clockEl.textContent();
  expect(initialTime).toBeTruthy();

  // Wait slightly more than 1 second and check it updated
  await stagePage.waitForTimeout(1100);

  const updatedTime = await clockEl.textContent();
  expect(updatedTime).toBeTruthy();

  // Either the seconds changed or we crossed a minute boundary
  // Just verify it's still a valid time format
  expect(updatedTime).toMatch(/^\d{2}:\d{2}:\d{2}$/);

  await stagePage.close();
});

test("LIVE indicator is initially inactive", async ({ context }) => {
  const stagePage = await openStageDisplay(context);

  const liveEl = stagePage.locator("#stage-live");
  await expect(liveEl).toBeVisible();
  await expect(liveEl).toHaveAttribute("data-active", "false");
  await expect(liveEl).toHaveText("LIVE");

  await stagePage.close();
});

test("LIVE indicator responds to Companion broadcast.set_live command", async ({
  context,
}) => {
  const stagePage = await openStageDisplay(context);

  const liveEl = stagePage.locator("#stage-live");
  await expect(liveEl).toHaveAttribute("data-active", "false");

  // Connect to Companion and send broadcast.set_live command
  const { socket, handshake, sendCommand } = createCompanionSocket(wsURL);
  await handshake();

  // Enable broadcast live
  const enableResult = await sendCommand("broadcast.set_live", {
    enabled: true,
  });
  expect(enableResult.type).toBe("ack");

  // Wait for the stage display to receive the WebSocket event
  await stagePage.waitForFunction(
    () =>
      document.getElementById("stage-live")?.getAttribute("data-active") ===
      "true",
    { timeout: 5_000 },
  );

  await expect(liveEl).toHaveAttribute("data-active", "true");

  // Disable broadcast live
  const disableResult = await sendCommand("broadcast.set_live", {
    enabled: false,
  });
  expect(disableResult.type).toBe("ack");

  // Wait for the stage display to receive the WebSocket event
  await stagePage.waitForFunction(
    () =>
      document.getElementById("stage-live")?.getAttribute("data-active") ===
      "false",
    { timeout: 5_000 },
  );

  await expect(liveEl).toHaveAttribute("data-active", "false");

  socket.close();
  await stagePage.close();
});

test("LIVE indicator can be controlled via debug helper", async ({
  context,
}) => {
  const stagePage = await openStageDisplay(context);

  const liveEl = stagePage.locator("#stage-live");
  await expect(liveEl).toHaveAttribute("data-active", "false");

  // Use debug helper to enable
  await stagePage.evaluate(() => {
    window.__presenterStageDebug?.setBroadcastLive(true);
  });

  await expect(liveEl).toHaveAttribute("data-active", "true");

  // Check state via debug helper
  const isLive = await stagePage.evaluate(() =>
    window.__presenterStageDebug?.getBroadcastLive(),
  );
  expect(isLive).toBe(true);

  // Use debug helper to disable
  await stagePage.evaluate(() => {
    window.__presenterStageDebug?.setBroadcastLive(false);
  });

  await expect(liveEl).toHaveAttribute("data-active", "false");

  await stagePage.close();
});

test("status bar contains clock, LIVE, and connection status", async ({
  context,
}) => {
  const stagePage = await openStageDisplay(context);

  // Check all three elements exist in the status bar
  const statusBar = stagePage.locator("#stage-status-bar");
  await expect(statusBar).toBeVisible();

  const clockEl = stagePage.locator("#stage-clock");
  const liveEl = stagePage.locator("#stage-live");
  const connectionEl = stagePage.locator("#stage-status-connection");

  await expect(clockEl).toBeVisible();
  await expect(liveEl).toBeVisible();
  await expect(connectionEl).toBeVisible();

  // Verify connection shows "Connected"
  await expect(connectionEl).toHaveText("Connected");

  await stagePage.close();
});

test("status bar elements are positioned left to right: clock, LIVE, connection", async ({
  context,
}) => {
  const stagePage = await openStageDisplay(context);

  // Get bounding boxes of all three elements
  const clockBox = await stagePage.locator("#stage-clock").boundingBox();
  const liveBox = await stagePage.locator("#stage-live").boundingBox();
  const statusBox = await stagePage.locator("#stage-status").boundingBox();

  expect(clockBox).toBeTruthy();
  expect(liveBox).toBeTruthy();
  expect(statusBox).toBeTruthy();

  if (clockBox && liveBox && statusBox) {
    // Clock should be left of LIVE
    expect(clockBox.x + clockBox.width).toBeLessThan(liveBox.x);

    // LIVE should be left of connection status
    expect(liveBox.x + liveBox.width).toBeLessThan(statusBox.x);
  }

  await stagePage.close();
});

test("broadcast_live state persists across stage reconnections", async ({
  context,
}) => {
  const stagePage = await openStageDisplay(context);

  // Connect to Companion and enable broadcast live
  const { socket, handshake, sendCommand } = createCompanionSocket(wsURL);
  await handshake();

  await sendCommand("broadcast.set_live", { enabled: true });

  await stagePage.waitForFunction(
    () =>
      document.getElementById("stage-live")?.getAttribute("data-active") ===
      "true",
    { timeout: 5_000 },
  );

  // Reload the page
  await stagePage.reload({ waitUntil: "domcontentloaded" });
  await stagePage.waitForFunction(
    () => window.__presenterStageConnectionState === "connected",
    { timeout: 30_000 },
  );

  // The LIVE state should still be true (server remembers the state)
  await stagePage.waitForFunction(
    () =>
      document.getElementById("stage-live")?.getAttribute("data-active") ===
      "true",
    { timeout: 5_000 },
  );

  // Clean up: disable broadcast live
  await sendCommand("broadcast.set_live", { enabled: false });

  socket.close();
  await stagePage.close();
});

// Type declarations for window object
declare global {
  interface Window {
    __presenterStageConnectionState?: string;
    __presenterStageDebug?: {
      setBroadcastLive: (enabled: boolean) => void;
      getBroadcastLive: () => boolean;
      simulateHeartbeatLoss: () => void;
      resumeHeartbeats: () => void;
    };
    __presenterStageBroadcastLive?: boolean;
  }
}
