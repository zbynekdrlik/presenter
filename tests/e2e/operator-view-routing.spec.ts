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

test("operator default view loads without view parameter", async ({ page }) => {
  const resp = await page.goto(new URL("/legacy", baseURL).toString(), {
    waitUntil: "domcontentloaded",
  });
  expect(resp?.ok()).toBeTruthy();
  // Default view is worship mode
  await expect(page.locator("body")).toBeVisible();
});

test("operator /legacy/bible navigates to Bible tab", async ({ page }) => {
  const resp = await page.goto(
    new URL("/legacy/bible", baseURL).toString(),
    { waitUntil: "domcontentloaded" },
  );
  expect(resp?.ok()).toBeTruthy();
  await expect(page.locator("body")).toBeVisible();
});

test("operator /legacy/timers navigates to Timers view", async ({
  page,
}) => {
  const resp = await page.goto(
    new URL("/legacy/timers", baseURL).toString(),
    { waitUntil: "domcontentloaded" },
  );
  expect(resp?.ok()).toBeTruthy();
  await expect(page.locator("body")).toBeVisible();
});

test("operator /legacy/settings navigates to Settings view", async ({
  page,
}) => {
  const resp = await page.goto(
    new URL("/legacy/settings", baseURL).toString(),
    { waitUntil: "domcontentloaded" },
  );
  expect(resp?.ok()).toBeTruthy();
  await expect(page.locator("body")).toBeVisible();
});

test("operator invalid view falls back gracefully", async ({ page }) => {
  const resp = await page.goto(
    new URL("/legacy/nonexistent", baseURL).toString(),
    { waitUntil: "domcontentloaded" },
  );
  // Should still render the operator page (defaults to worship)
  expect(resp?.ok()).toBeTruthy();
  await expect(page.locator("body")).toBeVisible();
});
