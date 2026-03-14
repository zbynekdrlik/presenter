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

  test("playlist accepts presentation drop", async ({ page }) => {
    await initPage(page);

    // Select library to load presentations
    await page.locator('[data-role="library-item"]').first().click();
    await page.waitForSelector('[data-role="presentation-item"]', {
      timeout: 15_000,
    });

    // Get a playlist
    const playlist = page.locator('[data-role="playlist-item"]').first();
    const playlistCount = await playlist.count();
    expect(
      playlistCount,
      "No playlists available for drop test",
    ).toBeGreaterThan(0);
    if (playlistCount === 0) return;

    // Get initial playlist count
    const initialCount = await page.evaluate(async () => {
      const helpers = (window as any).__presenterOperatorTestHelpers;
      const playlists = document.querySelectorAll(
        '[data-role="playlist-item"]',
      );
      const firstPlaylistId = playlists[0]
        ?.closest("[data-playlist-id]")
        ?.getAttribute("data-playlist-id");
      if (firstPlaylistId && helpers?.playlistPresentationCount) {
        return helpers.playlistPresentationCount(firstPlaylistId) ?? 0;
      }
      return 0;
    });

    // Simulate drag-drop using test helper
    const presId = await page
      .locator('[data-role="presentation-item"]')
      .first()
      .getAttribute("data-presentation-id");
    const playlistId = await playlist
      .closest("[data-playlist-id]")
      ?.getAttribute("data-playlist-id");

    if (presId && playlistId) {
      await page.evaluate(
        async ({ presId, playlistId }) => {
          const helpers = (window as any).__presenterOperatorTestHelpers;
          if (helpers?.addPresentationToPlaylist) {
            await helpers.addPresentationToPlaylist(playlistId, presId);
          }
        },
        { presId, playlistId },
      );

      // Wait for update
      await page.waitForTimeout(1000);

      // Verify count increased
      const newCount = await page.evaluate(async (playlistId) => {
        const helpers = (window as any).__presenterOperatorTestHelpers;
        if (helpers?.playlistPresentationCount) {
          return helpers.playlistPresentationCount(playlistId) ?? 0;
        }
        return 0;
      }, playlistId);

      expect(newCount).toBeGreaterThan(initialCount);
    }
  });

  test("slide reorder via test helper", async ({ page }) => {
    await loadPresentation(page);

    // Get slide order
    const presId = await page
      .locator('[data-role="presentation-item"][data-active="true"]')
      .getAttribute("data-presentation-id");

    expect(
      presId,
      "No active presentation found for slide reorder test",
    ).toBeTruthy();
    if (!presId) return;

    const initialOrder = await page.evaluate((presId) => {
      const helpers = (window as any).__presenterOperatorTestHelpers;
      if (helpers?.slideOrder) {
        return helpers.slideOrder(presId) ?? [];
      }
      return [];
    }, presId);

    expect(
      initialOrder.length,
      "Presentation needs at least 2 slides for reorder test",
    ).toBeGreaterThanOrEqual(2);
    if (initialOrder.length < 2) return;

    // Reorder: swap first two slides
    const reorderedSlides = [
      initialOrder[1],
      initialOrder[0],
      ...initialOrder.slice(2),
    ];

    await page.evaluate(
      async ({ presId, slideIds }) => {
        const helpers = (window as any).__presenterOperatorTestHelpers;
        if (helpers?.reorderSlides) {
          await helpers.reorderSlides(presId, slideIds);
        }
      },
      { presId, slideIds: reorderedSlides },
    );

    // Wait for update
    await page.waitForTimeout(1000);

    // Verify new order
    const newOrder = await page.evaluate((presId) => {
      const helpers = (window as any).__presenterOperatorTestHelpers;
      if (helpers?.slideOrder) {
        return helpers.slideOrder(presId) ?? [];
      }
      return [];
    }, presId);

    expect(newOrder[0]).toBe(initialOrder[1]);
    expect(newOrder[1]).toBe(initialOrder[0]);

    // Restore original order
    await page.evaluate(
      async ({ presId, slideIds }) => {
        const helpers = (window as any).__presenterOperatorTestHelpers;
        if (helpers?.reorderSlides) {
          await helpers.reorderSlides(presId, slideIds);
        }
      },
      { presId, slideIds: initialOrder },
    );
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

  test("playlist entry is draggable when in playlist context", async ({
    page,
  }) => {
    await initPage(page);

    // Select a playlist
    const playlist = page.locator('[data-role="playlist-item"]').first();
    const playlistCountForEntry = await playlist.count();
    expect(
      playlistCountForEntry,
      "No playlists available for entry drag test",
    ).toBeGreaterThan(0);
    if (playlistCountForEntry === 0) return;
    await playlist.click();

    // Wait for playlist entries
    await page.waitForTimeout(500);

    // Check if there are entries
    const entries = page.locator(
      '[data-role="presentation-item"][data-entry-id]',
    );
    const entriesCount = await entries.count();
    expect(
      entriesCount,
      "Empty playlist - no entries available for drag test",
    ).toBeGreaterThan(0);
    if (entriesCount === 0) return;

    // Verify entry is draggable
    const firstEntry = entries.first();
    await expect(firstEntry).toHaveAttribute("draggable", "true");
  });
});
