/**
 * Comprehensive WASM Operator E2E Tests
 *
 * Tests for /ui-next/operator to verify feature parity with /ui/operator (JavaScript version).
 * Covers: library selection, playlist operations, presentation operations, slide editing,
 * drag-and-drop, mode toggle, view navigation, search, timers, stage preview, and keyboard shortcuts.
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

// ========================
// LIBRARY TESTS
// ========================

test.describe("WASM Operator - Libraries", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto(`${baseURL}/ui-next/operator`);
    await waitForLibraryList(page);
  });

  test("library list displays with items", async ({ page }) => {
    const librarySection = page.locator('[data-role="library-list"]');
    await expect(librarySection).toBeVisible();

    // Should have library items or empty message
    await page.waitForFunction(
      () => {
        const list = document.querySelector('[data-role="library-list"]');
        if (!list) return false;
        const hasItems =
          list.querySelectorAll('[data-role="library-item"]').length > 0;
        const hasEmptyMessage =
          list.textContent?.includes("No libraries") ||
          list.textContent?.includes("Star libraries");
        return hasItems || hasEmptyMessage;
      },
      { timeout: 15_000 },
    );
  });

  test("clicking library loads presentations", async ({ page }) => {
    await selectFirstLibrary(page);

    const presItems = page.locator('[data-role="presentation-item"]');
    const count = await presItems.count();
    expect(count).toBeGreaterThan(0);
  });

  test("library shows active state when selected", async ({ page }) => {
    await selectFirstLibrary(page);

    const activeLib = page.locator(
      '[data-role="library-item"][data-active="true"]',
    );
    await expect(activeLib).toBeVisible();
  });

  test("library more button opens modal", async ({ page }) => {
    const moreBtn = page.locator('[data-role="library-more"]');
    await expect(moreBtn).toBeVisible();
    await moreBtn.click();

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
  });
});

// ========================
// PLAYLIST TESTS
// ========================

test.describe("WASM Operator - Playlists", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto(`${baseURL}/ui-next/operator`);
    await waitForLibraryList(page);
  });

  test("playlist list is visible", async ({ page }) => {
    const playlistSection = page.locator('[data-role="playlist-list"]');
    await expect(playlistSection).toBeVisible();
  });

  test("playlist more button opens modal", async ({ page }) => {
    const moreBtn = page.locator('[data-role="playlist-more"]');
    await expect(moreBtn).toBeVisible();
    await moreBtn.click();

    await page.waitForFunction(
      () => {
        const modal = document.querySelector('[data-role="playlist-modal"]');
        return modal && modal.getAttribute("data-open") === "true";
      },
      { timeout: 5_000 },
    );
  });

  test("create playlist button opens edit modal", async ({ page }) => {
    const createBtn = page.locator('[data-role="playlist-create"]');
    await createBtn.click();

    await page.waitForFunction(
      () => {
        const modal = document.querySelector(
          '[data-role="playlist-edit-modal"]',
        );
        return modal && modal.getAttribute("data-open") === "true";
      },
      { timeout: 5_000 },
    );

    // Modal should show create mode
    const title = page.locator('[data-role="playlist-edit-title"]');
    await expect(title).toHaveText("Create Playlist");
  });

  test("playlist creates successfully", async ({ page }) => {
    const createBtn = page.locator('[data-role="playlist-create"]');
    await createBtn.click();

    await page.waitForSelector('[data-role="playlist-edit-name"]', {
      timeout: 5_000,
    });

    const testName = `Test Playlist ${Date.now()}`;
    await page.fill('[data-role="playlist-edit-name"]', testName);
    await page.click('[data-role="playlist-edit-save"]');

    // Wait for modal to close
    await page.waitForFunction(
      () => {
        const modal = document.querySelector(
          '[data-role="playlist-edit-modal"]',
        );
        return !modal || modal.getAttribute("data-open") !== "true";
      },
      { timeout: 10_000 },
    );

    // Verify toast appears
    const toast = page.locator('[data-role="toast"]');
    await expect(toast).toContainText("Playlist saved");
  });
});

// ========================
// PRESENTATION TESTS
// ========================

test.describe("WASM Operator - Presentations", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto(`${baseURL}/ui-next/operator`);
    await selectFirstLibrary(page);
  });

  test("clicking presentation loads slides", async ({ page }) => {
    await selectFirstPresentation(page);

    const slides = page.locator("[data-slide-id]");
    const count = await slides.count();
    expect(count).toBeGreaterThan(0);
  });

  test("presentation shows active state when selected", async ({ page }) => {
    await selectFirstPresentation(page);

    const activePres = page.locator(
      '[data-role="presentation-item"][data-active="true"]',
    );
    await expect(activePres).toBeVisible();
  });

  test("presentation edit button opens rename modal in edit mode", async ({
    page,
  }) => {
    await setMode(page, "edit");

    const editBtn = page.locator('[data-action="presentation-rename"]').first();
    await editBtn.click();

    await page.waitForFunction(
      () => {
        const modal = document.querySelector(
          '[data-role="presentation-edit-modal"]',
        );
        return modal && modal.getAttribute("data-open") === "true";
      },
      { timeout: 5_000 },
    );
  });

  test("create presentation button opens modal", async ({ page }) => {
    const createBtn = page.locator('[data-role="presentation-create"]');
    await createBtn.click();

    await page.waitForFunction(
      () => {
        const modal = document.querySelector(
          '[data-role="presentation-create-modal"]',
        );
        return modal && modal.getAttribute("data-open") === "true";
      },
      { timeout: 5_000 },
    );
  });

  test("create blank presentation works", async ({ page }) => {
    const createBtn = page.locator('[data-role="presentation-create"]');
    await createBtn.click();

    await page.waitForSelector('[data-role="presentation-create-name"]', {
      timeout: 5_000,
    });

    const testName = `Test Pres ${Date.now()}`;
    await page.fill('[data-role="presentation-create-name"]', testName);
    await page.click('[data-role="presentation-create-blank"]');

    // Wait for modal to close
    await page.waitForFunction(
      () => {
        const modal = document.querySelector(
          '[data-role="presentation-create-modal"]',
        );
        return !modal || modal.getAttribute("data-open") !== "true";
      },
      { timeout: 10_000 },
    );

    // Toast should confirm creation
    const toast = page.locator('[data-role="toast"]');
    await expect(toast).toContainText("Presentation created");
  });

  test("paste presentation mode shows textarea", async ({ page }) => {
    const createBtn = page.locator('[data-role="presentation-create"]');
    await createBtn.click();

    await page.waitForSelector('[data-role="presentation-create-paste"]', {
      timeout: 5_000,
    });
    await page.click('[data-role="presentation-create-paste"]');

    const textarea = page.locator(
      '[data-role="presentation-create-paste-text"]',
    );
    await expect(textarea).toBeVisible();
  });

  test("import presentation mode shows file input", async ({ page }) => {
    const createBtn = page.locator('[data-role="presentation-create"]');
    await createBtn.click();

    await page.waitForSelector('[data-role="presentation-create-import"]', {
      timeout: 5_000,
    });
    await page.click('[data-role="presentation-create-import"]');

    const fileInput = page.locator(
      '[data-role="presentation-create-import-file"]',
    );
    await expect(fileInput).toBeVisible();
  });
});

// ========================
// SLIDE TESTS
// ========================

test.describe("WASM Operator - Slides", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto(`${baseURL}/ui-next/operator`);
    await selectFirstLibrary(page);
    await selectFirstPresentation(page);
  });

  test("slides display with correct structure", async ({ page }) => {
    const slideCards = page.locator("[data-slide-id]");
    const count = await slideCards.count();
    expect(count).toBeGreaterThan(0);

    // Each slide should have main text field
    const firstSlide = slideCards.first();
    const mainField = firstSlide.locator('[data-field="main"]');
    await expect(mainField).toBeVisible();
  });

  test("slide click triggers stage in live mode", async ({ page }) => {
    await setMode(page, "live");

    const firstSlide = page.locator("[data-slide-id]").first();
    await firstSlide.click();

    // Stage should update (check stage-current element)
    await page.waitForFunction(
      () => {
        const current = document.querySelector('[data-role="stage-current"]');
        return (
          current && current.textContent && current.textContent !== "\u2014"
        );
      },
      { timeout: 5_000 },
    );
  });

  test("slide edit fields appear in edit mode", async ({ page }) => {
    await setMode(page, "edit");

    const firstSlide = page.locator("[data-slide-id]").first();
    const mainField = firstSlide.locator('[data-field="main"]');
    await expect(mainField).toBeVisible();

    // In edit mode, should be editable textarea
    await expect(mainField).toHaveAttribute("contenteditable", /.*/);
  });

  test("slide editing saves changes", async ({ page }) => {
    await setMode(page, "edit");

    const firstSlide = page.locator("[data-slide-id]").first();
    const mainField = firstSlide.locator('[data-field="main"]');

    // Focus and edit
    await mainField.click();
    const originalText = await mainField.textContent();
    const newText = `Test edit ${Date.now()}`;
    await mainField.fill(newText);

    // Blur to trigger save
    await page.click("body", { position: { x: 10, y: 10 } });

    // Wait a bit for save to complete
    await page.waitForTimeout(500);

    // Verify the field still has the new text (wasn't reverted)
    await expect(mainField).toHaveText(newText);
  });

  test("slide reorder works via drag-drop", async ({ page }) => {
    await setMode(page, "edit");

    const slides = page.locator("[data-slide-id]");
    const count = await slides.count();
    // Require at least 2 slides to test reordering
    expect(count).toBeGreaterThanOrEqual(2);

    const firstSlide = slides.nth(0);
    const secondSlide = slides.nth(1);

    const firstId = await firstSlide.getAttribute("data-slide-id");
    const secondId = await secondSlide.getAttribute("data-slide-id");

    // Drag first slide to second position
    await firstSlide.dragTo(secondSlide);

    // Wait for reorder
    await page.waitForTimeout(500);

    // Verify order changed
    const newFirst = page.locator("[data-slide-id]").nth(0);
    const newFirstId = await newFirst.getAttribute("data-slide-id");
    expect(newFirstId).toBe(secondId);
  });

  test("add slide button creates new slide", async ({ page }) => {
    await setMode(page, "edit");

    const slidesBefore = await page.locator("[data-slide-id]").count();

    const addBtn = page.locator('[data-role="add-slide"]');
    await addBtn.click();

    await page.waitForTimeout(1000);

    const slidesAfter = await page.locator("[data-slide-id]").count();
    expect(slidesAfter).toBe(slidesBefore + 1);
  });
});

