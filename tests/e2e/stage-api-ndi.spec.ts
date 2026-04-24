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

test("api layout renders ApiStage wrapper with no NDI source active", async ({ page }) => {
  const consoleMessages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  // Ensure no video source is active
  await page.request.post(
    new URL("/integrations/video-sources/deactivate", baseURL).toString(),
  );

  // Switch stage to api layout
  await page.request.post(
    new URL("/stage/layout", baseURL).toString(),
    { data: { code: "api" } },
  );

  await page.goto(new URL("/stage", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await page.waitForSelector('body[data-layout-code="api"]', {
    timeout: 10_000,
  });

  // ApiStage wrapper must be in the DOM
  const wrapper = page.locator("div.stage-api");
  await expect(wrapper).toBeAttached();

  // No NDI image when no source is active
  const img = page.locator("img.stage-api__ndi");
  await expect(img).toHaveCount(0);

  // WorshipSnv content is nested inside the wrapper
  const slide = page.locator("div.stage-api .stage__current-slide");
  await expect(slide).toBeAttached();

  expect(consoleMessages).toEqual([]);
});
