import { test, expect, type Page, type WebSocketRoute } from "@playwright/test";
import {
  attachConsoleErrorCollector,
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

/**
 * #793 — stage displays ended up on DIFFERENT layouts after an event's WS
 * reset bursts (SNV 2026-09-27: SD3 + one other on `timer`, the rest on
 * `worship-snv`). The stage client dropped every `Stage` snapshot whose
 * layout differed from its own, so a lost `StageLayout` event (coalesced into
 * the single last-event slot, or published during a reconnect gap — the live
 * hub does not replay) left the display on the old layout forever.
 *
 * These specs drive the real `/stage` page through Playwright's WebSocket
 * route to reproduce both loss modes deterministically, and pin the plain
 * switch-then-trigger flow. Every test asserts a clean console.
 */

let serverHandle: ServerHandle | undefined;
let baseURL: string;
let presentationId: string;
let slideIds: string[];

test.describe.configure({ timeout: 120_000 });

test.beforeAll(async ({}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  baseURL = config.baseURL;
  await refreshDevData(config.dbUrl);
  serverHandle = await startTestServer(config.port, config.dbUrl, config.oscPort);

  const libResp = await fetch(new URL("/libraries", baseURL).toString(), {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ name: "_E2E Layout Sync #793" }),
  });
  const lib = await libResp.json();
  const presResp = await fetch(
    new URL(`/libraries/${lib.id}/presentations`, baseURL).toString(),
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        name: "Layout Sync",
        slides: [
          { main: "Prvá sloha\nlayout sync", group: "Verse" },
          { main: "Druhá sloha\nlayout sync", group: "Chorus" },
        ],
      }),
    },
  );
  const presData = await presResp.json();
  presentationId = presData.presentation.id;
  slideIds = presData.presentation.slides.map((s: { id: string }) => s.id);
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
});

async function setLayout(page: Page, code: string) {
  const resp = await page.request.post(new URL("/stage/layout", baseURL).toString(), {
    data: { code },
  });
  expect(resp.ok(), `POST /stage/layout ${code}`).toBeTruthy();
}

async function triggerSlide(page: Page, idx: number) {
  const resp = await page.request.post(new URL("/stage/state", baseURL).toString(), {
    data: {
      presentationId,
      currentSlideId: slideIds[idx],
      nextSlideId: slideIds[(idx + 1) % slideIds.length],
    },
  });
  expect(resp.ok(), "POST /stage/state").toBeTruthy();
}

/** Open `/stage` and wait for WASM + a live socket on the default layout. */
async function openStage(page: Page) {
  await page.goto(new URL("/stage", baseURL).toString(), {
    waitUntil: "domcontentloaded",
  });
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  await expect(page.locator("body")).toHaveAttribute("data-layout-code", "worship-snv", {
    timeout: 15_000,
  });
  await expect
    .poll(() => page.evaluate(() => (window as any).__presenterStageConnectionState), {
      timeout: 15_000,
    })
    .toBe("connected");
  // Record every layout the page renders, to prove a camera-crew snapshot
  // (always published alongside the selected layout) never flips the display.
  await page.evaluate(() => {
    const w = window as any;
    w.__layoutHistory = [document.body.getAttribute("data-layout-code")];
    new MutationObserver(() => {
      w.__layoutHistory.push(document.body.getAttribute("data-layout-code"));
    }).observe(document.body, { attributes: true, attributeFilter: ["data-layout-code"] });
  });
}

function frameType(message: string | Buffer): string | undefined {
  if (typeof message !== "string") return undefined;
  try {
    return JSON.parse(message).type;
  } catch {
    return undefined;
  }
}

test.beforeEach(async ({ request }) => {
  await request.post(new URL("/stage/layout", baseURL).toString(), {
    data: { code: "worship-snv" },
  });
});

test("#793 switch layout then immediately trigger — the stage ends on the new layout", async ({
  page,
}) => {
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  await openStage(page);

  await setLayout(page, "timer");
  await triggerSlide(page, 0);
  await expect(page.locator("body")).toHaveAttribute("data-layout-code", "timer", {
    timeout: 10_000,
  });

  await setLayout(page, "worship-snv");
  await triggerSlide(page, 1);
  await expect(page.locator("body")).toHaveAttribute("data-layout-code", "worship-snv", {
    timeout: 10_000,
  });
  await expect(page.locator("body")).toContainText("Druhá sloha", { timeout: 10_000 });

  expect(errors).toEqual([]);
});

test("#793 a LOST stage_layout frame is recovered from the following stage snapshot", async ({
  page,
}) => {
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  let dropLayoutFrames = false;
  await page.routeWebSocket(/\/live\/ws/, (ws: WebSocketRoute) => {
    const server = ws.connectToServer();
    server.onMessage((message) => {
      if (dropLayoutFrames && frameType(message) === "stage_layout") return;
      ws.send(message);
    });
  });
  await openStage(page);

  dropLayoutFrames = true;
  await setLayout(page, "timer");
  await triggerSlide(page, 0);

  await expect(page.locator("body")).toHaveAttribute("data-layout-code", "timer", {
    timeout: 10_000,
  });
  const history: string[] = await page.evaluate(() => (window as any).__layoutHistory);
  expect(history, "a camera-crew snapshot must never flip a /stage display").not.toContain(
    "camera-crew",
  );
  expect(await page.evaluate(() => (window as any).__presenterStageLayout)).toBe("timer");

  expect(errors).toEqual([]);
});

test("#793 a layout switched during a WS reset is resynced on reconnect", async ({ page }) => {
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  const routes: WebSocketRoute[] = [];
  let resetting: WebSocketRoute | null = null;
  await page.routeWebSocket(/\/live\/ws/, (ws: WebSocketRoute) => {
    routes.push(ws);
    const server = ws.connectToServer();
    server.onMessage((message) => {
      // The dying socket delivers nothing more: every frame published while
      // it resets is lost for good (the live hub does not replay events).
      if (ws === resetting) return;
      ws.send(message);
    });
  });
  await openStage(page);

  const dying = routes[routes.length - 1];
  resetting = dying;
  await setLayout(page, "timer");
  await triggerSlide(page, 0);
  await expect(page.locator("body")).toHaveAttribute("data-layout-code", "worship-snv");

  // The socket drops; the client reconnects on a fresh socket.
  const connectionsBefore = routes.length;
  await dying.close({ code: 4000, reason: "reset" });
  await expect.poll(() => routes.length, { timeout: 20_000 }).toBeGreaterThan(connectionsBefore);

  await expect(page.locator("body")).toHaveAttribute("data-layout-code", "timer", {
    timeout: 15_000,
  });

  expect(errors).toEqual([]);
});
