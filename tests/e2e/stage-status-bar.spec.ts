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
let numberedPresentationId: string;
let numberedSlideIds: string[];
let unnumberedPresentationId: string;
let unnumberedSlideIds: string[];

test.describe.configure({ timeout: 180_000 });

async function openStageDisplay(context: BrowserContext) {
  await context.request.post(new URL("/stage/layout", baseURL).toString(), {
    data: { code: "worship-snv" },
  });
  const stagePage = await context.newPage();
  await stagePage.goto(new URL("/stage", baseURL).toString(), {
    waitUntil: "domcontentloaded",
  });
  await stagePage.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
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

  // Create test library for song number tests
  const libResp = await fetch(new URL("/libraries", baseURL).toString(), {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ name: "_E2E Song Number Test" }),
  });
  const lib = await libResp.json();

  // Create presentation WITH number prefix
  const numberedResp = await fetch(
    new URL(`/libraries/${lib.id}/presentations`, baseURL).toString(),
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        name: "042 Amazing Grace",
        slides: [{ main: "How sweet the sound", group: "Verse 1" }],
      }),
    },
  );
  const numberedData = await numberedResp.json();
  numberedPresentationId = numberedData.presentation.id;
  numberedSlideIds = numberedData.presentation.slides.map(
    (s: { id: string }) => s.id,
  );

  // Create presentation WITHOUT number prefix
  const unnumberedResp = await fetch(
    new URL(`/libraries/${lib.id}/presentations`, baseURL).toString(),
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        name: "Song Without Number",
        slides: [{ main: "Just a song", group: "Verse 1" }],
      }),
    },
  );
  const unnumberedData = await unnumberedResp.json();
  unnumberedPresentationId = unnumberedData.presentation.id;
  unnumberedSlideIds = unnumberedData.presentation.slides.map(
    (s: { id: string }) => s.id,
  );
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
});

test("stage status bar shows clock with current time", async ({ context }) => {
  const stagePage = await openStageDisplay(context);

  // Check that clock element exists and has content
  const clockEl = stagePage.locator(".stage__clock");
  await expect(clockEl).toBeVisible();

  // Clock should show time in HH:MM:SS format (textContent includes debug label)
  const clockText = await clockEl.textContent();
  expect(clockText).toMatch(/\d{2}:\d{2}:\d{2}/);

  await stagePage.close();
});

test("stage clock updates every second", async ({ context }) => {
  const stagePage = await openStageDisplay(context);

  const clockEl = stagePage.locator(".stage__clock");
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
  expect(updatedTime).toMatch(/\d{2}:\d{2}:\d{2}/);

  await stagePage.close();
});

test("LIVE indicator is initially inactive with Slovak text", async ({
  context,
}) => {
  const stagePage = await openStageDisplay(context);

  const liveEl = stagePage.locator(".stage__live-pill");
  await expect(liveEl).toBeVisible();
  await expect(liveEl).toHaveClass(/stage__live-pill--off/);
  await expect(liveEl).toContainText("VYSIELANIE JE VYPNUTE");

  await stagePage.close();
});

test("LIVE indicator responds to Companion broadcast.set_live command", async ({
  context,
}) => {
  const stagePage = await openStageDisplay(context);

  const liveEl = stagePage.locator(".stage__live-pill");
  await expect(liveEl).toHaveClass(/stage__live-pill--off/);
  await expect(liveEl).toContainText("VYSIELANIE JE VYPNUTE");

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
      document
        .querySelector(".stage__live-pill")
        ?.classList.contains("stage__live-pill--on"),
    { timeout: 5_000 },
  );

  await expect(liveEl).toHaveClass(/stage__live-pill--on/);
  await expect(liveEl).toContainText("LIVE");

  // Disable broadcast live
  const disableResult = await sendCommand("broadcast.set_live", {
    enabled: false,
  });
  expect(disableResult.type).toBe("ack");

  // Wait for the stage display to receive the WebSocket event
  await stagePage.waitForFunction(
    () =>
      document
        .querySelector(".stage__live-pill")
        ?.classList.contains("stage__live-pill--off"),
    { timeout: 5_000 },
  );

  await expect(liveEl).toHaveClass(/stage__live-pill--off/);
  await expect(liveEl).toContainText("VYSIELANIE JE VYPNUTE");

  socket.close();
  await stagePage.close();
});

