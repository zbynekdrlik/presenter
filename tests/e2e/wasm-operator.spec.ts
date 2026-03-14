/**
 * WASM Operator E2E Tests
 *
 * Comprehensive tests for /ui-next/operator covering all major features.
 * Tests are grouped to minimize server setup overhead.
 */

import { test, expect, Page } from "@playwright/test";
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

// Helper functions
async function waitForLibraryList(page: Page) {
  await page.waitForSelector('[data-role="library-list"]', { timeout: 30_000 });
}

async function waitForPresentations(page: Page) {
  await page.waitForFunction(
    () => {
      const list = document.querySelector('[data-role="presentation-list"]');
      if (!list) return false;
      return (
        list.querySelectorAll('[data-role="presentation-item"]').length > 0 ||
        list.textContent?.includes("No presentations")
      );
    },
    { timeout: 15_000 },
  );
}

async function waitForSlides(page: Page) {
  await page.waitForFunction(
    () => {
      const slides = document.querySelector('[data-role="slides"]');
      if (!slides) return false;
      return (
        slides.querySelectorAll("[data-slide-id]").length > 0 ||
        slides.textContent?.includes("Select a presentation")
      );
    },
    { timeout: 15_000 },
  );
}

async function selectFirstLibrary(page: Page) {
  await waitForLibraryList(page);
  const lib = page.locator('[data-role="library-item"]').first();
  await lib.click();
  await waitForPresentations(page);
}

async function selectFirstPresentation(page: Page) {
  const pres = page.locator('[data-role="presentation-item"]').first();
  await pres.click();
  await waitForSlides(page);
}

async function setMode(page: Page, mode: "live" | "edit") {
  const btn = page.locator(`[data-role="mode-toggle"][data-mode="${mode}"]`);
  await btn.click();
  await expect(page.locator("body")).toHaveAttribute("data-mode", mode);
}

// Combined test for library and presentation core functionality
test("library selection, presentation loading, and slide display", async ({
  page,
}) => {
  await page.goto(`${baseURL}/ui-next/operator`);
  await waitForLibraryList(page);

  // Library list displays
  const librarySection = page.locator('[data-role="library-list"]');
  await expect(librarySection).toBeVisible();

  // Click library loads presentations
  await selectFirstLibrary(page);
  const presItems = page.locator('[data-role="presentation-item"]');
  expect(await presItems.count()).toBeGreaterThan(0);

  // Library shows active state
  const activeLib = page.locator(
    '[data-role="library-item"][data-active="true"]',
  );
  await expect(activeLib).toBeVisible();

  // Click presentation loads slides
  await selectFirstPresentation(page);
  const slides = page.locator("[data-slide-id]");
  expect(await slides.count()).toBeGreaterThan(0);

  // Presentation shows active state
  const activePres = page.locator(
    '[data-role="presentation-item"][data-active="true"]',
  );
  await expect(activePres).toBeVisible();
});

// Combined test for modal functionality
test("modal operations - library and playlist modals open and close", async ({
  page,
}) => {
  await page.goto(`${baseURL}/ui-next/operator`);
  await waitForLibraryList(page);

  // Library more button opens modal
  const libMoreBtn = page.locator('[data-role="library-more"]');
  await libMoreBtn.click();
  await page.waitForFunction(
    () => {
      const modal = document.querySelector('[data-role="library-modal"]');
      return modal && modal.getAttribute("data-open") === "true";
    },
    { timeout: 5_000 },
  );

  // Close with Escape
  await page.keyboard.press("Escape");
  await page.waitForFunction(
    () => {
      const modal = document.querySelector('[data-role="library-modal"]');
      return !modal || modal.getAttribute("data-open") !== "true";
    },
    { timeout: 5_000 },
  );

  // Playlist more button opens modal
  const plMoreBtn = page.locator('[data-role="playlist-more"]');
  await plMoreBtn.click();
  await page.waitForFunction(
    () => {
      const modal = document.querySelector('[data-role="playlist-modal"]');
      return modal && modal.getAttribute("data-open") === "true";
    },
    { timeout: 5_000 },
  );

  // Close with Escape
  await page.keyboard.press("Escape");
  await page.waitForFunction(
    () => {
      const modal = document.querySelector('[data-role="playlist-modal"]');
      return !modal || modal.getAttribute("data-open") !== "true";
    },
    { timeout: 5_000 },
  );
});

