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

  // #460: the header preview is a small LIVE iframe mirror of the real /stage
  // output (lyrics / Bible / timer AND video), not a text reconstruction.
  test("header preview is a live /stage?preview=1 iframe that renders", async ({
    page,
  }) => {
    // NOTE: we deliberately do NOT filter the `crbug.com/981419` wake-lock
    // warning here. The preview stage SKIPS the screen wake lock (stage.rs, in
    // preview mode) precisely so the embedded iframe never emits that warning —
    // so a clean console below ALSO positively proves the wake-lock skip works.
    // If preview-detection regressed and the iframe re-acquired the wake lock,
    // the warning would land in this listener and fail the assertion.
    const consoleMessages: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
      }
    });

    await initPage(page);

    // The preview frame wrapper + iframe exist and point at the live stage.
    const frameWrap = page.locator('[data-role="stage-preview-frame"]');
    await expect(frameWrap).toBeVisible();

    const iframe = frameWrap.locator("iframe.operator__stage-iframe");
    await expect(iframe).toHaveCount(1);
    const src = await iframe.getAttribute("src");
    expect(src).toBe("/stage?preview=1");

    // The embedded /stage actually renders — its stage-container mounts inside
    // the iframe (proves the live mirror loads, not just an empty frame).
    const stageContainer = page
      .frameLocator("iframe.operator__stage-iframe")
      .locator(".stage-container");
    await expect(stageContainer).toBeVisible({ timeout: 30_000 });

    // The embedded stage must not throw in the iframe AND must not emit the
    // wake-lock warning (proves the preview wake-lock skip).
    expect(consoleMessages).toEqual([]);
  });
});
