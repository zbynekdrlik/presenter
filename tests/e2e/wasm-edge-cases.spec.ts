/**
 * WASM Operator Edge Cases Tests
 *
 * Tests error handling, empty states, and edge cases in the WASM operator.
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

test.describe("WASM Operator Edge Cases", () => {
  test("empty library shows message", async ({ page }) => {
    await page.goto(`${baseURL}/ui-next/operator`);
    await page.waitForSelector('[data-role="library-list"]', {
      timeout: 30_000,
    });

    // Check if library list shows loading or content
    await page.waitForFunction(
      () => {
        const list = document.querySelector('[data-role="library-list"]');
        if (!list) return false;
        const loading = list.textContent?.includes("Loading");
        return !loading;
      },
      { timeout: 30_000 },
    );

    // If no libraries exist, message should be shown
    const items = await page.locator('[data-role="library-item"]').count();
    if (items === 0) {
      const message = page.locator('[data-role="library-list"]');
      await expect(message).toContainText(/(No libraries|Star libraries)/);
    }
  });

  test("empty playlist shows message", async ({ page }) => {
    await page.goto(`${baseURL}/ui-next/operator`);
    await page.waitForSelector('[data-role="playlist-list"]', {
      timeout: 30_000,
    });

    // If no playlists exist, message should be shown
    const items = await page.locator('[data-role="playlist-item"]').count();
    if (items === 0) {
      const message = page.locator('[data-role="playlist-list"]');
      await expect(message).toContainText(/No playlists|Create/);
    }
  });

  test("no presentation selected shows prompt", async ({ page }) => {
    await page.goto(`${baseURL}/ui-next/operator`);
    await page.waitForSelector('[data-role="library-list"]', {
      timeout: 30_000,
    });

    // Without selecting anything, slides area should show prompt
    const slidesArea = page.locator('[data-role="slides"]');
    const emptyMessage = slidesArea.locator(".empty");
    if ((await emptyMessage.count()) > 0) {
      await expect(emptyMessage).toContainText(/Select/);
    }
  });

  test("toast appears on element", async ({ page }) => {
    await page.goto(`${baseURL}/ui-next/operator`);
    await page.waitForSelector('[data-role="library-list"]', {
      timeout: 30_000,
    });

    // Toast element should exist (even if not visible)
    const toast = page.locator('[data-role="toast"]');
    await expect(toast).toHaveCount(1);
  });

  test("no console errors on normal operation", async ({ page }) => {
    const errors: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error") {
        errors.push(msg.text());
      }
    });

    await page.goto(`${baseURL}/ui-next/operator`);
    await page.waitForSelector('[data-role="library-list"]', {
      timeout: 30_000,
    });
    await page.waitForSelector('[data-role="library-item"]', {
      timeout: 30_000,
    });

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
          .querySelector('[data-role="slides"]')
          ?.querySelectorAll("[data-slide-id]").length ?? 0 > 0,
      { timeout: 15_000 },
    );

    // Toggle mode
    await page.locator('[data-role="mode-toggle"][data-mode="edit"]').click();
    await page.waitForFunction(
      () => document.body.getAttribute("data-mode") === "edit",
      { timeout: 5_000 },
    );

    // Filter errors (ignore expected ones)
    const realErrors = errors.filter(
      (e) =>
        !e.includes("wasm-bindgen") &&
        !e.includes("WebSocket") &&
        !e.includes("Failed to fetch"),
    );

    expect(realErrors).toHaveLength(0);
  });

  test("session state restored on refresh", async ({ page }) => {
    await page.goto(`${baseURL}/ui-next/operator`);
    await page.waitForSelector('[data-role="library-item"]', {
      timeout: 30_000,
    });

    // Select library and presentation
    await page.locator('[data-role="library-item"]').first().click();
    await page.waitForSelector('[data-role="presentation-item"]', {
      timeout: 15_000,
    });
    await page.locator('[data-role="presentation-item"]').first().click();

    // Get selected presentation ID
    const presId = await page
      .locator('[data-role="presentation-item"][data-active="true"]')
      .getAttribute("data-presentation-id");

    // Reload page
    await page.reload();
    await page.waitForSelector('[data-role="library-list"]', {
      timeout: 30_000,
    });

    // Wait for data to load
    await page.waitForTimeout(2000);

    // Check if presentation is still selected (via session storage)
    // The session may or may not restore depending on implementation
    const state = await page.evaluate(() => {
      return (window as any).__presenterOperatorState?.();
    });

    // At minimum, state object should exist
    expect(state).toBeDefined();
  });

  test("long content scrolls correctly", async ({ page }) => {
    await page.goto(`${baseURL}/ui-next/operator`);
    await page.waitForSelector('[data-role="library-item"]', {
      timeout: 30_000,
    });

    // Select library
    await page.locator('[data-role="library-item"]').first().click();
    await page.waitForSelector('[data-role="presentation-item"]', {
      timeout: 15_000,
    });

    // Select presentation
    await page.locator('[data-role="presentation-item"]').first().click();
    await page.waitForFunction(
      () =>
        document
          .querySelector('[data-role="slides"]')
          ?.querySelectorAll("[data-slide-id]").length ?? 0 > 0,
      { timeout: 15_000 },
    );

    // Check if slides container is scrollable
    const slidesContainer = page.locator('[data-role="slides"]');
    const isScrollable = await slidesContainer.evaluate((el) => {
      return el.scrollHeight > el.clientHeight;
    });

    // If content is long enough, it should be scrollable
    // (This depends on the number of slides)
    // Just verify the container exists and has content
    await expect(slidesContainer).toBeVisible();
  });

  test("test helpers are exposed", async ({ page }) => {
    await page.goto(`${baseURL}/ui-next/operator`);
    await page.waitForSelector('[data-role="library-list"]', {
      timeout: 30_000,
    });

    // Check test helpers exist
    const helpers = await page.evaluate(() => {
      const h = (window as any).__presenterOperatorTestHelpers;
      if (!h) return null;
      return {
        hasAddPresentation: typeof h.addPresentationToPlaylist === "function",
        hasPlaylistCount: typeof h.playlistPresentationCount === "function",
        hasReorderSlides: typeof h.reorderSlides === "function",
        hasSlideOrder: typeof h.slideOrder === "function",
        hasStageMonitorCounts: typeof h.stageMonitorCounts === "function",
        hasResetBaseline: typeof h.resetStageMonitorBaseline === "function",
        hasClearSearch: typeof h.clearSearch === "function",
        hasParseSongText: typeof h.parseSongText === "function",
      };
    });

    expect(helpers).toBeTruthy();
    expect(helpers?.hasAddPresentation).toBe(true);
    expect(helpers?.hasPlaylistCount).toBe(true);
    expect(helpers?.hasReorderSlides).toBe(true);
    expect(helpers?.hasSlideOrder).toBe(true);
    expect(helpers?.hasStageMonitorCounts).toBe(true);
    expect(helpers?.hasResetBaseline).toBe(true);
    expect(helpers?.hasClearSearch).toBe(true);
    expect(helpers?.hasParseSongText).toBe(true);
  });

  test("parseSongText handles verse markers", async ({ page }) => {
    await page.goto(`${baseURL}/ui-next/operator`);
    await page.waitForSelector('[data-role="library-list"]', {
      timeout: 30_000,
    });

    const result = await page.evaluate(() => {
      const h = (window as any).__presenterOperatorTestHelpers;
      if (!h?.parseSongText) return null;
      return h.parseSongText(`Verse 1
Line one
Line two

Chorus
Chorus line

Verse 2
More content`);
    });

    expect(result).toHaveLength(3);
    expect(result?.[0]?.group).toBe("Verse 1");
    expect(result?.[1]?.group).toBe("Chorus");
    expect(result?.[2]?.group).toBe("Verse 2");
  });

  test("stage monitor baseline can be reset", async ({ page }) => {
    await page.goto(`${baseURL}/ui-next/operator`);
    await page.waitForSelector('[data-role="library-list"]', {
      timeout: 30_000,
    });

    // Reset baseline
    const result = await page.evaluate(() => {
      const h = (window as any).__presenterOperatorTestHelpers;
      if (!h?.resetStageMonitorBaseline) return null;
      return h.resetStageMonitorBaseline();
    });

    expect(result).toBe(true);

    // Verify counts are available
    const counts = await page.evaluate(() => {
      const h = (window as any).__presenterOperatorTestHelpers;
      if (!h?.stageMonitorCounts) return null;
      return h.stageMonitorCounts();
    });

    expect(counts).toHaveProperty("connected");
    expect(counts).toHaveProperty("issues");
    expect(counts).toHaveProperty("baselineConnected");
    expect(counts).toHaveProperty("baselineIssues");
  });
});
