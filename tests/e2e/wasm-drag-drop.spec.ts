/**
 * WASM Operator Drag-Drop Tests
 *
 * Tests drag-and-drop functionality in the WASM operator.
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
  await page.waitForSelector('[data-role="library-item"]', { timeout: 30_000 });
}

async function loadPresentation(page: import("@playwright/test").Page) {
  await initPage(page);
  await page.locator('[data-role="library-item"]').first().click();
  await page.waitForSelector('[data-role="presentation-item"]', {
    timeout: 15_000,
  });
  await page.locator('[data-role="presentation-item"]').first().click();
  await page.waitForFunction(
    () =>
      document
        .querySelector('[data-role="slides"]')
        ?.querySelectorAll("[data-slide-id]").length ?? 0 > 0,
    { timeout: 15_000 },
  );
}

test.describe("WASM Operator Drag-Drop", () => {
  test("search result is draggable", async ({ page }) => {
    await initPage(page);

    // Type search query
    const searchInput = page.locator('[data-role="global-search-query"]');
    await searchInput.fill("a");

    // Wait for results
    await page.waitForFunction(
      () =>
        document
          .querySelector('[data-role="global-search-results"]')
          ?.querySelectorAll('[data-role="search-result-item"]').length ??
        0 > 0,
      { timeout: 10_000 },
    );

    // Verify results have draggable attribute
    const firstResult = page
      .locator('[data-role="search-result-item"]')
      .first();
    if ((await firstResult.count()) > 0) {
      await expect(firstResult).toHaveAttribute("draggable", "true");
    }
  });

  test("presentation is draggable from library", async ({ page }) => {
    await initPage(page);

    // Select library
    await page.locator('[data-role="library-item"]').first().click();
    await page.waitForSelector('[data-role="presentation-item"]', {
      timeout: 15_000,
    });

    // Verify presentation is draggable
    const firstPres = page.locator('[data-role="presentation-item"]').first();
    await expect(firstPres).toHaveAttribute("draggable", "true");
  });

  test("slide drag handle exists in edit mode", async ({ page }) => {
    await loadPresentation(page);

    // Switch to edit mode
    await page.locator('[data-role="mode-toggle"][data-mode="edit"]').click();
    await page.waitForFunction(
      () => document.body.getAttribute("data-mode") === "edit",
      { timeout: 5_000 },
    );

    // Verify drag handle exists
    const dragHandle = page.locator('[data-role="slide-drag-handle"]').first();
    await expect(dragHandle).toBeVisible();
  });

  test("drag handle is draggable", async ({ page }) => {
    await loadPresentation(page);

    // Switch to edit mode
    await page.locator('[data-role="mode-toggle"][data-mode="edit"]').click();
    await page.waitForFunction(
      () => document.body.getAttribute("data-mode") === "edit",
      { timeout: 5_000 },
    );

    // Verify drag handle has draggable attribute
    const dragHandle = page.locator('[data-role="slide-drag-handle"]').first();
    await expect(dragHandle).toHaveAttribute("draggable", "true");
  });
});
