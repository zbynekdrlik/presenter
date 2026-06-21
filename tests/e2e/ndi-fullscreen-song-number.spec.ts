import { test, expect, BrowserContext } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

// Regression for #436: the NDI fullscreen stage layout must NOT show the song
// number, while a normal layout (worship-snv) still does. The StatusBar's
// song-number <div data-role="song-number"> is suppressed via the
// hide_song_number prop that NdiFullscreen passes as true.

test.describe.configure({ timeout: 180_000 });

let server: ServerHandle | undefined;
let baseURL = "";
let dbUrl = "";
let port = 0;
let numberedPresentationId = "";
let numberedSlideId = "";

async function setLayout(context: BrowserContext, code: string) {
  const resp = await context.request.post(
    new URL("/stage/layout", baseURL).toString(),
    { data: { code } },
  );
  expect(resp.ok()).toBeTruthy();
}

async function triggerNumbered(context: BrowserContext) {
  const resp = await context.request.post(
    new URL("/stage/state", baseURL).toString(),
    {
      data: {
        presentationId: numberedPresentationId,
        currentSlideId: numberedSlideId,
        nextSlideId: null,
      },
    },
  );
  expect(resp.ok()).toBeTruthy();
}

async function openStage(context: BrowserContext, layoutCode: string) {
  const stagePage = await context.newPage();
  await stagePage.goto(new URL("/stage", baseURL).toString(), {
    waitUntil: "domcontentloaded",
  });
  await stagePage.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await stagePage.waitForSelector(`body[data-layout-code="${layoutCode}"]`, {
    timeout: 10_000,
  });
  await stagePage.waitForFunction(
    () => window.__presenterStageConnectionState === "connected",
    { timeout: 30_000 },
  );
  return stagePage;
}

test.beforeAll(async ({}, testInfo) => {
  const cfg = deriveTestConfig(testInfo);
  baseURL = cfg.baseURL;
  dbUrl = cfg.dbUrl;
  port = cfg.port;
  await refreshDevData(dbUrl);
  server = await startTestServer(port, dbUrl, cfg.oscPort);

  const libResp = await fetch(new URL("/libraries", baseURL).toString(), {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ name: "_E2E NDI Song Number Test" }),
  });
  const lib = await libResp.json();

  const presResp = await fetch(
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
  const presData = await presResp.json();
  numberedPresentationId = presData.presentation.id;
  numberedSlideId = presData.presentation.slides[0].id;
});

test.afterAll(async () => {
  await stopServer(server);
  server = undefined;
});

test("song number is HIDDEN on the ndi-fullscreen layout", async ({
  context,
}) => {
  // Trigger the numbered presentation FIRST and POSITIVELY prove its snapshot
  // (song_number=42) is delivered by asserting #042 renders on a worship-snv
  // page. This is the deterministic anchor — no blind sleep (CLAUDE.md:
  // "prefer retry-with-assert poll helpers over arbitrary sleeps"). Without
  // this proof, toHaveCount(0) on the NDI page could pass trivially if the
  // snapshot simply hadn't arrived yet, weakening the regression guard.
  await triggerNumbered(context);
  await setLayout(context, "worship-snv");
  const proofPage = await openStage(context, "worship-snv");
  await expect(proofPage.locator('[data-role="song-number"]')).toContainText(
    "#042",
    { timeout: 10_000 },
  );
  await proofPage.close();

  // Same persisted snapshot is now delivered to the ndi-fullscreen page.
  const consoleMessages: string[] = [];
  await setLayout(context, "ndi-fullscreen");
  const stagePage = await openStage(context, "ndi-fullscreen");
  stagePage.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  // The placeholder confirms the NDI layout is mounted (no active source here).
  await expect(stagePage.locator(".stage-ndi__placeholder")).toBeVisible({
    timeout: 10_000,
  });

  // The numbered snapshot is proven-delivered (above), yet the song number
  // must be absent on this layout.
  const songNumberEl = stagePage.locator('[data-role="song-number"]');
  await expect(songNumberEl).toHaveCount(0);

  expect(consoleMessages).toEqual([]);

  await stagePage.close();
});

test("song number is SHOWN on a normal layout (regression guard)", async ({
  context,
}) => {
  const consoleMessages: string[] = [];

  await setLayout(context, "worship-snv");
  const stagePage = await openStage(context, "worship-snv");
  stagePage.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  await triggerNumbered(context);

  const songNumberEl = stagePage.locator('[data-role="song-number"]');
  await expect(songNumberEl).toBeVisible({ timeout: 10_000 });
  await expect(songNumberEl).toContainText("#042");

  expect(consoleMessages).toEqual([]);

  await stagePage.close();
});

declare global {
  interface Window {
    __presenterStageConnectionState?: string;
  }
}