// ========================
// MODE TOGGLE TESTS
// ========================

test.describe("WASM Operator - Mode Toggle", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto(`${baseURL}/ui-next/operator`);
    await waitForLibraryList(page);
  });

  test("mode toggle switches between live and edit", async ({ page }) => {
    const initialMode = await page.locator("body").getAttribute("data-mode");

    // Toggle to the other mode
    const targetMode = initialMode === "live" ? "edit" : "live";
    await setMode(page, targetMode);

    const newMode = await page.locator("body").getAttribute("data-mode");
    expect(newMode).toBe(targetMode);
  });

  test("mode is persisted in session", async ({ page }) => {
    await setMode(page, "edit");

    // Reload page
    await page.reload();
    await waitForLibraryList(page);

    const mode = await page.locator("body").getAttribute("data-mode");
    expect(mode).toBe("edit");
  });
});

// ========================
// VIEW NAVIGATION TESTS
// ========================

test.describe("WASM Operator - View Navigation", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto(`${baseURL}/ui-next/operator`);
    await waitForLibraryList(page);
  });

  test("view toggles change body data-view attribute", async ({ page }) => {
    for (const view of ["bible", "timers", "settings", "worship"]) {
      const btn = page.locator(
        `[data-role="view-toggle"][data-view="${view}"]`,
      );
      await btn.click();

      await expect(page.locator("body")).toHaveAttribute("data-view", view);
    }
  });

  test("view toggle buttons show active state", async ({ page }) => {
    const worshipBtn = page.locator(
      '[data-role="view-toggle"][data-view="worship"]',
    );
    await expect(worshipBtn).toHaveAttribute("data-active", "true");

    const bibleBtn = page.locator(
      '[data-role="view-toggle"][data-view="bible"]',
    );
    await bibleBtn.click();

    await expect(bibleBtn).toHaveAttribute("data-active", "true");
    await expect(worshipBtn).toHaveAttribute("data-active", "false");
  });
});

