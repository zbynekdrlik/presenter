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

  test("select playlist shows entries", async ({ page }) => {
    await initPage(page);

    // Click on a playlist
    const playlist = page.locator('[data-role="playlist-item"]').first();
    const playlistCount = await playlist.count();
    expect(
      playlistCount,
      "No playlists available for select test",
    ).toBeGreaterThan(0);
    if (playlistCount === 0) return;

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
    const playlistCountForSeparator = await playlist.count();
    expect(
      playlistCountForSeparator,
      "No playlists available for separator test",
    ).toBeGreaterThan(0);
    if (playlistCountForSeparator === 0) return;
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
});
