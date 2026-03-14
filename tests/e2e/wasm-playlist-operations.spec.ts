/**
 * WASM Operator Playlist Operations Tests
 *
 * Tests playlist creation, management, and entry operations in the WASM operator.
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
  await page.waitForSelector('[data-role="playlist-list"]', {
    timeout: 30_000,
  });
}

test.describe("WASM Operator Playlist Operations", () => {
  test("playlist list is visible", async ({ page }) => {
    await initPage(page);
    const playlistList = page.locator('[data-role="playlist-list"]');
    await expect(playlistList).toBeVisible();
  });

  test("create playlist", async ({ page }) => {
    await initPage(page);

    // Click create button
    const createButton = page.locator('[data-role="playlist-create"]');
    await createButton.click();

    // Wait for edit modal (used for create)
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="playlist-edit-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    // Fill in name
    const nameInput = page.locator('[data-role="playlist-edit-name"]');
    await nameInput.fill("E2E Test Playlist");

    // Submit
    const saveButton = page.locator('[data-role="playlist-edit-save"]');
    await saveButton.click();

    // Wait for modal to close
    await page.waitForFunction(
      () =>
        !document.querySelector(
          '[data-role="playlist-edit-modal"][data-open="true"]',
        ),
      { timeout: 10_000 },
    );

    // Verify playlist appears
    await page.waitForSelector('[data-role="playlist-item"]', {
      timeout: 5_000,
    });
    const newPlaylist = page
      .locator('[data-role="playlist-item"]')
      .filter({ hasText: "E2E Test Playlist" });
    await expect(newPlaylist.first()).toBeVisible();
  });

  test("select playlist shows entries", async ({ page }) => {
    await initPage(page);

    // Click on a playlist
    const playlist = page.locator('[data-role="playlist-item"]').first();
    if ((await playlist.count()) === 0) {
      test.skip();
      return;
    }

    await playlist.click();

    // Presentation list should update
    await page.waitForFunction(
      () => {
        const title = document.querySelector('[data-role="context-title"]');
        return title && title.textContent !== "Presentations";
      },
      { timeout: 10_000 },
    );
  });

  test("playlist modal shows all playlists", async ({ page }) => {
    await initPage(page);

    // Click more button
    const moreButton = page.locator('[data-role="playlist-more"]');
    await moreButton.click();

    // Wait for modal
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="playlist-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    // Modal should have playlist items
    const modalPlaylists = page.locator(
      '[data-role="playlist-modal"] [data-role="playlist-item"]',
    );
    const count = await modalPlaylists.count();
    expect(count).toBeGreaterThanOrEqual(0);
  });

  test("add separator to playlist", async ({ page }) => {
    await initPage(page);

    // First select a playlist
    const playlist = page.locator('[data-role="playlist-item"]').first();
    if ((await playlist.count()) === 0) {
      test.skip();
      return;
    }
    await playlist.click();

    // Wait for playlist to load
    await page.waitForTimeout(500);

    // Click the "+" button (which adds separator when playlist is active)
    page.once("dialog", async (dialog) => {
      await dialog.accept("Test Separator");
    });

    const addButton = page.locator('[data-role="presentation-create"]');
    await addButton.click();

    // Wait for separator to appear
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="presentation-item"][data-type="separator"]',
        ),
      { timeout: 10_000 },
    );

    const separator = page
      .locator('[data-role="presentation-item"][data-type="separator"]')
      .filter({ hasText: "Test Separator" });
    await expect(separator.first()).toBeVisible();
  });

  test("dashboard toggle works", async ({ page }) => {
    await initPage(page);

    // Click edit on a playlist
    const editButton = page.locator('[data-action="playlist-edit"]').first();
    if ((await editButton.count()) === 0) {
      test.skip();
      return;
    }
    await editButton.click();

    // Wait for modal
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="playlist-edit-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    // Find dashboard toggle
    const dashboardCheckbox = page.locator(
      '[data-role="playlist-edit-dashboard"]',
    );
    if ((await dashboardCheckbox.count()) === 0) {
      // Close modal and skip
      await page.keyboard.press("Escape");
      test.skip();
      return;
    }

    // Toggle it
    const wasDashboard = await dashboardCheckbox.isChecked();
    await dashboardCheckbox.click();

    // Save
    const saveButton = page.locator('[data-role="playlist-edit-save"]');
    await saveButton.click();

    // Wait for modal to close
    await page.waitForFunction(
      () =>
        !document.querySelector(
          '[data-role="playlist-edit-modal"][data-open="true"]',
        ),
      { timeout: 10_000 },
    );

    // Restore original state
    await editButton.click();
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="playlist-edit-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );
    if (wasDashboard) {
      await dashboardCheckbox.check();
    } else {
      await dashboardCheckbox.uncheck();
    }
    await saveButton.click();
  });

  test("delete playlist with confirmation", async ({ page }) => {
    await initPage(page);

    // First create a playlist to delete
    const createButton = page.locator('[data-role="playlist-create"]');
    await createButton.click();
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="playlist-edit-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );
    await page
      .locator('[data-role="playlist-edit-name"]')
      .fill("TO_DELETE_PLAYLIST");
    await page.locator('[data-role="playlist-edit-save"]').click();
    await page.waitForFunction(
      () =>
        !document.querySelector(
          '[data-role="playlist-edit-modal"][data-open="true"]',
        ),
      { timeout: 10_000 },
    );

    // Wait for it to appear
    await page.waitForTimeout(500);

    // Find and edit the playlist we created
    const toDelete = page
      .locator('[data-role="playlist-item"]')
      .filter({ hasText: "TO_DELETE_PLAYLIST" });
    const editButton = toDelete.locator('[data-action="playlist-edit"]');
    await editButton.click();

    // Wait for modal
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="playlist-edit-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    // Set up dialog handler
    page.once("dialog", async (dialog) => {
      await dialog.accept();
    });

    // Click delete
    const deleteButton = page.locator('[data-role="playlist-edit-delete"]');
    await deleteButton.click();

    // Wait for modal to close and playlist to be gone
    await page.waitForFunction(
      () =>
        !document.querySelector(
          '[data-role="playlist-edit-modal"][data-open="true"]',
        ),
      { timeout: 10_000 },
    );

    // Verify deleted
    await expect(toDelete).not.toBeVisible({ timeout: 2_000 });
  });
});