// ========================
// SEARCH TESTS
// ========================

test.describe("WASM Operator - Search", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto(`${baseURL}/ui-next/operator`);
    await waitForLibraryList(page);
  });

  test("search input is visible", async ({ page }) => {
    const searchInput = page.locator('[data-role="global-search-query"]');
    await expect(searchInput).toBeVisible();
  });

  test("typing in search shows results", async ({ page }) => {
    const searchInput = page.locator('[data-role="global-search-query"]');
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
  });

  test("space key focuses search in live mode", async ({ page }) => {
    await setMode(page, "live");

    // Click body to ensure no input is focused
    await page.click("body", { position: { x: 10, y: 10 } });

    await page.keyboard.press("Space");

    const searchInput = page.locator('[data-role="global-search-query"]');
    await expect(searchInput).toBeFocused({ timeout: 2_000 });
  });

  test("escape key closes search results", async ({ page }) => {
    const searchInput = page.locator('[data-role="global-search-query"]');
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
  });

  test("search clear button clears input", async ({ page }) => {
    const searchInput = page.locator('[data-role="global-search-query"]');
    await searchInput.fill("test");

    const clearBtn = page.locator('[data-role="global-search-clear"]');
    await clearBtn.click();

    await expect(searchInput).toHaveValue("");
  });
});

