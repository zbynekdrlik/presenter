import { test, expect } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

test.describe.configure({ timeout: 180_000 });

let serverHandle: ServerHandle | undefined;
let baseURL: string;

test.beforeAll(async ({}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  baseURL = config.baseURL;
  await refreshDevData(config.dbUrl);
  serverHandle = await startTestServer(config.port, config.dbUrl, config.oscPort);
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
});

test("tablet network indicator renders LAN for direct fetch", async ({
  page,
  request,
}) => {
  const consoleMessages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  // Wait for server readiness
  await expect(async () => {
    const response = await request.get(
      new URL("/healthz", baseURL).toString(),
      { timeout: 120_000 },
    );
    expect(response.ok()).toBeTruthy();
  }).toPass({ timeout: 180_000 });

  await page.goto(new URL("/ui/tablet", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });

  // Network indicator pill should be visible with "LAN" text.
  // Test server has no CF headers and client is loopback → local (private-range fallback).
  const indicator = page.locator('[data-role="network-indicator"]');
  await expect(indicator).toBeVisible({ timeout: 10_000 });
  await expect(indicator).toHaveText("LAN");

  // Info button opens the info popover.
  const infoBtn = page.locator('[data-role="info-button"]');
  await expect(infoBtn).toBeVisible({ timeout: 5_000 });
  await infoBtn.click();

  const popover = page.locator('[data-role="info-popover"]');
  await expect(popover).toBeVisible({ timeout: 5_000 });
  await expect(popover).toContainText("Version");
  await expect(popover).toContainText("Host");
  await expect(popover).toContainText("Network");
  await expect(popover).toContainText("LAN");

  // Must have zero console errors/warnings throughout.
  expect(consoleMessages).toEqual([]);
});
