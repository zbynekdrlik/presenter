/**
 * WASM Operator Stage Monitor Tests
 *
 * Tests stage monitor functionality including connection tracking, baseline, and alerts.
 */

import { test, expect } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

let serverHandle: ServerHandle | undefined;
let baseURL: string;

test.describe.configure({ timeout: 180_000 });

test.beforeAll(async ({}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  baseURL = config.baseURL;
  await refreshDevData(config.dbUrl);
  serverHandle = await startTestServer(config.port, config.dbUrl);
});

test.afterAll(async () => {
  await stopServer(serverHandle);
});

async function initPage(page: import("@playwright/test").Page) {
  await page.goto(`${baseURL}/ui/operator`);
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
}

test.describe("WASM Operator Stage Monitor Tests", () => {
  test("stage monitor is visible in header", async ({ page }) => {
    await initPage(page);

    const stageMonitor = page.locator('[data-role="stage-monitor"]');
    await expect(stageMonitor).toBeVisible();
  });

  test("stage monitor shows connection count", async ({ page }) => {
    await initPage(page);

    const connectedCount = page.locator(
      '[data-role="stage-monitor-connected"]',
    );
    await expect(connectedCount).toBeVisible();

    const text = await connectedCount.textContent();
    // Should be a number
    expect(text).toMatch(/^\d+$/);
  });

  test("stage monitor shows issue count", async ({ page }) => {
    await initPage(page);

    const issuesCount = page.locator('[data-role="stage-monitor-issues"]');
    await expect(issuesCount).toBeVisible();

    const text = await issuesCount.textContent();
    // Should be a number
    expect(text).toMatch(/^\d+$/);
  });

  test("stage monitor has data attributes for counts", async ({ page }) => {
    await initPage(page);

    const stageMonitor = page.locator('[data-role="stage-monitor"]');

    // Should have data-connected attribute
    const connected = await stageMonitor.getAttribute("data-connected");
    expect(connected).toMatch(/^\d+$/);

    // Should have data-issues attribute
    const issues = await stageMonitor.getAttribute("data-issues");
    expect(issues).toMatch(/^\d+$/);
  });

  test("stage monitor click sets baseline", async ({ page }) => {
    await initPage(page);

    const stageMonitor = page.locator('[data-role="stage-monitor"]');
    await stageMonitor.click();

    // Wait for click to be processed
    await page
      .waitForFunction(
        () => {
          const monitor = document.querySelector('[data-role="stage-monitor"]');
          return monitor?.getAttribute("data-connected") !== null;
        },
        { timeout: 5_000 },
      )
      .catch(() => {});

    // Should not show error
    const errorToast = page.locator(
      '[data-role="toast"][data-variant="error"]',
    );
    await expect(errorToast).not.toBeVisible();
  });

  test("stage clear button is visible", async ({ page }) => {
    await initPage(page);

    const clearButton = page.locator('[data-role="clear-slide"]');
    await expect(clearButton).toBeVisible();
  });

  test("stage clear button clears display", async ({ page }) => {
    await initPage(page);

    const clearButton = page.locator('[data-role="clear-slide"]');
    await clearButton.click();

    // Should show info toast
    await page.waitForFunction(
      () => {
        const toast = document.querySelector('[data-role="toast"]');
        return toast && toast.textContent?.includes("cleared");
      },
      { timeout: 3_000 },
    );
  });

  test("stage status shows current/next slide preview", async ({ page }) => {
    await initPage(page);

    // Stage current preview should exist
    const stageCurrent = page.locator('[data-role="stage-current"]');
    await expect(stageCurrent).toBeVisible();

    // Stage next preview should exist
    const stageNext = page.locator('[data-role="stage-next"]');
    await expect(stageNext).toBeVisible();
  });
});