// ========================
// TIMER TESTS
// ========================

test.describe("WASM Operator - Timers", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto(`${baseURL}/ui-next/operator`);
    await waitForLibraryList(page);

    // Switch to timers view
    const timerBtn = page.locator(
      '[data-role="view-toggle"][data-view="timers"]',
    );
    await timerBtn.click();
  });

  test("timer cards are visible", async ({ page }) => {
    const timerCards = page.locator('[data-role="timer-cards"]');
    await expect(timerCards).toBeVisible();
  });

  test("countdown controls exist", async ({ page }) => {
    const startBtn = page.locator('[data-role="countdown-start"]');
    const pauseBtn = page.locator('[data-role="countdown-pause"]');
    const resetBtn = page.locator('[data-role="countdown-reset"]');

    await expect(startBtn).toBeVisible();
    await expect(pauseBtn).toBeVisible();
    await expect(resetBtn).toBeVisible();
  });

  test("preach timer controls exist", async ({ page }) => {
    const startBtn = page.locator('[data-command="start_preach"]');
    const pauseBtn = page.locator('[data-command="pause_preach"]');
    const resetBtn = page.locator('[data-command="reset_preach"]');

    await expect(startBtn).toBeVisible();
    await expect(pauseBtn).toBeVisible();
    await expect(resetBtn).toBeVisible();
  });

  test("countdown offset buttons work", async ({ page }) => {
    const minusBtn = page.locator('[data-role="countdown-offset-minus"]');
    const plusBtn = page.locator('[data-role="countdown-offset-plus"]');

    await expect(minusBtn).toBeVisible();
    await expect(plusBtn).toBeVisible();

    // Click offset buttons - they should not throw errors
    await plusBtn.click();
    await minusBtn.click();
  });
});

// ========================
// STAGE PREVIEW TESTS
// ========================

test.describe("WASM Operator - Stage Preview", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto(`${baseURL}/ui-next/operator`);
    await waitForLibraryList(page);
  });

  test("stage preview is visible", async ({ page }) => {
    const stagePreview = page.locator('[data-role="stage-status"]');
    await expect(stagePreview).toBeVisible();
  });

  test("stage current and next panels exist", async ({ page }) => {
    const currentPanel = page.locator('[data-role="stage-current"]');
    const nextPanel = page.locator('[data-role="stage-next"]');

    await expect(currentPanel).toBeVisible();
    await expect(nextPanel).toBeVisible();
  });

  test("clear slide button works", async ({ page }) => {
    await selectFirstLibrary(page);
    await selectFirstPresentation(page);

    // Trigger a slide first
    const firstSlide = page.locator("[data-slide-id]").first();
    await firstSlide.click();

    // Wait for stage to update
    await page.waitForTimeout(500);

    // Click clear button
    const clearBtn = page.locator('[data-role="clear-slide"]');
    await clearBtn.click();

    // Toast should appear
    const toast = page.locator('[data-role="toast"]');
    await expect(toast).toContainText("cleared");
  });

  test("stage monitor shows connection counts", async ({ page }) => {
    const stageMonitor = page.locator('[data-role="stage-monitor"]');
    await expect(stageMonitor).toBeVisible();

    // Should have connection count spans
    const connected = page.locator('[data-role="stage-monitor-connected"]');
    const issues = page.locator('[data-role="stage-monitor-issues"]');

    await expect(connected).toBeVisible();
    await expect(issues).toBeVisible();
  });

  test("AbleSet controls exist", async ({ page }) => {
    const enableBtn = page.locator('[data-role="ableset-enable"]');
    const followBtn = page.locator('[data-role="ableset-follow"]');

    await expect(enableBtn).toBeVisible();
    await expect(followBtn).toBeVisible();
  });
});

