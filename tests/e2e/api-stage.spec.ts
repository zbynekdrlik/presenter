import { test, expect, BrowserContext } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

test.describe.configure({ timeout: 180_000 });

let server: ServerHandle | undefined;
let baseURL = "";

test.beforeAll(async ({}, testInfo) => {
  const cfg = deriveTestConfig(testInfo);
  baseURL = cfg.baseURL;
  await refreshDevData(cfg.dbUrl);
  server = await startTestServer(cfg.port, cfg.dbUrl, cfg.oscPort);
});

test.afterAll(async () => {
  await stopServer(server);
  server = undefined;
});

async function openApiStage(context: BrowserContext) {
  // Set global layout to "api"
  await context.request.post(new URL("/stage/layout", baseURL).toString(), {
    data: { code: "api" },
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

test("API stage push displays text and group colors", async ({
  context,
  request,
}) => {
  const consoleMessages: string[] = [];

  const stagePage = await openApiStage(context);
  stagePage.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  // Push data via API
  const putResp = await request.put(
    new URL("/api/stage", baseURL).toString(),
    {
      data: {
        currentText: "Haleluja, haleluja",
        nextText: "Spievajte Hospodinovi",
        currentGroup: "Vsetci",
        nextGroup: "Zeny",
        currentSong: "Haleluja",
        nextSong: "Spievajte",
      },
    },
  );
  expect(putResp.status()).toBe(204);

  // Verify current text
  const currentText = stagePage.locator(".stage__current-text");
  await expect(currentText).toContainText("Haleluja, haleluja", {
    timeout: 10_000,
  });

  // Verify next text
  const nextText = stagePage.locator(".stage__next-text");
  await expect(nextText).toContainText("Spievajte Hospodinovi", {
    timeout: 10_000,
  });

  // Verify current group pill with legacy color for "Vsetci" (#E08A3C = rgb(224, 138, 60))
  const currentGroupPill = stagePage.locator(
    ".stage__current-group .stage__group-pill",
  );
  await expect(currentGroupPill).toBeVisible({ timeout: 10_000 });
  await expect(currentGroupPill).toContainText("Vsetci");
  const bgColor = await currentGroupPill.evaluate(
    (el) => window.getComputedStyle(el).backgroundColor,
  );
  expect(bgColor).toBe("rgb(224, 138, 60)");

  // Verify text color is black (WCAG contrast for light background)
  const textColor = await currentGroupPill.evaluate(
    (el) => window.getComputedStyle(el).color,
  );
  expect(textColor).toBe("rgb(0, 0, 0)");

  // Verify next group pill
  const nextGroupPill = stagePage.locator(
    ".stage__next-group .stage__group-pill",
  );
  await expect(nextGroupPill).toBeVisible({ timeout: 10_000 });
  await expect(nextGroupPill).toContainText("Zeny");

  // Verify current song name
  const songName = stagePage.locator(".stage__current-song");
  await expect(songName).toContainText("Haleluja", { timeout: 10_000 });

  // Verify next song name
  const nextSongName = stagePage.locator(".stage__next-song");
  await expect(nextSongName).toContainText("Spievajte", { timeout: 10_000 });

  await stagePage.close();
  expect(consoleMessages).toEqual([]);
});

test("API stage push with empty state clears display", async ({
  context,
  request,
}) => {
  const consoleMessages: string[] = [];

  const stagePage = await openApiStage(context);
  stagePage.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  // Push data first
  await request.put(new URL("/api/stage", baseURL).toString(), {
    data: {
      currentText: "Some text",
      currentGroup: "Vsetci",
      currentSong: "Song",
    },
  });

  const currentText = stagePage.locator(".stage__current-text");
  await expect(currentText).toContainText("Some text", { timeout: 10_000 });

  // Push empty state
  const putResp = await request.put(
    new URL("/api/stage", baseURL).toString(),
    { data: {} },
  );
  expect(putResp.status()).toBe(204);

  // Wait for the display to clear — current text should become empty
  await expect(currentText).toHaveText("", { timeout: 10_000 });

  // Group pill should not be visible
  const currentGroupPill = stagePage.locator(
    ".stage__current-group .stage__group-pill",
  );
  await expect(currentGroupPill).not.toBeVisible({ timeout: 5_000 });

  await stagePage.close();
  expect(consoleMessages).toEqual([]);
});

test("API stage does not interfere with normal stage", async ({
  context,
  request,
}) => {
  const consoleMessages: string[] = [];

  // Create a library and presentation for normal stage
  const libResp = await request.post(
    new URL("/libraries", baseURL).toString(),
    { data: { name: `ApiIsolation Lib ${Date.now()}` } },
  );
  expect(libResp.ok()).toBeTruthy();
  const library: { id: string } = await libResp.json();

  const presResp = await request.post(
    new URL(`/libraries/${library.id}/presentations`, baseURL).toString(),
    { data: { name: "Normal Song" } },
  );
  expect(presResp.ok()).toBeTruthy();
  const presPayload: {
    presentation: { id: string; slides: Array<{ id: string }> };
  } = await presResp.json();
  const presentationId = presPayload.presentation.id;
  const slideId = presPayload.presentation.slides[0].id;

  // Set slide text
  await request.patch(
    new URL(
      `/presentations/${presentationId}/slides/${slideId}`,
      baseURL,
    ).toString(),
    {
      data: { main: "Normal slide text", translation: "", stage: "" },
    },
  );

  // Set normal stage layout and trigger slide
  await request.post(new URL("/stage/layout", baseURL).toString(), {
    data: { code: "worship-snv" },
  });
  await request.post(new URL("/stage/state", baseURL).toString(), {
    data: { presentationId, currentSlideId: slideId },
  });

  // Open normal stage page
  const normalPage = await context.newPage();
  normalPage.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });
  await normalPage.goto(new URL("/stage", baseURL).toString(), {
    waitUntil: "domcontentloaded",
  });
  await normalPage.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await normalPage.waitForFunction(
    () => window.__presenterStageConnectionState === "connected",
    { timeout: 30_000 },
  );

  // Verify normal stage shows normal text
  const normalText = normalPage.locator(".stage__current-text");
  await expect(normalText).toContainText("Normal slide text", {
    timeout: 10_000,
  });

  // Push API stage data — should NOT affect the normal stage
  await request.put(new URL("/api/stage", baseURL).toString(), {
    data: { currentText: "API override attempt" },
  });

  // Wait briefly and verify normal stage still shows normal text
  await normalPage.waitForTimeout(2_000);
  await expect(normalText).toContainText("Normal slide text");

  await normalPage.close();
  expect(consoleMessages).toEqual([]);
});
