/**
 * WASM Operator Bible Tests
 *
 * Tests Bible functionality in the WASM operator: tab navigation, book/verse
 * selection, slide loading, trigger, selection, presentations, preferences.
 */

import { test, expect } from "@playwright/test";
import {
  assertTwoColumnLayout,
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

/** Navigate to operator and switch to Bible view. */
async function navigateToBible(page: import("@playwright/test").Page) {
  await page.goto(`${baseURL}/ui/operator`);
  await page.waitForSelector('[data-role="library-list"]', { timeout: 30_000 });

  // Navigate to bible view
  const bibleButton = page.locator(
    '[data-role="view-toggle"][data-view="bible"]',
  );
  if ((await bibleButton.count()) > 0) {
    await bibleButton.click();
  } else {
    const bibleTab = page.locator('button:has-text("Bible")').first();
    if ((await bibleTab.count()) > 0) {
      await bibleTab.click();
    }
  }

  // Wait for bible view to be active
  await page.waitForFunction(
    () => document.body.getAttribute("data-view") === "bible",
    { timeout: 5_000 },
  );
}

/** Get the bible view panel as a scoped locator. */
function biblePanel(page: import("@playwright/test").Page) {
  return page.locator('[data-view-panel="bible"]');
}

/** Check if Bible data is available (translations + books loaded). */
async function hasBibleData(
  page: import("@playwright/test").Page,
): Promise<boolean> {
  const bookList = page.locator('[data-role="book-list"]');
  const bookItems = bookList.locator('[data-role="book-item"]');
  const count = await bookItems.count();
  return count > 0;
}

/** Clear any active Bible broadcast before tests that need clean state. */
async function clearBroadcast() {
  try {
    await fetch(`${baseURL}/bible/clear`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: "{}",
    });
  } catch {
    // Ignore errors
  }
}