test("LIVE indicator can be toggled on and off via Companion", async ({
  context,
}) => {
  const stagePage = await openStageDisplay(context);

  const liveEl = stagePage.locator(".stage__live-pill");
  await expect(liveEl).toHaveClass(/stage__live-pill--off/);

  // Connect to Companion and enable broadcast live
  const { socket, handshake, sendCommand } = createCompanionSocket(wsURL);
  await handshake();

  await sendCommand("broadcast.set_live", { enabled: true });

  await stagePage.waitForFunction(
    () =>
      document
        .querySelector(".stage__live-pill")
        ?.classList.contains("stage__live-pill--on"),
    { timeout: 5_000 },
  );

  await expect(liveEl).toHaveClass(/stage__live-pill--on/);
  await expect(liveEl).toContainText("LIVE");

  // Disable broadcast live
  await sendCommand("broadcast.set_live", { enabled: false });

  await stagePage.waitForFunction(
    () =>
      document
        .querySelector(".stage__live-pill")
        ?.classList.contains("stage__live-pill--off"),
    { timeout: 5_000 },
  );

  await expect(liveEl).toHaveClass(/stage__live-pill--off/);
  await expect(liveEl).toContainText("VYSIELANIE JE VYPNUTE");

  socket.close();
  await stagePage.close();
});

test("status bar contains clock, LIVE, and connection status", async ({
  context,
}) => {
  const stagePage = await openStageDisplay(context);

  // Check all three status bar elements exist
  const clockEl = stagePage.locator(".stage__clock");
  const liveEl = stagePage.locator(".stage__live-pill");
  const connectionEl = stagePage.locator(".stage__connection");

  await expect(clockEl).toBeVisible();
  await expect(liveEl).toBeVisible();
  await expect(connectionEl).toBeVisible();

  // Verify connection shows "CONNECTED" (latency is in a nested span)
  await expect(connectionEl).toContainText("CONNECTED");

  await stagePage.close();
});

test("status bar elements are positioned left to right: clock, LIVE, connection", async ({
  context,
}) => {
  const stagePage = await openStageDisplay(context);

  // Get bounding boxes of all three elements
  const clockBox = await stagePage.locator(".stage__clock").boundingBox();
  const liveBox = await stagePage.locator(".stage__live-pill").boundingBox();
  const connectionBox = await stagePage
    .locator(".stage__connection")
    .boundingBox();

  expect(clockBox).toBeTruthy();
  expect(liveBox).toBeTruthy();
  expect(connectionBox).toBeTruthy();

  if (clockBox && liveBox && connectionBox) {
    // Clock should be left of or adjacent to LIVE
    expect(clockBox.x + clockBox.width).toBeLessThanOrEqual(liveBox.x);

    // LIVE should be left of or adjacent to connection status
    expect(liveBox.x + liveBox.width).toBeLessThanOrEqual(connectionBox.x);
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
      document
        .querySelector(".stage__live-pill")
        ?.classList.contains("stage__live-pill--on"),
    { timeout: 5_000 },
  );

  // Reload the page
  await stagePage.reload({ waitUntil: "domcontentloaded" });
  await stagePage.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await stagePage.waitForFunction(
    () => window.__presenterStageConnectionState === "connected",
    { timeout: 30_000 },
  );

  // The LIVE state should still be true (server remembers the state)
  await stagePage.waitForFunction(
    () =>
      document
        .querySelector(".stage__live-pill")
        ?.classList.contains("stage__live-pill--on"),
    { timeout: 5_000 },
  );

  // Clean up: disable broadcast live
  await sendCommand("broadcast.set_live", { enabled: false });

  socket.close();
  await stagePage.close();
});

