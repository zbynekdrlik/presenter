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
