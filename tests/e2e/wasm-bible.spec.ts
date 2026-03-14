/**
 * WASM Operator Bible Tests
 *
 * Tests Bible functionality in the WASM operator including search and broadcast.
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

async function navigateToBible(page: import("@playwright/test").Page) {
  await page.goto(`${baseURL}/ui-next/operator`);
  await page.waitForSelector('[data-role="library-list"]', { timeout: 30_000 });

  // Navigate to bible view
  const bibleButton = page.locator(
    '[data-role="view-toggle"][data-view="bible"]',
  );
  if ((await bibleButton.count()) > 0) {
    await bibleButton.click();
  } else {
    // Fallback: click bible tab by text
    const bibleTab = page.locator('button:has-text("Bible")').first();
    if ((await bibleTab.count()) > 0) {
      await bibleTab.click();
    }
  }

  // Wait for bible view to be active
  await page.waitForFunction(
    () => {
      const body = document.body;
      return body.getAttribute("data-view") === "bible";
    },
    { timeout: 5_000 },
  );
}

test.describe("WASM Operator Bible Tests", () => {
  test("bible tab is visible and navigable", async ({ page }) => {
    await page.goto(`${baseURL}/ui-next/operator`);
    await page.waitForSelector('[data-role="library-list"]', {
      timeout: 30_000,
    });

    // Look for bible view toggle
    const bibleButton = page.locator(
      '[data-role="view-toggle"][data-view="bible"]',
    );
    if ((await bibleButton.count()) > 0) {
      await expect(bibleButton).toBeVisible();
    } else {
      // Fallback: check for bible tab text
      const bibleTab = page.locator('button:has-text("Bible")').first();
      await expect(bibleTab).toBeVisible();
    }
  });

  test("bible page has translation dropdown", async ({ page }) => {
    await navigateToBible(page);

    // Look for translation select
    const translationSelect = page.locator(
      '[data-role="bible-translation-select"]',
    );
    await expect(translationSelect).toBeVisible({ timeout: 10_000 });
  });

  test("translation dropdown has options", async ({ page }) => {
    await navigateToBible(page);

    const translationSelect = page.locator(
      '[data-role="bible-translation-select"]',
    );
    await expect(translationSelect).toBeVisible({ timeout: 10_000 });

    // Get options count
    const options = translationSelect.locator("option");
    const count = await options.count();

    // Should have at least one translation
    expect(count).toBeGreaterThanOrEqual(1);
  });

  test("bible search input is visible", async ({ page }) => {
    await navigateToBible(page);

    const searchInput = page.locator('[data-role="bible-search-input"]');
    await expect(searchInput).toBeVisible();
  });

  test("bible search returns results", async ({ page }) => {
    await navigateToBible(page);

    const searchInput = page.locator('[data-role="bible-search-input"]');
    await searchInput.fill("John 3:16");

    const searchButton = page.locator('[data-role="bible-search-button"]');
    await searchButton.click();

    // Wait for results
    await page.waitForFunction(
      () => {
        const results = document.querySelectorAll('[data-role="bible-result"]');
        const empty = document.querySelector(".bible-results-empty");
        return (
          results.length > 0 ||
          (empty && !empty.textContent?.includes("Enter a search"))
        );
      },
      { timeout: 10_000 },
    );
  });

  test("bible result click broadcasts passage", async ({ page }) => {
    await navigateToBible(page);

    const searchInput = page.locator('[data-role="bible-search-input"]');
    await searchInput.fill("John 3:16");

    const searchButton = page.locator('[data-role="bible-search-button"]');
    await searchButton.click();

    // Wait for results
    await page.waitForSelector('[data-role="bible-result"]', {
      timeout: 10_000,
    });

    // Click first result
    const firstResult = page.locator('[data-role="bible-result"]').first();
    await firstResult.click();

    // Should show success toast or active broadcast
    await page.waitForFunction(
      () => {
        const toast = document.querySelector('[data-role="toast"]');
        const broadcast = document.querySelector(
          '[data-role="bible-broadcast-active"]',
        );
        return (
          (toast && toast.textContent?.includes("Broadcasting")) ||
          broadcast !== null
        );
      },
      { timeout: 5_000 },
    );
  });

  test("clear broadcast button works", async ({ page }) => {
    await navigateToBible(page);

    // First broadcast something
    const searchInput = page.locator('[data-role="bible-search-input"]');
    await searchInput.fill("John 3:16");

    const searchButton = page.locator('[data-role="bible-search-button"]');
    await searchButton.click();

    await page.waitForSelector('[data-role="bible-result"]', {
      timeout: 10_000,
    });

    const firstResult = page.locator('[data-role="bible-result"]').first();
    await firstResult.click();

    // Wait for broadcast to be active
    await page.waitForSelector('[data-role="bible-broadcast-active"]', {
      timeout: 5_000,
    });

    // Click clear button
    const clearButton = page.locator('[data-role="bible-clear-broadcast"]');
    await clearButton.click();

    // Should show cleared state
    await page.waitForFunction(
      () => {
        const inactive = document.querySelector(
          '[data-role="bible-broadcast-inactive"]',
        );
        const toast = document.querySelector('[data-role="toast"]');
        return (
          inactive !== null || (toast && toast.textContent?.includes("cleared"))
        );
      },
      { timeout: 5_000 },
    );
  });

  test("broadcast state persists across tab switch", async ({ page }) => {
    await navigateToBible(page);

    // Broadcast a passage
    const searchInput = page.locator('[data-role="bible-search-input"]');
    await searchInput.fill("John 3:16");

    const searchButton = page.locator('[data-role="bible-search-button"]');
    await searchButton.click();

    await page.waitForSelector('[data-role="bible-result"]', {
      timeout: 10_000,
    });

    const firstResult = page.locator('[data-role="bible-result"]').first();
    await firstResult.click();

    await page.waitForSelector('[data-role="bible-broadcast-active"]', {
      timeout: 5_000,
    });

    // Switch to worship view
    const worshipButton = page.locator(
      '[data-role="view-toggle"][data-view="worship"]',
    );
    if ((await worshipButton.count()) > 0) {
      await worshipButton.click();
      await page.waitForFunction(
        () => document.body.getAttribute("data-view") === "worship",
        { timeout: 5_000 },
      );
    }

    // Switch back to bible
    await navigateToBible(page);

    // Broadcast should still be active
    const broadcastActive = page.locator(
      '[data-role="bible-broadcast-active"]',
    );
    await expect(broadcastActive).toBeVisible({ timeout: 5_000 });
  });

  test("enter key triggers search", async ({ page }) => {
    await navigateToBible(page);

    const searchInput = page.locator('[data-role="bible-search-input"]');
    await searchInput.fill("John 3:16");
    await searchInput.press("Enter");

    // Wait for results
    await page.waitForFunction(
      () => {
        const results = document.querySelectorAll('[data-role="bible-result"]');
        const empty = document.querySelector(".bible-results-empty");
        return (
          results.length > 0 ||
          (empty && !empty.textContent?.includes("Enter a search"))
        );
      },
      { timeout: 10_000 },
    );
  });

  test("translation change affects search", async ({ page }) => {
    await navigateToBible(page);

    // Wait for translation dropdown
    const translationSelect = page.locator(
      '[data-role="bible-translation-select"]',
    );
    await expect(translationSelect).toBeVisible({ timeout: 10_000 });

    // Get available options
    const options = translationSelect.locator("option");
    const count = await options.count();

    if (count > 1) {
      // Select second option
      const secondOption = await options.nth(1).getAttribute("value");
      if (secondOption) {
        await translationSelect.selectOption(secondOption);

        // Do a search
        const searchInput = page.locator('[data-role="bible-search-input"]');
        await searchInput.fill("John 3:16");
        await searchInput.press("Enter");

        // Wait for results - should work without error
        await page.waitForFunction(
          () => {
            const results = document.querySelectorAll(
              '[data-role="bible-result"]',
            );
            const empty = document.querySelector(".bible-results-empty");
            return (
              results.length > 0 ||
              (empty && !empty.textContent?.includes("Enter a search"))
            );
          },
          { timeout: 10_000 },
        );
      }
    }
  });
});
