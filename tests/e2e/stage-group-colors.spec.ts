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

test("group pill renders with legacy color and correct text contrast", async ({
  context,
  request,
}) => {
  const consoleMessages: string[] = [];

  // Create a library and presentation
  const libResp = await request.post(
    new URL("/libraries", baseURL).toString(),
    { data: { name: `GroupColor Lib ${Date.now()}` } },
  );
  expect(libResp.ok()).toBeTruthy();
  const library: { id: string } = await libResp.json();

  const presResp = await request.post(
    new URL(`/libraries/${library.id}/presentations`, baseURL).toString(),
    { data: { name: "Test Song" } },
  );
  expect(presResp.ok()).toBeTruthy();
  const presPayload: {
    presentation: { id: string; slides: Array<{ id: string }> };
  } = await presResp.json();
  const presentationId = presPayload.presentation.id;
  const slideId = presPayload.presentation.slides[0].id;

  // Update the slide to add the "Vsetci" group (legacy color #E08A3C)
  const patchResp = await request.patch(
    new URL(
      `/presentations/${presentationId}/slides/${slideId}`,
      baseURL,
    ).toString(),
    {
      data: {
        main: "Test lyrics",
        translation: "",
        stage: "",
        group: "Vsetci",
      },
    },
  );
  expect(patchResp.ok()).toBeTruthy();

  // Trigger the slide on stage
  await request.post(new URL("/stage/state", baseURL).toString(), {
    data: { presentationId, currentSlideId: slideId },
  });

  // Open stage display
  const stagePage = await openStageDisplay(context);
  stagePage.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  // Wait for the current group pill to appear with the correct text
  const groupPill = stagePage.locator(".stage__current-group .stage__group-pill");
  await expect(groupPill).toBeVisible({ timeout: 10_000 });
  await expect(groupPill).toContainText("Vsetci", { timeout: 10_000 });

  // Verify the legacy background color for "Vsetci" is #E08A3C = rgb(224, 138, 60)
  const bgColor = await groupPill.evaluate(
    (el) => window.getComputedStyle(el).backgroundColor,
  );
  expect(bgColor).toBe("rgb(224, 138, 60)");

  // Verify text color is black (correct contrast for a light background)
  const textColor = await groupPill.evaluate(
    (el) => window.getComputedStyle(el).color,
  );
  expect(textColor).toBe("rgb(0, 0, 0)");

  await stagePage.close();
  expect(consoleMessages).toEqual([]);
});

test("unknown group gets auto-generated color", async ({
  context,
  request,
}) => {
  const consoleMessages: string[] = [];

  // Create a library and presentation
  const libResp = await request.post(
    new URL("/libraries", baseURL).toString(),
    { data: { name: `AutoColor Lib ${Date.now()}` } },
  );
  expect(libResp.ok()).toBeTruthy();
  const library: { id: string } = await libResp.json();

  const presResp = await request.post(
    new URL(`/libraries/${library.id}/presentations`, baseURL).toString(),
    { data: { name: "Auto Color Song" } },
  );
  expect(presResp.ok()).toBeTruthy();
  const presPayload: {
    presentation: { id: string; slides: Array<{ id: string }> };
  } = await presResp.json();
  const presentationId = presPayload.presentation.id;
  const slideId = presPayload.presentation.slides[0].id;

  // Update the slide with a unique group name that has no legacy color
  const uniqueGroup = `UniqueGroup${Date.now()}`;
  const patchResp = await request.patch(
    new URL(
      `/presentations/${presentationId}/slides/${slideId}`,
      baseURL,
    ).toString(),
    {
      data: {
        main: "Auto color lyrics",
        translation: "",
        stage: "",
        group: uniqueGroup,
      },
    },
  );
  expect(patchResp.ok()).toBeTruthy();

  // Trigger the slide on stage
  await request.post(new URL("/stage/state", baseURL).toString(), {
    data: { presentationId, currentSlideId: slideId },
  });

  // Open stage display
  const stagePage = await openStageDisplay(context);
  stagePage.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  // Wait for the current group pill to appear with the unique group text
  const groupPill = stagePage.locator(".stage__current-group .stage__group-pill");
  await expect(groupPill).toBeVisible({ timeout: 10_000 });
  await expect(groupPill).toContainText(uniqueGroup, { timeout: 10_000 });

  // Verify a background color was assigned (not transparent/unset)
  const bgColor = await groupPill.evaluate(
    (el) => window.getComputedStyle(el).backgroundColor,
  );
  expect(bgColor).not.toBe("rgba(0, 0, 0, 0)");
  expect(bgColor).not.toBe("transparent");

  // Verify text color is either black or white (proper contrast)
  const textColor = await groupPill.evaluate(
    (el) => window.getComputedStyle(el).color,
  );
  expect(["rgb(0, 0, 0)", "rgb(255, 255, 255)"]).toContain(textColor);

  await stagePage.close();
  expect(consoleMessages).toEqual([]);
});
