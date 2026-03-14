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
    await page.locator('[data-role="playlist-create"]').click();

    // Wait for create modal
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="playlist-edit-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    // Fill name
    const nameInput = page.locator('[data-role="playlist-edit-name"]');
    await nameInput.fill("E2E Test Playlist");

    // Submit
    await page.locator('[data-role="playlist-edit-save"]').click();

    // Modal should close
    await page.waitForFunction(
      () =>
        !document.querySelector(
          '[data-role="playlist-edit-modal"][data-open="true"]',
        ),
      { timeout: 10_000 },
    );

    // Wait for playlist list to update
    await page.waitForTimeout(500);

    // Verify playlist was created (should appear in list or modal)
    await page.locator('[data-role="playlist-more"]').click();
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="playlist-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    const createdPlaylist = page
      .locator('[data-role="playlist-modal"]')
      .getByText("E2E Test Playlist");
    await expect(createdPlaylist).toBeVisible();

    await page.keyboard.press("Escape");
  });

  test("dashboard toggle works", async ({ page }) => {
    await initPage(page);

    // Open playlist modal
    await page.locator('[data-role="playlist-more"]').click();
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="playlist-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    // Find toggle button
    const toggleButton = page
      .locator(
        '[data-role="playlist-modal"] [data-action="playlist-toggle-dashboard"]',
      )
      .first();
    const toggleCount = await toggleButton.count();
    expect(toggleCount, "No dashboard toggle button found").toBeGreaterThan(0);
    if (toggleCount === 0) return;

    // Get current state
    const wasPressed =
      (await toggleButton.getAttribute("aria-pressed")) === "true";

    // Toggle
    await toggleButton.click();

    // Wait for state to change
    await page.waitForFunction(
      (wasP) => {
        const btn = document.querySelector(
          '[data-role="playlist-modal"] [data-action="playlist-toggle-dashboard"]',
        );
        return btn && btn.getAttribute("aria-pressed") !== String(wasP);
      },
      wasPressed,
      { timeout: 5_000 },
    );

    // Close modal
    await page.keyboard.press("Escape");
  });

  test("delete playlist with confirmation", async ({ page }) => {
    await initPage(page);

    // First ensure we have a test playlist to delete (create one)
    await page.locator('[data-role="playlist-create"]').click();
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="playlist-edit-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    const nameInput = page.locator('[data-role="playlist-edit-name"]');
    await nameInput.fill("To Delete Playlist");
    await page.locator('[data-role="playlist-edit-save"]').click();

    await page.waitForFunction(
      () =>
        !document.querySelector(
          '[data-role="playlist-edit-modal"][data-open="true"]',
        ),
      { timeout: 10_000 },
    );

    await page.waitForTimeout(500);

    // Open playlist modal to find the newly created playlist (not in quick list unless dashboard is checked)
    await page.locator('[data-role="playlist-more"]').click();
    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="playlist-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    // Find playlist in modal
    const playlistRow = page
      .locator('[data-role="playlist-modal"] [data-role="playlist-row"]')
      .filter({ hasText: "To Delete Playlist" });
    const rowCount = await playlistRow.count();
    expect(rowCount, "Created playlist not found in modal").toBeGreaterThan(0);
    if (rowCount === 0) return;

    // Click the edit button within this row
    await playlistRow.locator('[data-action="playlist-edit"]').click();

    await page.waitForFunction(
      () =>
        document.querySelector(
          '[data-role="playlist-edit-modal"][data-open="true"]',
        ),
      { timeout: 5_000 },
    );

    // Accept confirmation dialog
    page.once("dialog", async (dialog) => {
      await dialog.accept();
    });

    // Click delete
    await page.locator('[data-role="playlist-edit-delete"]').click();

    // Wait for modal to close
    await page.waitForFunction(
      () =>
        !document.querySelector(
          '[data-role="playlist-edit-modal"][data-open="true"]',
        ),
      { timeout: 10_000 },
    );
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

    // Modal should have playlist items (modal uses playlist-row)
    const modalPlaylists = page.locator(
      '[data-role="playlist-modal"] [data-role="playlist-row"]',
    );
    const count = await modalPlaylists.count();
    expect(count, "Modal should contain playlists").toBeGreaterThan(0);
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
