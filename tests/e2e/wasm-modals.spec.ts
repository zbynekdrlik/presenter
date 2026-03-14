/**
 * WASM Operator Modals Tests
 *
 * Tests modal interactions in the WASM operator.
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

test.describe("WASM Operator Modals", () => {
  test("library modal opens and closes", async ({ page }) => {
    await initPage(page);

    // Click more button
    const moreButton = page.locator('[data-role="library-more"]');
    await moreButton.click();

    // Modal should open
    await page.waitForFunction(
      () =>
        document.querySelector('[data-role="library-modal"][data-open="true"]'),
      { timeout: 5_000 },
    );

    // Press Escape to close
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

  test("library modal: select library", async ({ page }) => {
    await initPage(page);

    // Open modal
    await page.locator('[data-role="library-more"]').click();
    await page.waitForFunction(
      () =>
        document.querySelector('[data-role="library-modal"][data-open="true"]'),
      { timeout: 5_000 },
    );

    // Click a library in the modal
    const modalLibrary = page
      .locator('[data-role="library-modal"] [data-role="library-item"]')
      .first();
    if ((await modalLibrary.count()) > 0) {
      await modalLibrary.click();

      // Modal should close and library should be selected
      await page.waitForFunction(
        () =>
          !document.querySelector(
            '[data-role="library-modal"][data-open="true"]',
          ),
        { timeout: 5_000 },
      );
    }
  });

  test("library modal: favorite toggle", async ({ page }) => {
    await initPage(page);

    // Open library modal
    await page.locator('[data-role="library-more"]').click();
    await page.waitForFunction(
      () =>
        document.querySelector('[data-role="library-modal"][data-open="true"]'),
      { timeout: 5_000 },
    );

    // Find a star button
    const starButton = page
      .locator('[data-role="library-modal"] [data-role="library-star"]')
      .first();
    if ((await starButton.count()) > 0) {
      // Get current state
      const wasFavorite =
        (await starButton.getAttribute("data-favorited")) === "true";

      // Click to toggle
      await starButton.click();

      // Wait for state to change
      await page.waitForFunction(
        (wasFav) => {
          const star = document.querySelector(
            '[data-role="library-modal"] [data-role="library-star"]',
          );
          return star && star.getAttribute("data-favorited") !== String(wasFav);
        },
        wasFavorite,
        { timeout: 5_000 },
      );
    }

    // Close modal
    await page.keyboard.press("Escape");
  });

  test("library edit modal: rename", async ({ page }) => {
    await initPage(page);

    // Click library more button (edit icon)
    const editButton = page.locator('[data-role="library-edit"]').first();
    const editButtonCount = await editButton.count();
    expect(
      editButtonCount,
      "No library edit button found for rename test",
    ).toBeGreaterThan(0);
    if (editButtonCount === 0) return;
    await editButton.click();

    // Wait for edit modal
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="library-edit-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    // Verify modal has name input
    const nameInput = page.locator('[data-role="library-edit-name"]');
    await expect(nameInput).toBeVisible();

    // Close without saving
    await page.keyboard.press("Escape");
  });

  test("playlist modal opens and closes", async ({ page }) => {
    await initPage(page);

    // Click playlist more button
    const moreButton = page.locator('[data-role="playlist-more"]');
    await moreButton.click();

    // Modal should open
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="playlist-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    // Press Escape to close
    await page.keyboard.press("Escape");

    // Modal should close
    await page.waitForFunction(
      () =>
        !document.querySelector(
          '[data-role="playlist-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );
  });

  test("playlist edit modal: rename", async ({ page }) => {
    await initPage(page);

    // Find a playlist edit button
    const editButton = page.locator('[data-action="playlist-edit"]').first();
    const playlistEditCount = await editButton.count();
    expect(
      playlistEditCount,
      "No playlist edit button found for rename test",
    ).toBeGreaterThan(0);
    if (playlistEditCount === 0) return;
    await editButton.click();

    // Wait for edit modal
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="playlist-edit-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    // Get current name
    const nameInput = page.locator('[data-role="playlist-edit-name"]');
    const originalName = await nameInput.inputValue();

    // Modify name
    await nameInput.fill(originalName + "_TEST");

    // Save
    await page.locator('[data-role="playlist-edit-save"]').click();

    // Wait for modal to close
    await page.waitForFunction(
      () =>
        !document.querySelector(
          '[data-role="playlist-edit-modal"][data-open="true"]',
        ),
      { timeout: 10_000 },
    );

    // Restore original name
    await editButton.click();
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="playlist-edit-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );
    await nameInput.fill(originalName);
    await page.locator('[data-role="playlist-edit-save"]').click();
  });

  test("presentation create modal: navigate steps", async ({ page }) => {
    await initPage(page);

    // Select a library first
    await page.locator('[data-role="library-item"]').first().click();
    await page.waitForSelector('[data-role="presentation-list"]', {
      timeout: 15_000,
    });

    // Open create modal
    await page.locator('[data-role="presentation-create"]').click();
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="presentation-create-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    // Verify options panel is visible
    const optionsPanel = page.locator(
      '[data-role="presentation-create-options"]',
    );
    await expect(optionsPanel).toBeVisible();

    // Click paste option
    await page.locator('[data-role="presentation-create-paste"]').click();

    // Paste area should be visible
    const pasteArea = page.locator(
      '[data-role="presentation-create-paste-area"]',
    );
    await expect(pasteArea).toBeVisible();

    // Click back
    await page.locator('[data-role="presentation-create-paste-back"]').click();

    // Options should be visible again
    await expect(optionsPanel).toBeVisible();

    // Click import option
    await page.locator('[data-role="presentation-create-import"]').click();

    // Import area should be visible
    const importArea = page.locator(
      '[data-role="presentation-create-import-area"]',
    );
    await expect(importArea).toBeVisible();

    // Close modal
    await page.keyboard.press("Escape");
  });

  test("all modals: escape closes", async ({ page }) => {
    await initPage(page);

    // Test library modal
    await page.locator('[data-role="library-more"]').click();
    await page.waitForFunction(
      () =>
        document.querySelector('[data-role="library-modal"][data-open="true"]'),
      { timeout: 5_000 },
    );
    await page.keyboard.press("Escape");
    await page.waitForFunction(
      () =>
        !document.querySelector(
          '[data-role="library-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    // Test playlist modal
    await page.locator('[data-role="playlist-more"]').click();
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="playlist-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );
    await page.keyboard.press("Escape");
    await page.waitForFunction(
      () =>
        !document.querySelector(
          '[data-role="playlist-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );
  });
});