// Combined test for playlist creation
test("playlist create modal and form submission", async ({ page }) => {
  await page.goto(`${baseURL}/ui-next/operator`);
  await waitForLibraryList(page);

  const createBtn = page.locator('[data-role="playlist-create"]');
  await createBtn.click();

  await page.waitForSelector('[data-role="playlist-edit-name"]', {
    timeout: 5_000,
  });

  // Modal shows create mode
  const title = page.locator('[data-role="playlist-edit-title"]');
  await expect(title).toHaveText("Create Playlist");

  const testName = `Test Playlist ${Date.now()}`;
  await page.fill('[data-role="playlist-edit-name"]', testName);
  await page.click('[data-role="playlist-edit-save"]');

  // Wait for modal to close
  await page.waitForFunction(
    () => {
      const modal = document.querySelector('[data-role="playlist-edit-modal"]');
      return !modal || modal.getAttribute("data-open") !== "true";
    },
    { timeout: 10_000 },
  );

  // Toast confirms success
  const toast = page.locator('[data-role="toast"]');
  await expect(toast).toContainText("Playlist saved");
});

// Combined test for presentation create modes
test("presentation create modal with blank, paste, and import options", async ({
  page,
}) => {
  await page.goto(`${baseURL}/ui-next/operator`);
  await selectFirstLibrary(page);

  const createBtn = page.locator('[data-role="presentation-create"]');
  await createBtn.click();

  await page.waitForSelector('[data-role="presentation-create-options"]', {
    timeout: 5_000,
  });

  // Blank button visible
  await expect(
    page.locator('[data-role="presentation-create-blank"]'),
  ).toBeVisible();

  // Paste mode shows textarea
  await page.click('[data-role="presentation-create-paste"]');
  const textarea = page.locator('[data-role="presentation-create-paste-text"]');
  await expect(textarea).toBeVisible();

  // Back to options
  await page.click('[data-role="presentation-create-paste-back"]');
  await expect(
    page.locator('[data-role="presentation-create-options"]'),
  ).toBeVisible();

  // Import mode shows file input
  await page.click('[data-role="presentation-create-import"]');
  const fileInput = page.locator(
    '[data-role="presentation-create-import-file"]',
  );
  await expect(fileInput).toBeVisible();
});

// Combined test for mode toggle and view navigation
test("mode toggle and view navigation work correctly", async ({ page }) => {
  await page.goto(`${baseURL}/ui-next/operator`);
  await waitForLibraryList(page);

  // Mode toggle
  const initialMode = await page.locator("body").getAttribute("data-mode");
  const targetMode = initialMode === "live" ? "edit" : "live";
  await setMode(page, targetMode);
  expect(await page.locator("body").getAttribute("data-mode")).toBe(targetMode);

  // View navigation
  for (const view of ["bible", "timers", "settings", "worship"]) {
    const btn = page.locator(`[data-role="view-toggle"][data-view="${view}"]`);
    await btn.click();
    await expect(page.locator("body")).toHaveAttribute("data-view", view);
  }
});

// Combined test for search functionality
test("search input, results display, and keyboard shortcuts", async ({
  page,
}) => {
  await page.goto(`${baseURL}/ui-next/operator`);
  await waitForLibraryList(page);

  const searchInput = page.locator('[data-role="global-search-query"]');
  await expect(searchInput).toBeVisible();

  // Type shows results
  await searchInput.fill("test");
  await page.waitForFunction(
    () => {
      const results = document.querySelector(
        '[data-role="global-search-results"]',
      );
      return results && results.getAttribute("data-visible") === "true";
    },
    { timeout: 5_000 },
  );

  // Escape closes results
  await page.keyboard.press("Escape");
  await page.waitForFunction(
    () => {
      const results = document.querySelector(
        '[data-role="global-search-results"]',
      );
      return !results || results.getAttribute("data-visible") !== "true";
    },
    { timeout: 5_000 },
  );

  // Space in live mode focuses search
  await setMode(page, "live");
  await page.click("body", { position: { x: 10, y: 10 } });
  await page.keyboard.press("Space");
  await expect(searchInput).toBeFocused({ timeout: 2_000 });
});

// Combined test for slide interaction
test("slide triggering in live mode and editing in edit mode", async ({
  page,
}) => {
  await page.goto(`${baseURL}/ui-next/operator`);
  await selectFirstLibrary(page);
  await selectFirstPresentation(page);

  // Live mode: clicking slide triggers stage
  await setMode(page, "live");
  const firstSlide = page.locator("[data-slide-id]").first();
  await firstSlide.click();
  await page.waitForFunction(
    () => {
      const current = document.querySelector('[data-role="stage-current"]');
      return current && current.textContent && current.textContent !== "\u2014";
    },
    { timeout: 5_000 },
  );

  // Edit mode: slide fields are visible
  await setMode(page, "edit");
  const mainField = firstSlide.locator('[data-field="main"]');
  await expect(mainField).toBeVisible();
});