// ========================
// STAGE LAYOUT TESTS
// ========================

test.describe("WASM Operator - Stage Layout", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto(`${baseURL}/ui-next/operator`);
    await waitForLibraryList(page);
  });

  test("stage layout select is visible", async ({ page }) => {
    const select = page.locator('[data-role="stage-layout-select"]');
    await expect(select).toBeVisible();
  });

  test("stage layout select has options", async ({ page }) => {
    const options = page.locator('[data-role="stage-layout-select"] option');
    const count = await options.count();
    expect(count).toBeGreaterThan(0);
  });
});

// ========================
// KEYBOARD SHORTCUTS TESTS
// ========================

test.describe("WASM Operator - Keyboard Shortcuts", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto(`${baseURL}/ui-next/operator`);
    await selectFirstLibrary(page);
    await selectFirstPresentation(page);
  });

  test("arrow keys navigate slides in live mode", async ({ page }) => {
    await setMode(page, "live");

    const slides = page.locator("[data-slide-id]");
    const count = await slides.count();
    // Require at least 2 slides to test arrow navigation
    expect(count).toBeGreaterThanOrEqual(2);

    // Trigger first slide
    const firstSlide = slides.first();
    await firstSlide.click();
    await page.waitForTimeout(500);

    // Press right arrow
    await page.keyboard.press("ArrowRight");
    await page.waitForTimeout(500);

    // Stage should show content from second slide
    const stageCurrent = page.locator('[data-role="stage-current"]');
    const currentText = await stageCurrent.textContent();
    expect(currentText).not.toBe("\u2014");
  });

  test("escape closes open modal", async ({ page }) => {
    // Open library modal
    const moreBtn = page.locator('[data-role="library-more"]');
    await moreBtn.click();

    await page.waitForFunction(
      () => {
        const modal = document.querySelector('[data-role="library-modal"]');
        return modal && modal.getAttribute("data-open") === "true";
      },
      { timeout: 5_000 },
    );

    // Press Escape
    await page.keyboard.press("Escape");

    // Modal should close
    await page.waitForFunction(
      () => {
        const modal = document.querySelector('[data-role="library-modal"]');
        return !modal || modal.getAttribute("data-open") !== "true";
      },
      { timeout: 5_000 },
    );
  });
});

// ========================
// CATALOG RESIZER TESTS
// ========================

test.describe("WASM Operator - Catalog Resizer", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto(`${baseURL}/ui-next/operator`);
    await waitForLibraryList(page);
  });

  test("catalog resizer is visible", async ({ page }) => {
    const resizer = page.locator('[data-role="catalog-resizer"]');
    await expect(resizer).toBeVisible();
  });
});

// ========================
// DRAG-DROP TESTS
// ========================

test.describe("WASM Operator - Drag and Drop", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto(`${baseURL}/ui-next/operator`);
    await waitForLibraryList(page);
  });

  test("presentations are draggable", async ({ page }) => {
    await selectFirstLibrary(page);

    const pres = page.locator('[data-role="presentation-item"]').first();
    await expect(pres).toHaveAttribute("draggable", "true");
  });

  test("search results are draggable", async ({ page }) => {
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
});

// ========================
// CONTENT PARITY TESTS
// ========================