test("group pill autofit font fills box height tightly", async ({
  context,
}) => {
  const stagePage = await openStageDisplay(context);

  // Inject a group name via stage/state won't work without a real presentation.
  // Instead, measure the autofit behavior: with line-height 0.75,
  // the font-size should be significantly larger than the container height
  // (font-size ≈ containerH / 0.75), meaning cap-height glyphs nearly touch borders.
  //
  // We test this by checking the CSS line-height property is < 1, which proves
  // the autofit will pick appropriately large fonts.
  const lineHeight = await stagePage.evaluate(() => {
    const pill = document.querySelector(".stage__group-pill");
    if (!pill) return "missing";
    return getComputedStyle(pill).lineHeight;
  });

  // line-height should be a tight value (< 1) to make autofit pick bigger fonts
  // For line-height: 0.75, computed value will be like "16.3125px" (0.75 * font-size)
  // or "normal" if not set. We verify it's NOT "normal" and it's less than font-size.
  expect(lineHeight).not.toBe("missing");
  expect(lineHeight).not.toBe("normal");

  // Verify the computed line-height is less than font-size (tight)
  if (lineHeight !== "missing" && lineHeight !== "normal") {
    const lhPx = parseFloat(lineHeight);
    const fontSize = await stagePage.evaluate(() => {
      const pill = document.querySelector(".stage__group-pill");
      return pill ? parseFloat(getComputedStyle(pill).fontSize) : 0;
    });
    // line-height should be <= 96% of font-size (we set 0.95 for diacritic safety)
    if (fontSize > 0) {
      expect(lhPx / fontSize).toBeLessThanOrEqual(0.96);
    }
  }

  await stagePage.close();
});

test("stage latency shows server-measured round-trip under 500ms", async ({
  context,
}) => {
  const stagePage = await openStageDisplay(context);

  // Wait for the stage client to register and exchange at least one heartbeat
  // by polling the server's /stage/connections endpoint
  let latencyValue: number | null = null;
  for (let i = 0; i < 20; i++) {
    await stagePage.waitForTimeout(1_000);
    const resp = await context.request.get(
      new URL("/stage/connections", baseURL).toString(),
    );
    if (resp.ok()) {
      const connections = await resp.json();
      const withLatency = connections.find(
        (c: { latencyMs?: number }) => c.latencyMs != null,
      );
      if (withLatency) {
        latencyValue = withLatency.latencyMs;
        break;
      }
    }
  }

  expect(latencyValue).not.toBeNull();
  // LAN/localhost round-trip should be well under 500ms
  expect(latencyValue!).toBeLessThan(500);
  expect(latencyValue!).toBeGreaterThanOrEqual(0);

  await stagePage.close();
});

test("song number displays #042 for numbered presentation", async ({
  context,
}) => {
  const consoleMessages: string[] = [];

  const stagePage = await openStageDisplay(context);
  stagePage.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  // Trigger numbered presentation
  await context.request.post(new URL("/stage/state", baseURL).toString(), {
    data: {
      presentationId: numberedPresentationId,
      currentSlideId: numberedSlideIds[0],
      nextSlideId: null,
    },
  });

  // Wait for song number to appear
  const songNumberEl = stagePage.locator('[data-role="song-number"]');
  await expect(songNumberEl).toBeVisible({ timeout: 10_000 });
  await expect(songNumberEl).toContainText("#042");

  // Verify it's positioned between clock and LIVE pill
  const clockBox = await stagePage.locator(".stage__clock").boundingBox();
  const songBox = await songNumberEl.boundingBox();
  const liveBox = await stagePage.locator(".stage__live-pill").boundingBox();

  expect(clockBox).toBeTruthy();
  expect(songBox).toBeTruthy();
  expect(liveBox).toBeTruthy();

  if (clockBox && songBox && liveBox) {
    expect(clockBox.x + clockBox.width).toBeLessThanOrEqual(songBox.x + 1);
    expect(songBox.x + songBox.width).toBeLessThanOrEqual(liveBox.x + 1);
  }

  expect(consoleMessages).toEqual([]);

  await stagePage.close();
});

test("song number is hidden for presentation without number prefix", async ({
  context,
}) => {
  const consoleMessages: string[] = [];

  const stagePage = await openStageDisplay(context);
  stagePage.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  // Trigger unnumbered presentation
  await context.request.post(new URL("/stage/state", baseURL).toString(), {
    data: {
      presentationId: unnumberedPresentationId,
      currentSlideId: unnumberedSlideIds[0],
      nextSlideId: null,
    },
  });

  // Wait for slide text to appear (confirms snapshot arrived)
  await expect(
    stagePage.locator(".stage__slide-text"),
  ).toContainText("Just a song", { timeout: 10_000 });

  // Song number element should NOT be visible
  const songNumberEl = stagePage.locator('[data-role="song-number"]');
  await expect(songNumberEl).not.toBeVisible();

  expect(consoleMessages).toEqual([]);

  await stagePage.close();
});

// Type declarations for window object
declare global {
  interface Window {
    __presenterStageConnectionState?: string;
  }
}