// Combined test for timer controls
test("timer panel with countdown and preach controls", async ({ page }) => {
  await page.goto(`${baseURL}/ui-next/operator`);
  await waitForLibraryList(page);

  // Switch to timers view
  await page.click('[data-role="view-toggle"][data-view="timers"]');

  const timerCards = page.locator('[data-role="timer-cards"]');
  await expect(timerCards).toBeVisible();

  // Countdown controls
  await expect(page.locator('[data-role="countdown-start"]')).toBeVisible();
  await expect(page.locator('[data-role="countdown-pause"]')).toBeVisible();
  await expect(page.locator('[data-role="countdown-reset"]')).toBeVisible();
  await expect(
    page.locator('[data-role="countdown-offset-minus"]'),
  ).toBeVisible();
  await expect(
    page.locator('[data-role="countdown-offset-plus"]'),
  ).toBeVisible();

  // Preach controls
  await expect(page.locator('[data-command="start_preach"]')).toBeVisible();
  await expect(page.locator('[data-command="pause_preach"]')).toBeVisible();
  await expect(page.locator('[data-command="reset_preach"]')).toBeVisible();
});

// Combined test for stage preview
test("stage preview panel with controls and monitor", async ({ page }) => {
  await page.goto(`${baseURL}/ui-next/operator`);
  await waitForLibraryList(page);

  const stagePreview = page.locator('[data-role="stage-status"]');
  await expect(stagePreview).toBeVisible();

  // Stage panels exist
  await expect(page.locator('[data-role="stage-current"]')).toBeVisible();
  await expect(page.locator('[data-role="stage-next"]')).toBeVisible();

  // Monitor shows connection counts
  const stageMonitor = page.locator('[data-role="stage-monitor"]');
  await expect(stageMonitor).toBeVisible();

  // AbleSet controls exist
  await expect(page.locator('[data-role="ableset-enable"]')).toBeVisible();
  await expect(page.locator('[data-role="ableset-follow"]')).toBeVisible();

  // Clear button exists
  await expect(page.locator('[data-role="clear-slide"]')).toBeVisible();
});

// Combined test for content parity with JS operator
test("content parity - library and playlist counts match JS operator", async ({
  page,
}) => {
  await page.goto(`${baseURL}/ui-next/operator`);
  await waitForLibraryList(page);

  const wasmLibCount = await page
    .locator('[data-role="library-more"]')
    .textContent();
  const wasmPlCount = await page
    .locator('[data-role="playlist-more"]')
    .textContent();

  await page.goto(`${baseURL}/ui/operator`);
  await page.waitForSelector('[data-role="library-list"]', { timeout: 30_000 });

  const jsLibCount = await page
    .locator('[data-role="library-more"]')
    .textContent();
  const jsPlCount = await page
    .locator('[data-role="playlist-more"]')
    .textContent();

  expect(wasmLibCount).toBe(jsLibCount);
  expect(wasmPlCount).toBe(jsPlCount);
});

// Test for drag and drop capabilities
test("drag-drop capabilities exist for presentations and search results", async ({
  page,
}) => {
  await page.goto(`${baseURL}/ui-next/operator`);
  await selectFirstLibrary(page);

  // Presentations are draggable
  const pres = page.locator('[data-role="presentation-item"]').first();
  await expect(pres).toHaveAttribute("draggable", "true");

  // Search results are draggable
  const searchInput = page.locator('[data-role="global-search-query"]');
  await searchInput.fill("test");
  await page.waitForFunction(
    () => {
      const results = document.querySelector(
        '[data-role="global-search-results"]',
      );
      const items = results?.querySelectorAll(
        '[data-role="search-result-item"]',
      );
      return items && items.length > 0;
    },
    { timeout: 5_000 },
  );
  const resultItem = page.locator('[data-role="search-result-item"]').first();
  await expect(resultItem).toHaveAttribute("draggable", "true");
});

// Test for UI components visibility
test("UI components - header, catalog resizer, version, toast", async ({
  page,
}) => {
  await page.goto(`${baseURL}/ui-next/operator`);
  await waitForLibraryList(page);

  // Header elements
  await expect(page.locator(".operator__header")).toBeVisible();
  await expect(page.locator('[data-role="global-search-query"]')).toBeVisible();

  // Catalog resizer
  await expect(page.locator('[data-role="catalog-resizer"]')).toBeVisible();

  // Stage layout select
  await expect(page.locator('[data-role="stage-layout-select"]')).toBeVisible();

  // Version badge
  const versionBadge = page.locator(".operator__version-badge");
  await expect(versionBadge).toBeVisible();

  // Toast component exists
  const toast = page.locator('[data-role="toast"]');
  await expect(toast).toBeAttached();
});

// Test for no console errors
test("no console errors on page load", async ({ page }) => {
  const errors: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error") {
      errors.push(msg.text());
    }
  });

  await page.goto(`${baseURL}/ui-next/operator`);
  await waitForLibraryList(page);
  await page.waitForTimeout(2_000);

  // Filter out expected WASM/WebSocket messages
  const realErrors = errors.filter(
    (e) =>
      !e.includes("wasm-bindgen") &&
      !e.includes("WebSocket") &&
      !e.includes("Failed to fetch"),
  );

  expect(realErrors).toHaveLength(0);
});
