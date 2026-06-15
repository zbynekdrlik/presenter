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
let dbUrl = "";
let port = 0;

test.beforeAll(async ({}, testInfo) => {
  const cfg = deriveTestConfig(testInfo);
  baseURL = cfg.baseURL;
  dbUrl = cfg.dbUrl;
  port = cfg.port;
  await refreshDevData(dbUrl);
  server = await startTestServer(port, dbUrl, cfg.oscPort);
});

test.afterAll(async () => {
  await stopServer(server);
  server = undefined;
});

test("ndi-fullscreen appears in stage displays list", async ({ request }) => {
  const resp = await request.get(
    new URL("/stage-displays", baseURL).toString(),
  );
  expect(resp.status()).toBe(200);
  const layouts = await resp.json();
  const ndi = layouts.find((l: any) => l.code === "ndi-fullscreen");
  expect(ndi).toBeDefined();
  expect(ndi.name).toBe("NDI FULLSCREEN");
});

test("stage page renders ndi-fullscreen layout", async ({ page }) => {
  // REGRESSION GUARD (2026-06-12): the retired lite auto-redirect sent
  // /stage to /stage/lite for this layout, silently dropping the stage
  // overlay blocks (clock, song number, status). The NDI layout MUST keep
  // serving the full WASM stage page.
  const consoleMessages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  // Set layout to ndi-fullscreen
  await page.request.post(
    new URL("/stage/layout", baseURL).toString(),
    { data: { code: "ndi-fullscreen" } },
  );

  await page.goto(new URL("/stage", baseURL).toString());
  await expect(page).toHaveURL(/\/stage$/);
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await page.waitForSelector('body[data-layout-code="ndi-fullscreen"]', {
    timeout: 10_000,
  });

  // Verify placeholder is shown (no active source in test env)
  const placeholder = page.locator(".stage-ndi__placeholder");
  await expect(placeholder).toBeVisible();
  await expect(placeholder).toContainText("No video source");

  expect(consoleMessages).toEqual([]);
});

test("mounts NdiVideo with WebRTC when source is active", async ({ page }) => {
  const consoleMessages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  // Need an active source for the <NdiVideo> to render (Show gate on ndi_active).
  const statusResp = await page.request.get(
    new URL("/ndi/status", baseURL).toString(),
  );
  const { available } = await statusResp.json();
  test.skip(!available, "NDI SDK not available");

  // Wait for finder
  await new Promise((r) => setTimeout(r, 6000));
  const sourcesResp = await page.request.get(
    new URL("/ndi/sources", baseURL).toString(),
  );
  const sources = await sourcesResp.json();
  test.skip(sources.length === 0, "No NDI sources on network");

  const createResp = await page.request.post(
    new URL("/integrations/video-sources", baseURL).toString(),
    { data: { label: "E2E WebRTC", ndiName: sources[0].name } },
  );
  const source = await createResp.json();
  await page.request.post(
    new URL(
      `/integrations/video-sources/${source.id}/activate`,
      baseURL,
    ).toString(),
  );

  await page.request.post(new URL("/stage/layout", baseURL).toString(), {
    data: { code: "ndi-fullscreen" },
  });

  await page.goto(new URL("/stage", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await page.waitForSelector('body[data-layout-code="ndi-fullscreen"]', {
    timeout: 10_000,
  });

  // Verify <video data-role="ndi-video"> mounts with correct source_id.
  const video = page.locator('[data-role="ndi-video"]');
  await expect(video).toHaveCount(1, { timeout: 10_000 });
  await expect(video).toHaveAttribute("data-source-id", source.id);

  // Legacy MJPEG element must NOT be present.
  await expect(page.locator('img[src*="/ndi/mjpeg"]')).toHaveCount(0);

  expect(
    consoleMessages.filter((m) => !m.includes("favicon")),
  ).toEqual([]);

  // Cleanup
  await page.request.post(
    new URL("/integrations/video-sources/deactivate", baseURL).toString(),
  );
  await page.request.delete(
    new URL(
      `/integrations/video-sources/${source.id}`,
      baseURL,
    ).toString(),
  );
});
