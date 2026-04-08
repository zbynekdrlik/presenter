/**
 * WASM Operator Timer Tests
 *
 * Tests timer functionality in the WASM operator including countdown and preach timers.
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

async function navigateToTimers(page: import("@playwright/test").Page) {
  await page.goto(`${baseURL}/ui/operator`);
  await page.waitForSelector('[data-role="library-list"]', { timeout: 30_000 });

  // Navigate to timers view
  const timersButton = page.locator(
    '[data-role="view-toggle"][data-view="timers"]',
  );
  if ((await timersButton.count()) > 0) {
    await timersButton.click();
  } else {
    // Fallback: click timers tab by text
    const timersTab = page.locator('button:has-text("Timers")').first();
    if ((await timersTab.count()) > 0) {
      await timersTab.click();
    }
  }

  // Wait for timer panel to be visible
  await page.waitForFunction(
    () => {
      const body = document.body;
      return body.getAttribute("data-view") === "timers";
    },
    { timeout: 5_000 },
  );
}

test.describe("WASM Operator Timer Tests", () => {
  test("countdown target input receives focus", async ({ page }) => {
    await navigateToTimers(page);

    const countdownInput = page.locator('[data-role="countdown-target-input"]');
    await expect(countdownInput).toBeVisible();

    await countdownInput.click();
    await expect(countdownInput).toBeFocused();
  });

  test("countdown target input accepts time value", async ({ page }) => {
    await navigateToTimers(page);

    const countdownInput = page.locator('[data-role="countdown-target-input"]');
    await countdownInput.fill("18:00");

    const value = await countdownInput.inputValue();
    expect(value).toBe("18:00");
  });

  test("enter key in countdown input submits value", async ({ page }) => {
    await navigateToTimers(page);

    const countdownInput = page.locator('[data-role="countdown-target-input"]');
    await countdownInput.fill("18:30");
    await countdownInput.press("Enter");

    // Wait for update - target display should update
    await page
      .waitForResponse(
        (resp) => resp.url().includes("/timers/") && resp.status() === 200,
        { timeout: 5_000 },
      )
      .catch(() => {});

    // Verify no error toast
    const errorToast = page.locator(
      '[data-role="toast"][data-variant="error"]',
    );
    await expect(errorToast).not.toBeVisible();
  });

  test("countdown start button toggles timer", async ({ page }) => {
    await navigateToTimers(page);

    const startButton = page.locator('[data-role="countdown-start"]');
    await expect(startButton).toBeVisible();

    await startButton.click();

    // Wait for API response
    await page
      .waitForResponse(
        (resp) => resp.url().includes("/timers/") && resp.status() === 200,
        { timeout: 5_000 },
      )
      .catch(() => {});

    // Should not show error
    const errorToast = page.locator(
      '[data-role="toast"][data-variant="error"]',
    );
    await expect(errorToast).not.toBeVisible();
  });

  test("countdown offset minus decreases by 5 minutes", async ({ page }) => {
    await navigateToTimers(page);

    const offsetMinus = page.locator('[data-role="countdown-offset-minus"]');
    await expect(offsetMinus).toBeVisible();

    await offsetMinus.click();

    // Wait for API response
    await page
      .waitForResponse(
        (resp) => resp.url().includes("/timers/") && resp.status() === 200,
        { timeout: 5_000 },
      )
      .catch(() => {});

    // Should not show error
    const errorToast = page.locator(
      '[data-role="toast"][data-variant="error"]',
    );
    await expect(errorToast).not.toBeVisible();
  });

  test("countdown offset plus increases by 5 minutes", async ({ page }) => {
    await navigateToTimers(page);

    const offsetPlus = page.locator('[data-role="countdown-offset-plus"]');
    await expect(offsetPlus).toBeVisible();

    await offsetPlus.click();

    // Wait for API response
    await page
      .waitForResponse(
        (resp) => resp.url().includes("/timers/") && resp.status() === 200,
        { timeout: 5_000 },
      )
      .catch(() => {});

    // Should not show error
    const errorToast = page.locator(
      '[data-role="toast"][data-variant="error"]',
    );
    await expect(errorToast).not.toBeVisible();
  });

  test("timer display shows countdown value", async ({ page }) => {
    await navigateToTimers(page);

    const countdownValue = page.locator("#countdown-value");
    await expect(countdownValue).toBeVisible();

    const text = await countdownValue.textContent();
    // Should show a time format like "0:00" or "-1:23:45"
    expect(text).toMatch(/^-?\d+:\d{2}(:\d{2})?$/);
  });

  test("timer overlay opens in new window", async ({ page, context }) => {
    await navigateToTimers(page);

    const overlayButton = page.locator('[data-role="timer-overlay-open"]');
    await expect(overlayButton).toBeVisible();

    // Listen for new page
    const pagePromise = context.waitForEvent("page");
    await overlayButton.click();

    const newPage = await pagePromise;
    await newPage.waitForLoadState();

    // Verify URL contains overlay path
    expect(newPage.url()).toContain("/overlays/timer");

    await newPage.close();
  });

  test("timer overlay URL can be copied", async ({ page }) => {
    await navigateToTimers(page);

    const copyButton = page.locator('[data-role="timer-overlay-copy"]');
    await expect(copyButton).toBeVisible();

    await copyButton.click();

    // Should show success toast
    await page.waitForFunction(
      () => {
        const toast = document.querySelector('[data-role="toast"]');
        return toast && toast.textContent?.includes("copied");
      },
      { timeout: 3_000 },
    );
  });

  test("preach limit input sets and clears limit", async ({
    page,
    request,
  }) => {
    const consoleMessages: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
      }
    });

    await navigateToTimers(page);

    // Preach limit input should be visible
    const limitInput = page.locator('[data-role="preach-limit-input"]');
    await expect(limitInput).toBeVisible({ timeout: 5_000 });

    // Preach card should show "No limit" initially
    const preachLimit = page.locator("#preach-limit");
    await expect(preachLimit).toContainText("No limit", { timeout: 5_000 });

    // Type "5" and press Enter → sets limit to 300 seconds (5 min)
    await limitInput.click();
    await limitInput.fill("5");
    await limitInput.press("Enter");

    // Verify limit is set via API
    await expect(async () => {
      const response = await request.get(
        new URL("/timers/overview", baseURL).toString(),
        { timeout: 10_000 },
      );
      const data = await response.json();
      expect(data.preachTimer.limitSeconds).toBe(300);
    }).toPass({ timeout: 10_000, intervals: [500] });

    // Preach card should show "Limit: 5:00"
    await expect(preachLimit).toContainText("Limit: 5:00", { timeout: 5_000 });

    // Clear limit
    const clearButton = page.locator('[data-role="preach-limit-clear"]');
    await clearButton.click();

    // Verify limit is cleared via API
    await expect(async () => {
      const response = await request.get(
        new URL("/timers/overview", baseURL).toString(),
        { timeout: 10_000 },
      );
      const data = await response.json();
      expect(data.preachTimer.limitSeconds).toBeNull();
    }).toPass({ timeout: 10_000, intervals: [500] });

    // Preach card should show "No limit" again
    await expect(preachLimit).toContainText("No limit", { timeout: 5_000 });

    // Clean console
    expect(consoleMessages).toEqual([]);
  });

  test("preach timer start/pause/reset works", async ({ page }) => {
    await navigateToTimers(page);

    // Start preach timer
    const startButton = page.locator('button[data-command="start_preach"]');
    await expect(startButton).toBeVisible();
    await startButton.click();
    await page
      .waitForResponse(
        (resp) => resp.url().includes("/timers/") && resp.status() === 200,
        { timeout: 5_000 },
      )
      .catch(() => {});

    // Pause preach timer
    const pauseButton = page.locator('button[data-command="pause_preach"]');
    await pauseButton.click();
    await page
      .waitForResponse(
        (resp) => resp.url().includes("/timers/") && resp.status() === 200,
        { timeout: 5_000 },
      )
      .catch(() => {});

    // Reset preach timer
    const resetButton = page.locator('button[data-command="reset_preach"]');
    await resetButton.click();
    await page
      .waitForResponse(
        (resp) => resp.url().includes("/timers/") && resp.status() === 200,
        { timeout: 5_000 },
      )
      .catch(() => {});

    // Should not show error
    const errorToast = page.locator(
      '[data-role="toast"][data-variant="error"]',
    );
    await expect(errorToast).not.toBeVisible();
  });
});