test.describe("WASM Operator Bible Tests", () => {
  // -----------------------------------------------------------------------
  // Tab navigation
  // -----------------------------------------------------------------------

  test("bible tab is visible and navigable", async ({ page }) => {
    await page.goto(`${baseURL}/ui/operator`);
    await page.waitForSelector('[data-role="library-list"]', {
      timeout: 30_000,
    });

    const bibleButton = page.locator(
      '[data-role="view-toggle"][data-view="bible"]',
    );
    if ((await bibleButton.count()) > 0) {
      await expect(bibleButton).toBeVisible();
    } else {
      const bibleTab = page.locator('button:has-text("Bible")').first();
      await expect(bibleTab).toBeVisible();
    }
  });

  test("body gets operator--bible class when in bible view", async ({
    page,
  }) => {
    await navigateToBible(page);

    const hasBibleClass = await page.evaluate(() =>
      document.body.classList.contains("operator--bible"),
    );
    expect(hasBibleClass).toBe(true);
  });

  test("operator--bible class removed when leaving bible view", async ({
    page,
  }) => {
    await navigateToBible(page);

    // Switch to worship view
    const worshipButton = page.locator(
      '[data-role="view-toggle"][data-view="worship"]',
    );
    if ((await worshipButton.count()) > 0) {
      await worshipButton.click();
      await page.waitForFunction(
        () => document.body.getAttribute("data-view") === "worship",
        { timeout: 5_000 },
      );
    }

    const hasBibleClass = await page.evaluate(() =>
      document.body.classList.contains("operator--bible"),
    );
    expect(hasBibleClass).toBe(false);
  });

  test("bible sub-tabs are visible (Live, Prepared, Settings)", async ({
    page,
  }) => {
    await navigateToBible(page);

    const tabNav = page.locator('[data-role="bible-tab-nav"]');
    await expect(tabNav).toBeVisible();

    const liveTab = page.locator('[data-role="bible-tab"][data-tab="live"]');
    const preparedTab = page.locator(
      '[data-role="bible-tab"][data-tab="prepared"]',
    );
    const settingsTab = page.locator(
      '[data-role="bible-tab"][data-tab="settings"]',
    );

    await expect(liveTab).toBeVisible();
    await expect(preparedTab).toBeVisible();
    await expect(settingsTab).toBeVisible();
  });

  test("tab switching shows correct panels", async ({ page }) => {
    await navigateToBible(page);

    // Live tab should be active by default
    const livePanel = page.locator('[data-bible-panel="live"]');
    await expect(livePanel).toHaveAttribute("data-visible", "true");

    // Click Prepared tab
    await page.locator('[data-role="bible-tab"][data-tab="prepared"]').click();
    const preparedPanel = page.locator('[data-bible-panel="prepared"]');
    await expect(preparedPanel).toHaveAttribute("data-visible", "true");
    await expect(livePanel).toHaveAttribute("data-visible", "false");

    // Click Settings tab
    await page.locator('[data-role="bible-tab"][data-tab="settings"]').click();
    const settingsPanel = page.locator('[data-bible-panel="settings"]');
    await expect(settingsPanel).toHaveAttribute("data-visible", "true");
    await expect(preparedPanel).toHaveAttribute("data-visible", "false");

    // Click Live tab again
    await page.locator('[data-role="bible-tab"][data-tab="live"]').click();
    await expect(livePanel).toHaveAttribute("data-visible", "true");
    await expect(settingsPanel).toHaveAttribute("data-visible", "false");
  });

  // -----------------------------------------------------------------------
  // Live tab: translation selectors
  // -----------------------------------------------------------------------

  test("translation selectors are visible", async ({ page }) => {
    await navigateToBible(page);

    const mainTranslation = page.locator('[data-role="main-translation"]');
    const secondaryTranslation = page.locator(
      '[data-role="secondary-translation"]',
    );

    await expect(mainTranslation).toBeVisible({ timeout: 10_000 });
    await expect(secondaryTranslation).toBeVisible();
  });

  test("main translation dropdown has options", async ({ page }) => {
    await navigateToBible(page);

    const mainTranslation = page.locator('[data-role="main-translation"]');
    await expect(mainTranslation).toBeVisible({ timeout: 10_000 });

    const options = mainTranslation.locator("option");
    const count = await options.count();
    expect(count).toBeGreaterThanOrEqual(1);
  });

  test("secondary translation has 'None' option", async ({ page }) => {
    await navigateToBible(page);

    const secondaryTranslation = page.locator(
      '[data-role="secondary-translation"]',
    );
    await expect(secondaryTranslation).toBeVisible({ timeout: 10_000 });

    const noneOption = secondaryTranslation.locator('option[value=""]');
    await expect(noneOption).toHaveText("None");
  });

  // -----------------------------------------------------------------------
  // Live tab: book selection
  // -----------------------------------------------------------------------

  test("book filter input is visible", async ({ page }) => {
    await navigateToBible(page);

    const bookFilter = page.locator('[data-role="book-filter"]');
    await expect(bookFilter).toBeVisible({ timeout: 10_000 });
  });

  test("book list loads when translation is selected", async ({ page }) => {
    await navigateToBible(page);

    // Wait for books to load
    const bookList = page.locator('[data-role="book-list"]');
    await expect(bookList).toBeVisible({ timeout: 10_000 });

    // Should have book items
    await page.waitForFunction(
      () => {
        const items = document.querySelectorAll('[data-role="book-item"]');
        return items.length > 0;
      },
      { timeout: 10_000 },
    );

    const bookItems = bookList.locator('[data-role="book-item"]');
    const count = await bookItems.count();
    expect(count).toBeGreaterThan(0);
  });

  test("book filter narrows book list", async ({ page }) => {
    await navigateToBible(page);

    // Wait for books to load
    await page.waitForFunction(
      () => document.querySelectorAll('[data-role="book-item"]').length > 0,
      { timeout: 10_000 },
    );

    const initialCount = await page.locator('[data-role="book-item"]').count();

    // Type a filter
    const bookFilter = page.locator('[data-role="book-filter"]');
    await bookFilter.fill("John");

    // Wait for filtered list to update
    await page.waitForTimeout(300);

    const filteredCount = await page.locator('[data-role="book-item"]').count();

    // Filtered count should be less (unless all books contain "John", unlikely)
    if (initialCount > 5) {
      expect(filteredCount).toBeLessThan(initialCount);
    }
    expect(filteredCount).toBeGreaterThan(0);
  });

  test("clicking a book selects it", async ({ page }) => {
    await navigateToBible(page);

    await page.waitForFunction(
      () => document.querySelectorAll('[data-role="book-item"]').length > 0,
      { timeout: 10_000 },
    );

    const firstBook = page.locator('[data-role="book-item"]').first();
    await firstBook.click();

    // Should be marked active
    await expect(firstBook).toHaveAttribute("data-active", "true");
  });

  // -----------------------------------------------------------------------
  // Live tab: reference inputs and load passage
  // -----------------------------------------------------------------------

  test("chapter and verse inputs are visible", async ({ page }) => {
    await navigateToBible(page);

    const chapterInput = page.locator('[data-role="chapter-input"]');
    const verseStart = page.locator('[data-role="verse-start"]');
    const verseEnd = page.locator('[data-role="verse-end"]');

    await expect(chapterInput).toBeVisible();
    await expect(verseStart).toBeVisible();
    await expect(verseEnd).toBeVisible();
  });

  test("load passage button is visible", async ({ page }) => {
    await navigateToBible(page);

    const loadButton = page.locator('[data-role="load-button"]');
    await expect(loadButton).toBeVisible();
  });

  test("loading a passage generates slides", async ({ page }) => {
    await navigateToBible(page);

    if (!(await hasBibleData(page))) {
      // No Bible data available — skip gracefully
      return;
    }

    // Select first book
    const firstBook = page.locator('[data-role="book-item"]').first();
    await firstBook.click();

    // Set chapter to 1, verse 1
    const chapterInput = page.locator('[data-role="chapter-input"]');
    await chapterInput.fill("1");

    const verseStart = page.locator('[data-role="verse-start"]');
    await verseStart.fill("1");

    // Click Load passage
    const loadButton = page.locator('[data-role="load-button"]');
    await loadButton.click();

    // Wait for slides to appear
    await page.waitForFunction(
      () => {
        const slides = document.querySelectorAll('[data-role="slide-card"]');
        return slides.length > 0;
      },
      { timeout: 15_000 },
    );

    const slideCount = await page.locator('[data-role="slide-card"]').count();
    expect(slideCount).toBeGreaterThan(0);
  });

  // -----------------------------------------------------------------------
  // Slide trigger
  // -----------------------------------------------------------------------

  test("slide trigger sends to stage", async ({ page }) => {
    await navigateToBible(page);
    await clearBroadcast();

    if (!(await hasBibleData(page))) {
      // No Bible data available — skip gracefully
      return;
    }

    // Load a passage
    const firstBook = page.locator('[data-role="book-item"]').first();
    await firstBook.click();
    await page.locator('[data-role="load-button"]').click();

    // Wait for slides
    await page.waitForFunction(
      () => document.querySelectorAll('[data-role="slide-card"]').length > 0,
      { timeout: 15_000 },
    );

    // Click trigger zone on first slide
    const firstTrigger = page
      .locator('[data-role="slide-trigger-zone"]')
      .first();
    await firstTrigger.click();

    // Should show success toast
    await page.waitForFunction(
      () => {
        const toast = document.querySelector('[data-role="toast"]');
        return toast && toast.textContent?.includes("Triggered");
      },
      { timeout: 5_000 },
    );
  });

  // -----------------------------------------------------------------------
  // Slide selection
  // -----------------------------------------------------------------------

  test("slide selection toggles on click", async ({ page }) => {
    await navigateToBible(page);

    if (!(await hasBibleData(page))) {
      // No Bible data available — skip gracefully
      return;
    }

    // Load a passage
    const firstBook = page.locator('[data-role="book-item"]').first();
    await firstBook.click();
    await page.locator('[data-role="load-button"]').click();

    await page.waitForFunction(
      () => document.querySelectorAll('[data-role="slide-card"]').length > 0,
      { timeout: 15_000 },
    );

    // Click select zone on first slide
    const firstSelectZone = page
      .locator('[data-role="slide-select-zone"]')
      .first();
    await firstSelectZone.click();

    // Should be selected (is-selected class)
    const firstCard = page.locator('[data-role="slide-card"]').first();
    await expect(firstCard).toHaveClass(/is-selected/);

    // Selection count should update
    const selectionCount = page.locator('[data-role="selection-count"]');
    await expect(selectionCount).toHaveText("1 selected");
  });

  test("select all selects all slides", async ({ page }) => {
    await navigateToBible(page);

    if (!(await hasBibleData(page))) {
      // No Bible data available — skip gracefully
      return;
    }

    // Load a passage
    const firstBook = page.locator('[data-role="book-item"]').first();
    await firstBook.click();
    await page.locator('[data-role="load-button"]').click();

    await page.waitForFunction(
      () => document.querySelectorAll('[data-role="slide-card"]').length > 0,
      { timeout: 15_000 },
    );

    const totalSlides = await page.locator('[data-role="slide-card"]').count();

    // Click "Select all"
    await page.locator('[data-role="select-all-slides"]').click();

    const selectionCount = page.locator('[data-role="selection-count"]');
    await expect(selectionCount).toHaveText(`${totalSlides} selected`);
  });

  // -----------------------------------------------------------------------
  // Edit mode
  // -----------------------------------------------------------------------

  test("edit mode shows textareas in slide cards", async ({ page }) => {
    await navigateToBible(page);

    if (!(await hasBibleData(page))) {
      // No Bible data available — skip gracefully
      return;
    }

    // Load a passage
    const firstBook = page.locator('[data-role="book-item"]').first();
    await firstBook.click();
    await page.locator('[data-role="load-button"]').click();

    await page.waitForFunction(
      () => document.querySelectorAll('[data-role="slide-card"]').length > 0,
      { timeout: 15_000 },
    );

    // Switch to edit mode
    const editButton = page.locator(
      '[data-role="mode-toggle"][data-mode="edit"]',
    );
    await editButton.click();

    // Wait for edit textareas
    await page.waitForSelector('[data-role="slide-main-edit"]', {
      timeout: 5_000,
    });

    const editTextarea = page.locator('[data-role="slide-main-edit"]').first();
    await expect(editTextarea).toBeVisible();

    // Switch back to live mode
    const liveButton = page.locator(
      '[data-role="mode-toggle"][data-mode="live"]',
    );
    await liveButton.click();

    // Textareas should be gone, content should show
    await page.waitForSelector(".operator__slide-content", {
      timeout: 5_000,
    });
  });

  // -----------------------------------------------------------------------
  // Presentations (Prepared tab)
  // -----------------------------------------------------------------------

  test("prepared tab shows create button and empty state", async ({ page }) => {
    await navigateToBible(page);
    const bp = biblePanel(page);

    // Switch to Prepared tab
    await page.locator('[data-role="bible-tab"][data-tab="prepared"]').click();

    const createButton = bp.locator('[data-role="presentation-create"]');
    await expect(createButton).toBeVisible();
  });

  test("create presentation adds to list", async ({ page }) => {
    await navigateToBible(page);
    const bp = biblePanel(page);

    // Switch to Prepared tab
    await page.locator('[data-role="bible-tab"][data-tab="prepared"]').click();

    // Count existing presentations
    const initialCount = await bp
      .locator('[data-role="presentation-card"]')
      .count();

    // Click create
    await bp.locator('[data-role="presentation-create"]').click();

    // Wait for new presentation to appear
    await page.waitForFunction(
      (expected: number) => {
        const cards = document.querySelectorAll(
          '[data-view-panel="bible"] [data-role="presentation-card"]',
        );
        return cards.length > expected;
      },
      initialCount,
      { timeout: 10_000 },
    );

    const newCount = await bp
      .locator('[data-role="presentation-card"]')
      .count();
    expect(newCount).toBeGreaterThan(initialCount);
  });

  test("clicking presentation loads its slides in column", async ({ page }) => {
    await navigateToBible(page);
    const bp = biblePanel(page);

    // Switch to Prepared tab
    await page.locator('[data-role="bible-tab"][data-tab="prepared"]').click();

    // Ensure at least one presentation exists
    const presCards = bp.locator('[data-role="presentation-card"]');
    if ((await presCards.count()) === 0) {
      await bp.locator('[data-role="presentation-create"]').click();
      await page.waitForFunction(
        () =>
          document.querySelectorAll(
            '[data-view-panel="bible"] [data-role="presentation-card"]',
          ).length > 0,
        { timeout: 10_000 },
      );
    }

    // Click first presentation card
    const firstPres = presCards.first();
    await firstPres.click();

    // Should become active
    await expect(firstPres).toHaveClass(/is-active/);
  });

  test("delete presentation removes it from list", async ({ page }) => {
    await navigateToBible(page);
    const bp = biblePanel(page);

    // Switch to Prepared tab
    await page.locator('[data-role="bible-tab"][data-tab="prepared"]').click();

    // Create a fresh presentation to delete
    await bp.locator('[data-role="presentation-create"]').click();
    await page.waitForFunction(
      () =>
        document.querySelectorAll(
          '[data-view-panel="bible"] [data-role="presentation-card"]',
        ).length > 0,
      { timeout: 10_000 },
    );

    // Select the last created presentation
    const presCards = bp.locator('[data-role="presentation-card"]');
    const countBefore = await presCards.count();
    const lastPres = presCards.last();
    await lastPres.click();

    // Handle confirm dialog
    page.once("dialog", (dialog) => dialog.accept());

    // Click delete
    await bp.locator('[data-role="presentation-delete"]').click();

    // Wait for deletion
    await page.waitForFunction(
      (expected: number) => {
        const cards = document.querySelectorAll(
          '[data-view-panel="bible"] [data-role="presentation-card"]',
        );
        return cards.length < expected;
      },
      countBefore,
      { timeout: 10_000 },
    );

    const countAfter = await presCards.count();
    expect(countAfter).toBeLessThan(countBefore);
  });

  // -----------------------------------------------------------------------
  // Add slides to presentation
  // -----------------------------------------------------------------------

  test("add selected slides to presentation", async ({ page }) => {
    await navigateToBible(page);

    if (!(await hasBibleData(page))) {
      // No Bible data available — skip gracefully
      return;
    }

    // Load a passage
    const firstBook = page.locator('[data-role="book-item"]').first();
    await firstBook.click();
    await page.locator('[data-role="load-button"]').click();

    await page.waitForFunction(
      () => document.querySelectorAll('[data-role="slide-card"]').length > 0,
      { timeout: 15_000 },
    );

    // Select all slides
    await page.locator('[data-role="select-all-slides"]').click();

    // Create a presentation if none exist (via API for speed)
    const presResponse = await page.evaluate(async (url: string) => {
      const resp = await fetch(`${url}/bible/presentations`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ name: "E2E Test Pres" }),
      });
      return resp.json();
    }, baseURL);
    const presId = presResponse.id;

    // Select that presentation in the dropdown
    const presSelect = page.locator('[data-role="presentation-select"]');
    await presSelect.selectOption(presId);

    // Click "Add selected"
    await page.locator('[data-role="presentation-add"]').click();

    // Should show success toast
    await page.waitForFunction(
      () => {
        const toast = document.querySelector('[data-role="toast"]');
        return toast && toast.textContent?.includes("Added");
      },
      { timeout: 5_000 },
    );

    // Clean up: delete test presentation
    await page.evaluate(
      async ({ url, id }: { url: string; id: string }) => {
        await fetch(`${url}/bible/presentations/${id}`, { method: "DELETE" });
      },
      { url: baseURL, id: presId },
    );
  });

  // -----------------------------------------------------------------------
  // Settings tab: character limit
  // -----------------------------------------------------------------------

  test("settings tab has character limit input", async ({ page }) => {
    await navigateToBible(page);

    // Switch to Settings tab
    await page.locator('[data-role="bible-tab"][data-tab="settings"]').click();

    const charLimit = page.locator('[data-role="char-limit"]');
    await expect(charLimit).toBeVisible();

    // Should have a default value
    const value = await charLimit.inputValue();
    expect(parseInt(value)).toBeGreaterThan(0);
  });

  test("save preferences persists character limit", async ({ page }) => {
    await navigateToBible(page);

    // Switch to Settings tab
    await page.locator('[data-role="bible-tab"][data-tab="settings"]').click();

    const charLimit = page.locator('[data-role="char-limit"]');
    await charLimit.fill("400");

    // Manually trigger change event
    await charLimit.evaluate((el: HTMLInputElement) =>
      el.dispatchEvent(new Event("change", { bubbles: true })),
    );

    // Click save
    await page.locator('[data-role="save-preferences"]').click();

    // Should show success toast
    await page.waitForFunction(
      () => {
        const toast = document.querySelector('[data-role="toast"]');
        return toast && toast.textContent?.includes("Preferences saved");
      },
      { timeout: 5_000 },
    );

    // Reload page and verify persisted
    await navigateToBible(page);
    await page.locator('[data-role="bible-tab"][data-tab="settings"]').click();

    const savedValue = await page
      .locator('[data-role="char-limit"]')
      .inputValue();
    expect(parseInt(savedValue)).toBe(400);

    // Reset to default
    await page.locator('[data-role="char-limit"]').fill("320");
    await page
      .locator('[data-role="char-limit"]')
      .evaluate((el: HTMLInputElement) =>
        el.dispatchEvent(new Event("change", { bubbles: true })),
      );
    await page.locator('[data-role="save-preferences"]').click();
  });

  // -----------------------------------------------------------------------
  // Secondary translation
  // -----------------------------------------------------------------------

  test("secondary translation can be set", async ({ page }) => {
    await navigateToBible(page);

    const secondarySelect = page.locator('[data-role="secondary-translation"]');
    await expect(secondarySelect).toBeVisible({ timeout: 10_000 });

    const options = secondarySelect.locator("option");
    const count = await options.count();

    // Should have at least "None" + one translation
    expect(count).toBeGreaterThanOrEqual(2);

    // Select first non-None option
    if (count >= 2) {
      const secondOption = await options.nth(1).getAttribute("value");
      if (secondOption) {
        await secondarySelect.selectOption(secondOption);
        // Verify selection stuck
        const selected = await secondarySelect.inputValue();
        expect(selected).toBe(secondOption);
      }
    }
  });

  // -----------------------------------------------------------------------
  // Slides column layout
  // -----------------------------------------------------------------------

  test("slides column shows empty state initially", async ({ page }) => {
    await navigateToBible(page);
    const bp = biblePanel(page);

    const slidesColumn = bp.locator('[data-role="slides"]');
    await expect(slidesColumn).toBeVisible();

    const emptyState = slidesColumn.locator(".operator__slides-empty");
    await expect(emptyState).toBeVisible();
    await expect(emptyState).toHaveText("Load a passage to populate slides.");
  });

  test("prepared tab slides column shows empty state", async ({ page }) => {
    await navigateToBible(page);
    const bp = biblePanel(page);

    // Switch to Prepared tab
    await page.locator('[data-role="bible-tab"][data-tab="prepared"]').click();

    const slidesColumn = bp.locator('[data-role="slides"]');
    const emptyState = slidesColumn.locator(".operator__slides-empty");
    await expect(emptyState).toHaveText(
      "Select a presentation to view slides.",
    );
  });

  // -----------------------------------------------------------------------
  // Two-column layout
  // -----------------------------------------------------------------------

  test("two-column layout renders catalog and slides side-by-side at 320px", async ({
    page,
  }) => {
    await navigateToBible(page);
    const bp = biblePanel(page);

    const container = bp.locator(".operator__panel--bible");
    const catalog = bp.locator('[data-role="catalog"]');
    const slidesColumn = bp.locator('[data-role="slides-column"]');

    await expect(catalog).toBeVisible();
    await expect(slidesColumn).toBeVisible();

    await assertTwoColumnLayout(container, catalog, slidesColumn, {
      expectedLeftWidth: 320,
    });
  });
});
