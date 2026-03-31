import { test, expect } from "@playwright/test";
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

test("POST /stage/clear empties the stage display", async ({ request }) => {
  // First, set up a presentation and trigger a slide
  const libResp = await request.post(
    new URL("/libraries", baseURL).toString(),
    { data: { name: `Clear Lib ${Date.now()}` } },
  );
  expect(libResp.ok()).toBeTruthy();
  const library: { id: string } = await libResp.json();

  const presResp = await request.post(
    new URL(`/libraries/${library.id}/presentations`, baseURL).toString(),
    { data: { name: "Clear Song" } },
  );
  expect(presResp.ok()).toBeTruthy();
  const presPayload: {
    presentation: { id: string; slides: Array<{ id: string }> };
  } = await presResp.json();
  const presentationId = presPayload.presentation.id;
  const slideId = presPayload.presentation.slides[0].id;

  // Trigger the slide
  const triggerResp = await request.post(
    new URL("/stage/state", baseURL).toString(),
    {
      data: {
        presentationId,
        currentSlideId: slideId,
      },
    },
  );
  expect(triggerResp.status()).toBe(204);

  // Verify stage has content
  const snapshotBefore = await request.get(
    new URL("/stage/snapshot", baseURL).toString(),
  );
  expect(snapshotBefore.ok()).toBeTruthy();
  const before = await snapshotBefore.json();
  expect(before.current).toBeTruthy();

  // Clear the stage
  const clearResp = await request.post(
    new URL("/stage/clear", baseURL).toString(),
  );
  expect(clearResp.status()).toBe(204);

  // Verify stage is empty
  const snapshotAfter = await request.get(
    new URL("/stage/snapshot", baseURL).toString(),
  );
  expect(snapshotAfter.ok()).toBeTruthy();
  const after = await snapshotAfter.json();
  expect(after.current).toBeNull();
});

test("stage clear broadcasts to WebSocket clients", async ({
  request,
  page,
}) => {
  // Connect to stage display via browser
  await request.post(new URL("/stage/layout", baseURL).toString(), {
    data: { code: "worship-snv" },
  });
  await page.goto(new URL("/stage", baseURL).toString(), {
    waitUntil: "domcontentloaded",
  });

  // Wait for WASM to load and WebSocket connection
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await page.waitForFunction(
    () => window.__presenterStageConnectionState === "connected",
    { timeout: 30_000 },
  );

  // Create and trigger a slide
  const libResp = await request.post(
    new URL("/libraries", baseURL).toString(),
    { data: { name: `WS Clear Lib ${Date.now()}` } },
  );
  const library: { id: string } = await libResp.json();

  const presResp = await request.post(
    new URL(`/libraries/${library.id}/presentations`, baseURL).toString(),
    { data: { name: "WS Clear Song" } },
  );
  const presPayload: {
    presentation: { id: string; slides: Array<{ id: string }> };
  } = await presResp.json();

  // Update slide with visible content
  await request.patch(
    new URL(
      `/presentations/${presPayload.presentation.id}/slides/${presPayload.presentation.slides[0].id}`,
      baseURL,
    ).toString(),
    { data: { main: "Visible Text", translation: "", stage: "" } },
  );

  await request.post(new URL("/stage/state", baseURL).toString(), {
    data: {
      presentationId: presPayload.presentation.id,
      currentSlideId: presPayload.presentation.slides[0].id,
    },
  });

  // Wait for stage to show content
  await expect(page.locator(".stage__current-slide .stage__slide-text")).toContainText("Visible Text", {
    timeout: 10_000,
  });

  // Clear stage
  await request.post(new URL("/stage/clear", baseURL).toString());

  // Stage should update via WebSocket — current text should be empty
  await expect(page.locator(".stage__current-slide .stage__slide-text")).toHaveText("", {
    timeout: 10_000,
  });
});
