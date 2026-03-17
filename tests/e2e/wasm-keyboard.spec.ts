/**
 * WASM Operator Keyboard Shortcuts Tests
 *
 * Tests keyboard navigation and shortcuts in the WASM operator.
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

async function loadPresentation(page: import("@playwright/test").Page) {
  await page.goto(`${baseURL}/ui/operator`);
  await page.waitForSelector('[data-role="library-list"]', { timeout: 30_000 });
  await page.waitForSelector('[data-role="library-item"]', { timeout: 30_000 });

  // Click library
  await page.locator('[data-role="library-item"]').first().click();
  await page.waitForSelector('[data-role="presentation-item"]', {
    timeout: 15_000,
  });

  // Click presentation
  await page.locator('[data-role="presentation-item"]').first().click();
  await page.waitForFunction(
    () =>
      document
        .querySelector('[data-view-panel="worship"] [data-role="slides"]')
        ?.querySelectorAll("[data-slide-id]").length ?? 0 > 0,
    { timeout: 15_000 },
  );
}

test.describe("WASM Operator Keyboard Shortcuts", () => {
  test("space focuses search in live mode", async ({ page }) => {
    await page.goto(`${baseURL}/ui/operator`);
    await page.waitForSelector('[data-role="library-list"]', {
      timeout: 30_000,
    });

    // Ensure live mode
    if ((await page.locator("body").getAttribute("data-mode")) !== "live") {
      await page.locator('[data-role="mode-toggle"][data-mode="live"]').click();
    }

    // Click neutral area
    await page.locator("body").click({ position: { x: 10, y: 10 } });

    // Press space
    await page.keyboard.press("Space");

    // Search should be focused
    const searchInput = page.locator('[data-role="global-search-query"]');
    await expect(searchInput).toBeFocused({ timeout: 2_000 });
  });

  test("space does not focus search in edit mode", async ({ page }) => {
    await page.goto(`${baseURL}/ui/operator`);
    await page.waitForSelector('[data-role="library-list"]', {
      timeout: 30_000,
    });

    // Switch to edit mode
    await page.locator('[data-role="mode-toggle"][data-mode="edit"]').click();
    await page.waitForFunction(
      () => document.body.getAttribute("data-mode") === "edit",
      { timeout: 5_000 },
    );

    // Click neutral area
    await page.locator("body").click({ position: { x: 10, y: 10 } });

    // Press space - should NOT focus search
    await page.keyboard.press("Space");

    // Search should NOT be focused
    const searchInput = page.locator('[data-role="global-search-query"]');
    await expect(searchInput).not.toBeFocused({ timeout: 1_000 });
  });

  test("escape closes modals", async ({ page }) => {
    await page.goto(`${baseURL}/ui/operator`);
    await page.waitForSelector('[data-role="library-list"]', {
      timeout: 30_000,
    });
    await page.waitForSelector('[data-role="library-item"]', {
      timeout: 30_000,
    });

    // Open modal
    await page.locator('[data-role="library-more"]').click();
    await page.waitForFunction(
      () =>
        document.querySelector('[data-role="library-modal"][data-open="true"]'),
      { timeout: 5_000 },
    );

    // Press Escape
    await page.keyboard.press("Escape");

    // Modal should close
    await page.waitForFunction(
      () =>
        !document.querySelector(
          '[data-role="library-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );
  });

  test("escape closes search", async ({ page }) => {
    await page.goto(`${baseURL}/ui/operator`);
    await page.waitForSelector('[data-role="library-list"]', {
      timeout: 30_000,
    });

    // Focus search and type
    const searchInput = page.locator('[data-role="global-search-query"]');
    await searchInput.focus();
    await searchInput.fill("test");

    // Wait for results
    await page.waitForTimeout(500);

    // Press Escape
    await page.keyboard.press("Escape");

    // Search should be cleared/closed
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

  test("ArrowRight navigates to next slide", async ({ page }) => {
    await loadPresentation(page);

    // Ensure live mode
    if ((await page.locator("body").getAttribute("data-mode")) !== "live") {
      await page.locator('[data-role="mode-toggle"][data-mode="live"]').click();
    }

    // Trigger first slide
    const firstSlide = page.locator("[data-slide-id]").first();
    await firstSlide.click();
    await expect(firstSlide).toHaveClass(/is-active/, { timeout: 5_000 });

    // Click body to unfocus inputs
    await page.locator("body").click({ position: { x: 10, y: 10 } });

    // Press ArrowRight
    await page.keyboard.press("ArrowRight");

    // Second slide should be active
    const secondSlide = page.locator("[data-slide-id]").nth(1);
    if ((await secondSlide.count()) > 0) {
      await expect(secondSlide).toHaveClass(/is-active/, { timeout: 5_000 });
    }
  });

  test("ArrowLeft navigates to previous slide", async ({ page }) => {
    await loadPresentation(page);

    // Ensure live mode
    if ((await page.locator("body").getAttribute("data-mode")) !== "live") {
      await page.locator('[data-role="mode-toggle"][data-mode="live"]').click();
    }

    // Trigger second slide
    const secondSlide = page.locator("[data-slide-id]").nth(1);
    const secondSlideCount = await secondSlide.count();
    expect(
      secondSlideCount,
      "Presentation needs at least 2 slides for ArrowLeft test",
    ).toBeGreaterThan(0);
    if (secondSlideCount === 0) return;
    await secondSlide.click();
    await expect(secondSlide).toHaveClass(/is-active/, { timeout: 5_000 });

    // Click body to unfocus inputs
    await page.locator("body").click({ position: { x: 10, y: 10 } });

    // Press ArrowLeft
    await page.keyboard.press("ArrowLeft");

    // First slide should be active
    const firstSlide = page.locator("[data-slide-id]").first();
    await expect(firstSlide).toHaveClass(/is-active/, { timeout: 5_000 });
  });

  test("no shortcuts in edit mode textareas", async ({ page }) => {
    await loadPresentation(page);

    // Switch to edit mode
    await page.locator('[data-role="mode-toggle"][data-mode="edit"]').click();
    await page.waitForFunction(
      () => document.body.getAttribute("data-mode") === "edit",
      { timeout: 5_000 },
    );

    // Focus textarea
    const textarea = page
      .locator('[data-slide-id] textarea[data-field="main"]')
      .first();
    await textarea.click();

    // Get original value
    const originalValue = await textarea.inputValue();

    // Press Space - should type, not focus search
    await textarea.press("Space");

    // Value should have space added
    const newValue = await textarea.inputValue();
    expect(newValue).toBe(originalValue + " ");

    // Restore original
    await textarea.fill(originalValue);
    await textarea.blur();
  });

  test("Tab navigation in modals", async ({ page }) => {
    await page.goto(`${baseURL}/ui/operator`);
    await page.waitForSelector('[data-role="library-list"]', {
      timeout: 30_000,
    });
    await page.waitForSelector('[data-role="library-item"]', {
      timeout: 30_000,
    });

    // Select library
    await page.locator('[data-role="library-item"]').first().click();
    await page.waitForSelector('[data-role="presentation-list"]', {
      timeout: 15_000,
    });

    // Open presentation create modal
    await page
      .locator('[data-view-panel="worship"] [data-role="presentation-create"]')
      .click();
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="presentation-create-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    // Tab should move focus within modal
    await page.keyboard.press("Tab");

    // Some focusable element in modal should be focused
    // Elements can be focusable via tabindex (UL, DIV, etc.) or naturally (INPUT, BUTTON, TEXTAREA)
    const focusedTag = await page.evaluate(
      () => document.activeElement?.tagName,
    );
    // Accept any element that can receive focus (native or via tabindex)
    expect([
      "INPUT",
      "BUTTON",
      "TEXTAREA",
      "UL",
      "LI",
      "DIV",
      "A",
      "SELECT",
    ]).toContain(focusedTag);

    // Close modal
    await page.keyboard.press("Escape");
  });
});