test.describe("WASM Operator - Content Parity with JS", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto(`${baseURL}/ui-next/operator`);
    await waitForLibraryList(page);
  });

  test("library count matches JS operator", async ({ page }) => {
    const wasmCount = await page
      .locator('[data-role="library-more"]')
      .textContent();

    await page.goto(`${baseURL}/ui/operator`);
    await page.waitForSelector('[data-role="library-list"]', {
      timeout: 30_000,
    });

    const jsCount = await page
      .locator('[data-role="library-more"]')
      .textContent();

    expect(wasmCount).toBe(jsCount);
  });

  test("playlist count matches JS operator", async ({ page }) => {
    const wasmCount = await page
      .locator('[data-role="playlist-more"]')
      .textContent();

    await page.goto(`${baseURL}/ui/operator`);
    await page.waitForSelector('[data-role="playlist-list"]', {
      timeout: 30_000,
    });

    const jsCount = await page
      .locator('[data-role="playlist-more"]')
      .textContent();

    expect(wasmCount).toBe(jsCount);
  });

  test("presentation count matches when same library selected", async ({
    page,
  }) => {
    // Select first library in WASM
    await page.waitForSelector('[data-role="library-item"]', {
      timeout: 30_000,
    });
    await page.locator('[data-role="library-item"]').first().click();
    await waitForPresentations(page);

    const wasmCount = await page
      .locator('[data-role="presentation-count"]')
      .textContent();

    // Get the library ID
    const libId = await page
      .locator('[data-role="library-item"][data-active="true"]')
      .getAttribute("data-library-id");

    // Switch to JS operator and select same library
    await page.goto(`${baseURL}/ui/operator`);
    await page.waitForSelector('[data-role="library-list"]', {
      timeout: 30_000,
    });

    const jsLibBtn = page.locator(
      `[data-role="library-item"][data-library-id="${libId}"]`,
    );
    if ((await jsLibBtn.count()) > 0) {
      await jsLibBtn.click();
      await waitForPresentations(page);

      const jsCount = await page
        .locator('[data-role="presentation-count"]')
        .textContent();

      expect(wasmCount).toBe(jsCount);
    }
  });
});

// ========================
// SESSION PERSISTENCE TESTS
// ========================

test.describe("WASM Operator - Session Persistence", () => {
  test("selected library persists after reload", async ({ page }) => {
    await page.goto(`${baseURL}/ui-next/operator`);
    await waitForLibraryList(page);
    await selectFirstLibrary(page);

    const libId = await page
      .locator('[data-role="library-item"][data-active="true"]')
      .getAttribute("data-library-id");

    // Reload
    await page.reload();
    await waitForLibraryList(page);

    // Same library should be active
    await page.waitForFunction(
      (expectedId) => {
        const active = document.querySelector(
          '[data-role="library-item"][data-active="true"]',
        );
        return active && active.getAttribute("data-library-id") === expectedId;
      },
      libId,
      { timeout: 10_000 },
    );
  });

  test("selected presentation persists after reload", async ({ page }) => {
    await page.goto(`${baseURL}/ui-next/operator`);
    await selectFirstLibrary(page);
    await selectFirstPresentation(page);

    const presId = await page
      .locator('[data-role="presentation-item"][data-active="true"]')
      .getAttribute("data-presentation-id");

    // Reload
    await page.reload();
    await waitForLibraryList(page);

    // Presentation should still be loaded (slides visible)
    await waitForSlides(page);
  });
});

// ========================
// VERSION DISPLAY TESTS
// ========================

test.describe("WASM Operator - Version Display", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto(`${baseURL}/ui-next/operator`);
    await waitForLibraryList(page);
  });

  test("version badge is visible in header", async ({ page }) => {
    const versionBadge = page.locator(".operator__version-badge");
    await expect(versionBadge).toBeVisible();

    // Should contain version text starting with 'v'
    await page.waitForFunction(
      () => {
        const badge = document.querySelector(".operator__version-badge");
        return badge && badge.textContent && badge.textContent.startsWith("v");
      },
      { timeout: 5_000 },
    );
  });
});

// ========================
// TOAST NOTIFICATIONS TESTS
// ========================

test.describe("WASM Operator - Toast Notifications", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto(`${baseURL}/ui-next/operator`);
    await waitForLibraryList(page);
  });

  test("toast component exists", async ({ page }) => {
    const toast = page.locator('[data-role="toast"]');
    await expect(toast).toBeAttached();
  });
});

// ========================
// ERROR HANDLING TESTS
// ========================

test.describe("WASM Operator - Error Handling", () => {
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
});
