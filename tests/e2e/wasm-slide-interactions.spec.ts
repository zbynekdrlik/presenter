/**
 * WASM Operator Slide Interactions Tests
 *
 * Tests slide triggering, editing, focus management, and reordering in the WASM operator.
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

async function loadPresentationWithSlides(
  page: import("@playwright/test").Page,
) {
  await page.goto(`${baseURL}/ui/operator`);
  await page.waitForSelector('[data-role="library-list"]', { timeout: 30_000 });

  // Wait for libraries to load
  await page.waitForSelector('[data-role="library-item"]', { timeout: 30_000 });

  // Click first library
  await page.locator('[data-role="library-item"]').first().click();

  // Wait for presentations
  await page.waitForSelector('[data-role="presentation-item"]', {
    timeout: 15_000,
  });

  // Click first presentation
  await page.locator('[data-role="presentation-item"]').first().click();

  // Wait for slides to load
  await page.waitForFunction(
    () => {
      const slides = document.querySelector('[data-role="slides"]');
      return slides && slides.querySelectorAll("[data-slide-id]").length > 0;
    },
    { timeout: 15_000 },
  );
}

test.describe("WASM Operator Slide Interactions", () => {
  test("click slide triggers stage in live mode", async ({ page }) => {
    await loadPresentationWithSlides(page);

    // Ensure we're in live mode
    const body = page.locator("body");
    if ((await body.getAttribute("data-mode")) !== "live") {
      await page.locator('[data-role="mode-toggle"][data-mode="live"]').click();
      await page.waitForFunction(
        () => document.body.getAttribute("data-mode") === "live",
        { timeout: 5_000 },
      );
    }

    // Click the first slide
    const firstSlide = page.locator("[data-slide-id]").first();
    await firstSlide.click();

    // Verify slide becomes active
    await expect(firstSlide).toHaveClass(/is-active/, { timeout: 5_000 });
  });

  test("click anywhere on card triggers in live mode", async ({ page }) => {
    await loadPresentationWithSlides(page);

    // Ensure live mode
    if ((await page.locator("body").getAttribute("data-mode")) !== "live") {
      await page.locator('[data-role="mode-toggle"][data-mode="live"]').click();
    }

    // Click on the slide card body (not text)
    const slideCard = page.locator("[data-slide-id]").first();
    await slideCard.click({ position: { x: 5, y: 5 } });

    // Should still become active
    await expect(slideCard).toHaveClass(/is-active/, { timeout: 5_000 });
  });

  test("edit mode: click textarea focuses", async ({ page }) => {
    await loadPresentationWithSlides(page);

    // Switch to edit mode
    await page.locator('[data-role="mode-toggle"][data-mode="edit"]').click();
    await page.waitForFunction(
      () => document.body.getAttribute("data-mode") === "edit",
      { timeout: 5_000 },
    );

    // Click on textarea in first slide
    const textarea = page
      .locator('[data-slide-id] textarea[data-field="main"]')
      .first();
    await textarea.click();

    // Textarea should be focused
    await expect(textarea).toBeFocused({ timeout: 2_000 });
  });

  test("edit mode: blur saves content", async ({ page }) => {
    await loadPresentationWithSlides(page);

    // Switch to edit mode
    await page.locator('[data-role="mode-toggle"][data-mode="edit"]').click();
    await page.waitForFunction(
      () => document.body.getAttribute("data-mode") === "edit",
      { timeout: 5_000 },
    );

    // Get textarea and modify content
    const textarea = page
      .locator('[data-slide-id] textarea[data-field="main"]')
      .first();
    const originalValue = await textarea.inputValue();
    const testValue = originalValue + " E2E_TEST_MARKER";

    await textarea.fill(testValue);

    // Blur to trigger save
    await textarea.blur();
    await page.waitForTimeout(500); // Wait for API call

    // Verify value persists (reload page and check)
    await page.reload();
    await loadPresentationWithSlides(page);

    // Switch back to edit mode
    await page.locator('[data-role="mode-toggle"][data-mode="edit"]').click();
    await page.waitForFunction(
      () => document.body.getAttribute("data-mode") === "edit",
      { timeout: 5_000 },
    );

    const reloadedTextarea = page
      .locator('[data-slide-id] textarea[data-field="main"]')
      .first();
    await expect(reloadedTextarea).toHaveValue(testValue);

    // Clean up - restore original value
    await reloadedTextarea.fill(originalValue);
    await reloadedTextarea.blur();
  });

  test("edit mode: focused slide has is-focused class", async ({ page }) => {
    await loadPresentationWithSlides(page);

    // Switch to edit mode
    await page.locator('[data-role="mode-toggle"][data-mode="edit"]').click();
    await page.waitForFunction(
      () => document.body.getAttribute("data-mode") === "edit",
      { timeout: 5_000 },
    );

    // Click on first slide's textarea
    const textarea = page
      .locator('[data-slide-id] textarea[data-field="main"]')
      .first();
    await textarea.click();

    // Parent slide card should have is-focused class
    const slideCard = page.locator("[data-slide-id]").first();
    await expect(slideCard).toHaveClass(/is-focused/, { timeout: 2_000 });
  });

  test("arrow keys navigate slides in live mode", async ({ page }) => {
    await loadPresentationWithSlides(page);

    // Ensure live mode
    if ((await page.locator("body").getAttribute("data-mode")) !== "live") {
      await page.locator('[data-role="mode-toggle"][data-mode="live"]').click();
    }

    // Trigger first slide
    const firstSlide = page.locator("[data-slide-id]").first();
    await firstSlide.click();
    await expect(firstSlide).toHaveClass(/is-active/, { timeout: 5_000 });

    // Click body to ensure no input is focused
    await page.locator("body").click({ position: { x: 10, y: 10 } });

    // Press ArrowRight to move to next slide
    await page.keyboard.press("ArrowRight");

    // Second slide should now be active
    const secondSlide = page.locator("[data-slide-id]").nth(1);
    if ((await secondSlide.count()) > 0) {
      await expect(secondSlide).toHaveClass(/is-active/, { timeout: 5_000 });
    }
  });

  test("add slide button creates new slide", async ({ page }) => {
    await loadPresentationWithSlides(page);

    // Count current slides
    const initialCount = await page.locator("[data-slide-id]").count();

    // Click add slide button
    const addButton = page.locator('[data-role="add-slide"]');
    await addButton.click();

    // Wait for new slide to appear
    await page.waitForFunction(
      (initial) =>
        document.querySelectorAll("[data-slide-id]").length > initial,
      initialCount,
      { timeout: 10_000 },
    );

    const newCount = await page.locator("[data-slide-id]").count();
    expect(newCount).toBe(initialCount + 1);
  });

  test("slide duplicate creates copy", async ({ page }) => {
    await loadPresentationWithSlides(page);

    // Switch to edit mode
    await page.locator('[data-role="mode-toggle"][data-mode="edit"]').click();
    await page.waitForFunction(
      () => document.body.getAttribute("data-mode") === "edit",
      { timeout: 5_000 },
    );

    // Count current slides
    const initialCount = await page.locator("[data-slide-id]").count();

    // Click duplicate button on first slide
    const duplicateButton = page
      .locator('[data-slide-id] [data-action="duplicate"]')
      .first();
    await duplicateButton.click();

    // Wait for duplicate to appear
    await page.waitForFunction(
      (initial) =>
        document.querySelectorAll("[data-slide-id]").length > initial,
      initialCount,
      { timeout: 10_000 },
    );

    const newCount = await page.locator("[data-slide-id]").count();
    expect(newCount).toBe(initialCount + 1);
  });

  test("slide delete with confirmation", async ({ page }) => {
    await loadPresentationWithSlides(page);

    // First add a slide so we have something to delete
    const addButton = page.locator('[data-role="add-slide"]');
    await addButton.click();
    await page.waitForTimeout(1000);

    const initialCount = await page.locator("[data-slide-id]").count();

    // Switch to edit mode
    await page.locator('[data-role="mode-toggle"][data-mode="edit"]').click();
    await page.waitForFunction(
      () => document.body.getAttribute("data-mode") === "edit",
      { timeout: 5_000 },
    );

    // Set up dialog handler to accept confirmation
    page.once("dialog", async (dialog) => {
      expect(dialog.type()).toBe("confirm");
      await dialog.accept();
    });

    // Click delete button on last slide
    const deleteButton = page
      .locator('[data-slide-id] [data-action="delete"]')
      .last();
    await deleteButton.click();

    // Wait for slide to be removed
    await page.waitForFunction(
      (initial) =>
        document.querySelectorAll("[data-slide-id]").length < initial,
      initialCount,
      { timeout: 10_000 },
    );

    const newCount = await page.locator("[data-slide-id]").count();
    expect(newCount).toBe(initialCount - 1);
  });

  test("slide delete cancellation preserves slide", async ({ page }) => {
    await loadPresentationWithSlides(page);

    const initialCount = await page.locator("[data-slide-id]").count();

    // Switch to edit mode
    await page.locator('[data-role="mode-toggle"][data-mode="edit"]').click();
    await page.waitForFunction(
      () => document.body.getAttribute("data-mode") === "edit",
      { timeout: 5_000 },
    );

    // Set up dialog handler to dismiss (cancel)
    page.once("dialog", async (dialog) => {
      await dialog.dismiss();
    });

    // Click delete button
    const deleteButton = page
      .locator('[data-slide-id] [data-action="delete"]')
      .first();
    await deleteButton.click();

    // Wait a bit and verify count unchanged
    await page.waitForTimeout(500);
    const newCount = await page.locator("[data-slide-id]").count();
    expect(newCount).toBe(initialCount);
  });

  test("clear slide button works", async ({ page }) => {
    await loadPresentationWithSlides(page);

    // Trigger a slide first
    const firstSlide = page.locator("[data-slide-id]").first();
    await firstSlide.click();
    await expect(firstSlide).toHaveClass(/is-active/, { timeout: 5_000 });

    // Click clear button
    const clearButton = page.locator('[data-role="clear-slide"]');
    await clearButton.click();

    // No slide should be active now
    await page.waitForFunction(
      () => !document.querySelector("[data-slide-id].is-active"),
      { timeout: 5_000 },
    );
  });

  test("line limit warnings display", async ({ page }) => {
    await loadPresentationWithSlides(page);

    // Switch to edit mode
    await page.locator('[data-role="mode-toggle"][data-mode="edit"]').click();
    await page.waitForFunction(
      () => document.body.getAttribute("data-mode") === "edit",
      { timeout: 5_000 },
    );

    // Set a low line limit
    const lineLimitInput = page.locator('[data-role="line-limit"]');
    await lineLimitInput.fill("10");

    // Type a long line in the textarea
    const textarea = page
      .locator('[data-slide-id] textarea[data-field="main"]')
      .first();
    await textarea.fill("This is a very long line that exceeds the line limit");

    // Warning should appear
    const warning = page
      .locator('[data-role="slide-warning"][data-visible="true"]')
      .first();
    await expect(warning).toBeVisible({ timeout: 2_000 });

    // Reset line limit
    await lineLimitInput.fill("50");
    await textarea.blur();
  });
});
