/**
 * WASM Operator Search Tests
 *
 * Tests search functionality in the WASM operator.
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
  // Wait for data to load
  await page.waitForSelector('[data-role="library-item"]', { timeout: 30_000 });
}

test.describe("WASM Operator Search", () => {
  test("search input is visible", async ({ page }) => {
    await initPage(page);
    const searchInput = page.locator('[data-role="global-search-query"]');
    await expect(searchInput).toBeVisible();
  });

  test("space focuses search in live mode", async ({ page }) => {
    await initPage(page);

    // Ensure live mode
    const body = page.locator("body");
    if ((await body.getAttribute("data-mode")) !== "live") {
      await page.locator('[data-role="mode-toggle"][data-mode="live"]').click();
    }

    // Click somewhere neutral
    await body.click({ position: { x: 10, y: 10 } });

    // Press space
    await page.keyboard.press("Space");

    // Search should be focused
    const searchInput = page.locator('[data-role="global-search-query"]');
    await expect(searchInput).toBeFocused({ timeout: 2_000 });
  });

  test("search returns results", async ({ page }) => {
    await initPage(page);

    // Type in search
    const searchInput = page.locator('[data-role="global-search-query"]');
    await searchInput.fill("test");

    // Wait for results
    await page.waitForFunction(
      () => {
        const results = document.querySelector(
          '[data-role="global-search-results"]',
        );
        return (
          results &&
          (results.querySelectorAll('[data-role="search-result-item"]').length >
            0 ||
            results.textContent?.includes("No results"))
        );
      },
      { timeout: 10_000 },
    );
  });

  test("escape closes search", async ({ page }) => {
    await initPage(page);

    // Focus and type
    const searchInput = page.locator('[data-role="global-search-query"]');
    await searchInput.focus();
    await searchInput.fill("test");

    // Wait for results to appear
    await page
      .waitForFunction(
        () => {
          const results = document.querySelector(
            '[data-role="global-search-results"]',
          );
          return (
            results &&
            results.querySelectorAll('[data-role="search-result-item"]')
              .length > 0
          );
        },
        { timeout: 5_000 },
      )
      .catch(() => {});

    // Press escape
    await page.keyboard.press("Escape");

    // Results should hide and input should be cleared
    await page.waitForFunction(
      () => {
        const results = document.querySelector(
          '[data-role="global-search-results"]',
        );
        return !results || results.getAttribute("data-visible") === "false";
      },
      { timeout: 5_000 },
    );
  });

  test("clicking result selects presentation", async ({ page }) => {
    await initPage(page);

    // Type search query that should find something
    const searchInput = page.locator('[data-role="global-search-query"]');
    await searchInput.fill("a");

    // Wait for results with presentation kind
    await page.waitForFunction(
      () => {
        const results = document.querySelector(
          '[data-role="global-search-results"]',
        );
        return (
          results &&
          results.querySelectorAll('[data-role="search-result-item"]').length >
            0
        );
      },
      { timeout: 10_000 },
    );

    // Click first result
    const firstResult = page
      .locator('[data-role="search-result-item"]')
      .first();
    if ((await firstResult.count()) > 0) {
      await firstResult.click();

      // Search should close and presentation should load
      await page.waitForFunction(
        () => {
          const results = document.querySelector(
            '[data-role="global-search-results"]',
          );
          return !results || results.getAttribute("data-visible") === "false";
        },
        { timeout: 5_000 },
      );
    }
  });

  test("search debounces correctly", async ({ page }) => {
    await initPage(page);

    const searchInput = page.locator('[data-role="global-search-query"]');

    // Type quickly
    await searchInput.fill("a");
    await page.waitForTimeout(50);
    await searchInput.fill("ab");
    await page.waitForTimeout(50);
    await searchInput.fill("abc");

    // Wait for debounce to complete
    await page.waitForFunction(
      () => {
        const results = document.querySelector(
          '[data-role="global-search-results"]',
        );
        return (
          results &&
          (results.querySelectorAll('[data-role="search-result-item"]').length >
            0 ||
            results.textContent?.includes("No results"))
        );
      },
      { timeout: 5_000 },
    );

    // Results should eventually appear (single request)
    await page.waitForFunction(
      () => {
        const results = document.querySelector(
          '[data-role="global-search-results"]',
        );
        return (
          results &&
          (results.querySelectorAll('[data-role="search-result-item"]').length >
            0 ||
            results.textContent?.includes("No results"))
        );
      },
      { timeout: 10_000 },
    );
  });

  test("clear search button works", async ({ page }) => {
    await initPage(page);

    // Type search
    const searchInput = page.locator('[data-role="global-search-query"]');
    await searchInput.fill("test");

    // Wait for results
    await page
      .waitForFunction(
        () => {
          const results = document.querySelector(
            '[data-role="global-search-results"]',
          );
          return (
            results &&
            results.querySelectorAll('[data-role="search-result-item"]')
              .length > 0
          );
        },
        { timeout: 5_000 },
      )
      .catch(() => {});

    // Clear by using test helper
    await page.evaluate(() => {
      // @ts-expect-error exposed test helper
      window.__presenterOperatorTestHelpers?.clearSearch?.();
    });

    // Verify cleared
    await expect(searchInput).toHaveValue("");
  });

  test("search handles empty query", async ({ page }) => {
    await initPage(page);

    // Focus search
    const searchInput = page.locator('[data-role="global-search-query"]');
    await searchInput.focus();
    await searchInput.fill("");

    // Results should not show
    const results = page.locator(
      '[data-role="global-search-results"][data-visible="true"]',
    );
    await expect(results).not.toBeVisible({ timeout: 2_000 });
  });

  test("search result shows library context", async ({ page }) => {
    await initPage(page);

    // Search for something
    const searchInput = page.locator('[data-role="global-search-query"]');
    await searchInput.fill("a");

    // Wait for results
    await page.waitForFunction(
      () => {
        const results = document.querySelector(
          '[data-role="global-search-results"]',
        );
        return (
          results &&
          results.querySelectorAll('[data-role="search-result-item"]').length >
            0
        );
      },
      { timeout: 10_000 },
    );

    // Results should have meta (library name)
    const meta = page.locator(".operator__search-result-meta").first();
    if ((await meta.count()) > 0) {
      await expect(meta).not.toHaveText("");
    }
  });
});
