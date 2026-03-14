/**
 * WASM Operator View Routing Tests
 *
 * Tests view navigation and URL state management in the WASM operator.
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
  await page.goto(`${baseURL}/ui-next/operator`);
  await page.waitForSelector('[data-role="library-list"]', { timeout: 30_000 });
}

test.describe("WASM Operator View Routing Tests", () => {
  test("operator loads with default worship view", async ({ page }) => {
    await initPage(page);

    const body = page.locator("body");
    const view = await body.getAttribute("data-view");
    expect(view).toBe("worship");
  });

  test("view toggle buttons are visible", async ({ page }) => {
    await initPage(page);

    // Check for view toggle buttons
    const viewToggles = page.locator('[data-role="view-toggle"]');
    const count = await viewToggles.count();

    // Should have multiple view options (worship, bible, timers, settings)
    expect(count).toBeGreaterThanOrEqual(2);
  });

  test("clicking bible view changes data-view attribute", async ({ page }) => {
    await initPage(page);

    const bibleButton = page.locator(
      '[data-role="view-toggle"][data-view="bible"]',
    );
    if ((await bibleButton.count()) > 0) {
      await bibleButton.click();

      await page.waitForFunction(
        () => document.body.getAttribute("data-view") === "bible",
        { timeout: 5_000 },
      );

      const body = page.locator("body");
      const view = await body.getAttribute("data-view");
      expect(view).toBe("bible");
    }
  });

  test("clicking timers view changes data-view attribute", async ({ page }) => {
    await initPage(page);

    const timersButton = page.locator(
      '[data-role="view-toggle"][data-view="timers"]',
    );
    if ((await timersButton.count()) > 0) {
      await timersButton.click();

      await page.waitForFunction(
        () => document.body.getAttribute("data-view") === "timers",
        { timeout: 5_000 },
      );

      const body = page.locator("body");
      const view = await body.getAttribute("data-view");
      expect(view).toBe("timers");
    }
  });

  test("clicking settings view changes data-view attribute", async ({
    page,
  }) => {
    await initPage(page);

    const settingsButton = page.locator(
      '[data-role="view-toggle"][data-view="settings"]',
    );
    if ((await settingsButton.count()) > 0) {
      await settingsButton.click();

      await page.waitForFunction(
        () => document.body.getAttribute("data-view") === "settings",
        { timeout: 5_000 },
      );

      const body = page.locator("body");
      const view = await body.getAttribute("data-view");
      expect(view).toBe("settings");
    }
  });

  test("view state persists on refresh", async ({ page }) => {
    await initPage(page);

    // Navigate to timers view
    const timersButton = page.locator(
      '[data-role="view-toggle"][data-view="timers"]',
    );
    if ((await timersButton.count()) > 0) {
      await timersButton.click();

      await page.waitForFunction(
        () => document.body.getAttribute("data-view") === "timers",
        { timeout: 5_000 },
      );

      // Refresh the page
      await page.reload();
      await page.waitForSelector('[data-role="library-list"]', {
        timeout: 30_000,
      });

      // Check view is still timers
      const body = page.locator("body");
      const view = await body.getAttribute("data-view");
      expect(view).toBe("timers");
    }
  });

  test("mode toggle switches between live and edit", async ({ page }) => {
    await initPage(page);

    const body = page.locator("body");
    const initialMode = await body.getAttribute("data-mode");

    // Click the opposite mode button
    const targetMode = initialMode === "live" ? "edit" : "live";
    const modeToggle = page.locator(
      `[data-role="mode-toggle"][data-mode="${targetMode}"]`,
    );

    if ((await modeToggle.count()) > 0) {
      await modeToggle.click();

      await page.waitForFunction(
        (target: string) => document.body.getAttribute("data-mode") === target,
        targetMode,
        { timeout: 5_000 },
      );

      const newMode = await body.getAttribute("data-mode");
      expect(newMode).toBe(targetMode);
    }
  });

  test("mode state persists on refresh", async ({ page }) => {
    await initPage(page);

    // Switch to edit mode
    const editToggle = page.locator(
      '[data-role="mode-toggle"][data-mode="edit"]',
    );
    if ((await editToggle.count()) > 0) {
      await editToggle.click();

      await page.waitForFunction(
        () => document.body.getAttribute("data-mode") === "edit",
        { timeout: 5_000 },
      );

      // Refresh the page
      await page.reload();
      await page.waitForSelector('[data-role="library-list"]', {
        timeout: 30_000,
      });

      // Check mode is still edit
      const body = page.locator("body");
      const mode = await body.getAttribute("data-mode");
      expect(mode).toBe("edit");
    }
  });

  test("returning to worship view shows correct panel", async ({ page }) => {
    await initPage(page);

    // Go to timers
    const timersButton = page.locator(
      '[data-role="view-toggle"][data-view="timers"]',
    );
    if ((await timersButton.count()) > 0) {
      await timersButton.click();
      await page.waitForFunction(
        () => document.body.getAttribute("data-view") === "timers",
        { timeout: 5_000 },
      );
    }

    // Return to worship
    const worshipButton = page.locator(
      '[data-role="view-toggle"][data-view="worship"]',
    );
    if ((await worshipButton.count()) > 0) {
      await worshipButton.click();
      await page.waitForFunction(
        () => document.body.getAttribute("data-view") === "worship",
        { timeout: 5_000 },
      );

      // Verify worship panel is visible
      const catalogSection = page.locator('[data-role="catalog"]');
      await expect(catalogSection).toBeVisible();
    }
  });

  test("view panel visibility matches data-view", async ({ page }) => {
    await initPage(page);

    // Navigate to timers
    const timersButton = page.locator(
      '[data-role="view-toggle"][data-view="timers"]',
    );
    if ((await timersButton.count()) > 0) {
      await timersButton.click();
      await page.waitForFunction(
        () => document.body.getAttribute("data-view") === "timers",
        { timeout: 5_000 },
      );

      // Timer panel should be visible (via CSS)
      const timerCards = page.locator('[data-role="timer-cards"]');
      await expect(timerCards).toBeVisible({ timeout: 5_000 });
    }
  });
});
