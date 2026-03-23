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

    // Wait for translation options to load asynchronously
    await page.waitForFunction(
      () => {
        const select = document.querySelector('[data-role="main-translation"]');
        return select && select.querySelectorAll("option").length >= 1;
      },
      { timeout: 15_000 },
    );

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

  test("book items have proper button structure and chapter count badges", async ({
    page,
  }) => {
    await navigateToBible(page);

    await page.waitForFunction(
      () => document.querySelectorAll('[data-role="book-item"]').length > 0,
      { timeout: 10_000 },
    );

    const firstBook = page.locator('[data-role="book-item"]').first();

    // Book item should be a button with operator__list-button class
    await expect(firstBook).toHaveClass(/operator__list-button/);

    // Should be wrapped in a div.operator__list-item
    const wrapper = firstBook.locator("..");
    await expect(wrapper).toHaveClass(/operator__list-item/);

    // Should contain a label span and a chapter count meta span
    const label = firstBook.locator(".operator__list-label");
    await expect(label).toBeVisible();
    const labelText = await label.textContent();
    expect(labelText?.trim().length).toBeGreaterThan(0);

    const meta = firstBook.locator(".operator__list-meta");
    await expect(meta).toBeVisible();
    const metaText = await meta.textContent();
    // Meta should show chapter count like "50 ch."
    expect(metaText).toMatch(/\d+\s*ch\./);

    // Button should have pointer cursor (styled as clickable)
    const cursor = await firstBook.evaluate(
      (el) => window.getComputedStyle(el).cursor,
    );
    expect(cursor).toBe("pointer");
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

    // Wait for filtered list to update reactively
    await expect
      .poll(async () => page.locator('[data-role="book-item"]').count(), {
        timeout: 5_000,
      })
      .toBeLessThan(initialCount > 5 ? initialCount : initialCount + 1);

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
      test.skip(true, "No Bible data available");
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
      test.skip(true, "No Bible data available");
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

    // Verify actual stage content via active-slide API
    const activeSlide = await page.evaluate(async (url: string) => {
      const resp = await fetch(`${url}/bible/active-slide`);
      return resp.json();
    }, baseURL);

    expect(activeSlide).not.toBeNull();
    expect(activeSlide.mainText).toBeTruthy();
    expect(activeSlide.mainText.length).toBeGreaterThan(0);
    expect(activeSlide.mainReference).toBeTruthy();
  });

  // -----------------------------------------------------------------------
  // Slide selection
  // -----------------------------------------------------------------------

  test("slide selection toggles on click", async ({ page }) => {
    await navigateToBible(page);

    if (!(await hasBibleData(page))) {
      test.skip(true, "No Bible data available");
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
      test.skip(true, "No Bible data available");
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
      test.skip(true, "No Bible data available");
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
    await page.waitForSelector(".operator__slide-bodies--bible", {
      timeout: 5_000,
    });
  });

  test("slide cards use legacy content classes (operator__slide-bodies--bible)", async ({
    page,
  }) => {
    await navigateToBible(page);

    if (!(await hasBibleData(page))) {
      test.skip(true, "No Bible data available");
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

    const firstCard = page.locator('[data-role="slide-card"]').first();

    // Verify legacy class structure
    const bodiesSection = firstCard.locator(
      ".operator__slide-bodies.operator__slide-bodies--bible",
    );
    await expect(bodiesSection).toBeVisible();

    const mainText = firstCard.locator(
      ".operator__slide-text.operator__slide-text--main",
    );
    await expect(mainText).toBeVisible();

    // Main text should be centered and bold
    const textAlign = await mainText.evaluate(
      (el) => window.getComputedStyle(el).textAlign,
    );
    expect(textAlign).toBe("center");

    const fontWeight = await mainText.evaluate(
      (el) => window.getComputedStyle(el).fontWeight,
    );
    expect(parseInt(fontWeight)).toBeGreaterThanOrEqual(600);
  });

  test("slide cards show reference footer when reference exists", async ({
    page,
  }) => {
    await navigateToBible(page);

    if (!(await hasBibleData(page))) {
      test.skip(true, "No Bible data available");
      return;
    }

    const firstBook = page.locator('[data-role="book-item"]').first();
    await firstBook.click();
    await page.locator('[data-role="chapter-input"]').fill("1");
    await page.locator('[data-role="verse-start"]').fill("1");
    await page.locator('[data-role="load-button"]').click();

    await page.waitForFunction(
      () => document.querySelectorAll('[data-role="slide-card"]').length > 0,
      { timeout: 15_000 },
    );

    // Check that at least one slide has a reference footer
    const footers = page.locator(
      '[data-role="slide-card"] .operator__slide-footer .operator__slide-reference',
    );
    const footerCount = await footers.count();
    expect(footerCount).toBeGreaterThan(0);

    // Reference should have small italic styling
    const firstFooter = footers.first();
    const fontStyle = await firstFooter.evaluate(
      (el) => window.getComputedStyle(el).fontStyle,
    );
    expect(fontStyle).toBe("italic");
  });

  test("edit mode shows labeled textareas and reference inputs", async ({
    page,
  }) => {
    await navigateToBible(page);

    if (!(await hasBibleData(page))) {
      test.skip(true, "No Bible data available");
      return;
    }

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

    // Wait for editor section
    await page.waitForSelector(".operator__slide-editor--bible", {
      timeout: 5_000,
    });

    const firstCard = page.locator('[data-role="slide-card"]').first();

    // Editor section should use legacy class
    const editorSection = firstCard.locator(
      ".operator__slide-editor.operator__slide-editor--bible",
    );
    await expect(editorSection).toBeVisible();

    // Labels should be visible
    const mainLabel = editorSection
      .locator("label span")
      .filter({ hasText: "Main" })
      .first();
    await expect(mainLabel).toBeVisible();

    const transLabel = editorSection
      .locator("label span")
      .filter({ hasText: "Translation" })
      .first();
    await expect(transLabel).toBeVisible();

    // Reference inputs should exist in editor grid
    const mainRefInput = firstCard.locator('[data-role="slide-main-ref"]');
    await expect(mainRefInput).toBeVisible();

    const transRefInput = firstCard.locator(
      '[data-role="slide-translation-ref"]',
    );
    await expect(transRefInput).toBeVisible();

    // Editor grid should use 2-column layout
    const editorGrid = firstCard.locator(".operator__slide-editor-grid");
    const gridDisplay = await editorGrid.evaluate(
      (el) => window.getComputedStyle(el).display,
    );
    expect(gridDisplay).toBe("grid");

    // Header with trigger button should be visible
    const triggerBtn = firstCard.locator(
      ".operator__slide-header .operator__list-action--primary",
    );
    await expect(triggerBtn).toBeVisible();
    await expect(triggerBtn).toHaveText("Trigger");

    // Checkbox should be visible
    const checkbox = firstCard.locator(
      '.operator__slide-header input[data-role="slide-select"]',
    );
    await expect(checkbox).toBeVisible();
  });

  test("translation text has blue italic styling with secondary class", async ({
    page,
  }) => {
    await navigateToBible(page);

    if (!(await hasBibleData(page))) {
      test.skip(true, "No Bible data available");
      return;
    }

    // Select secondary translation first
    const secondarySelect = page.locator('[data-role="secondary-translation"]');
    await expect(secondarySelect).toBeVisible({ timeout: 10_000 });
    const options = secondarySelect.locator("option");
    const count = await options.count();
    if (count < 2) return; // Need at least one real translation

    const secondOption = await options.nth(1).getAttribute("value");
    if (secondOption) {
      await secondarySelect.selectOption(secondOption);
    }

    // Load a passage
    const firstBook = page.locator('[data-role="book-item"]').first();
    await firstBook.click();
    await page.locator('[data-role="load-button"]').click();

    await page.waitForFunction(
      () => document.querySelectorAll('[data-role="slide-card"]').length > 0,
      { timeout: 15_000 },
    );

    // Check for translation text with proper class
    const transText = page
      .locator(
        ".operator__slide-text--translation.operator__slide-text--secondary",
      )
      .first();

    if ((await transText.count()) > 0) {
      const fontStyle = await transText.evaluate(
        (el) => window.getComputedStyle(el).fontStyle,
      );
      expect(fontStyle).toBe("italic");
    }
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

    // Handle the prompt dialog for presentation name
    page.once("dialog", (dialog) => dialog.accept("Test Presentation"));

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
      page.once("dialog", (dialog) => dialog.accept("Test Pres"));
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
    page.once("dialog", (dialog) => dialog.accept("To Delete"));
    await bp.locator('[data-role="presentation-create"]').click();
    await page.waitForFunction(
      () =>
        document.querySelectorAll(
          '[data-view-panel="bible"] [data-role="presentation-card"]',
        ).length > 0,
      { timeout: 15_000 },
    );

    // Wait for the card list to stabilise after creation
    await page.waitForFunction(
      () => {
        const cards = document.querySelectorAll(
          '[data-view-panel="bible"] [data-role="presentation-card"]',
        );
        return cards.length > 0;
      },
      { timeout: 5_000 },
    );

    // Select the last created presentation
    const presCards = bp.locator('[data-role="presentation-card"]');
    const countBefore = await presCards.count();
    const lastPres = presCards.last();
    await lastPres.click();
    await expect(lastPres).toHaveClass(/is-active/, { timeout: 5_000 });

    // Open the presentation edit modal via the edit button on the active card
    await lastPres.locator('[data-role="presentation-edit"]').click();

    // Wait for modal to appear
    const modal = page.locator('[data-role="presentation-modal"]');
    await expect(modal).toBeVisible({ timeout: 5_000 });

    // Set up dialog handler BEFORE triggering the delete (confirm dialog)
    page.once("dialog", (dialog) => dialog.accept());

    // Click delete in the modal
    await modal.locator('[data-role="modal-delete"]').click();

    // Wait for deletion with extended timeout
    await page.waitForFunction(
      (expected: number) => {
        const cards = document.querySelectorAll(
          '[data-view-panel="bible"] [data-role="presentation-card"]',
        );
        return cards.length < expected;
      },
      countBefore,
      { timeout: 15_000 },
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
      test.skip(true, "No Bible data available");
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

    // Handle the prompt dialog for new presentation name
    page.once("dialog", (dialog) => dialog.accept("Test Slides Pres"));

    // Click "Add to new presentation" — creates a new presentation and appends slides
    await page.locator('[data-role="presentation-add"]').click();

    // Should show success toast
    await page.waitForFunction(
      () => {
        const toast = document.querySelector('[data-role="toast"]');
        return toast && toast.textContent?.includes("Added");
      },
      { timeout: 10_000 },
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

    // Wait for options to be populated (loaded asynchronously)
    await page.waitForFunction(
      () => {
        const sel = document.querySelector(
          '[data-role="secondary-translation"]',
        );
        return sel && sel.querySelectorAll("option").length >= 2;
      },
      { timeout: 10_000 },
    );

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

    // Wait for the operator--bible class on body (set reactively, may lag behind data-view)
    await page.waitForFunction(
      () => document.body.classList.contains("operator--bible"),
      { timeout: 5_000 },
    );

    // bp IS the .operator__panel--bible element (data-view-panel="bible")
    const catalog = bp.locator('[data-role="catalog"]');
    const slidesColumn = bp.locator('[data-role="slides-column"]');

    await expect(catalog).toBeVisible();
    await expect(slidesColumn).toBeVisible();

    await assertTwoColumnLayout(bp, catalog, slidesColumn, {
      expectedLeftWidth: 320,
    });
  });

  // -----------------------------------------------------------------------
  // Trigger sends to stage (verifying actual stage output)
  // -----------------------------------------------------------------------

  test("triggering a slide updates the stage display", async ({ page }) => {
    await navigateToBible(page);
    await clearBroadcast();

    if (!(await hasBibleData(page))) {
      test.skip(true, "No Bible data available");
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

    // Click trigger zone on first slide
    const firstTrigger = page
      .locator('[data-role="slide-trigger-zone"]')
      .first();
    await firstTrigger.click();

    // Wait for success toast
    await page.waitForFunction(
      () => {
        const toast = document.querySelector('[data-role="toast"]');
        return toast && toast.textContent?.includes("Triggered");
      },
      { timeout: 5_000 },
    );

    // Verify stage content via active-slide API (single-source-of-truth format)
    const activeSlide = await page.evaluate(async (url: string) => {
      const resp = await fetch(`${url}/bible/active-slide`);
      return resp.json();
    }, baseURL);

    expect(activeSlide).not.toBeNull();
    expect(activeSlide.mainText).toBeTruthy();
    expect(activeSlide.mainText.length).toBeGreaterThan(0);
    expect(activeSlide.mainReference).toBeTruthy();
  });

  // -----------------------------------------------------------------------
  // Header Bible preview
  // -----------------------------------------------------------------------

  test("header preview shows triggered Bible verse text", async ({ page }) => {
    await navigateToBible(page);
    await clearBroadcast();

    if (!(await hasBibleData(page))) {
      test.skip(true, "No Bible data available");
      return;
    }

    // In Bible view, bible-preview should be visible (even if empty)
    const biblePreview = page.locator('[data-role="bible-preview"]');
    await expect(biblePreview).toBeVisible();
    // Worship preview should be hidden
    const worshipPreview = page.locator('[data-role="worship-preview"]');
    await expect(worshipPreview).toBeHidden();

    // Initially shows "No active passage"
    await expect(biblePreview).toContainText("No active passage");

    // Load a passage
    const firstBook = page.locator('[data-role="book-item"]').first();
    await firstBook.click();
    await page.locator('[data-role="load-button"]').click();

    await page.waitForFunction(
      () => document.querySelectorAll('[data-role="slide-card"]').length > 0,
      { timeout: 15_000 },
    );

    // Trigger first slide
    const firstTrigger = page
      .locator('[data-role="slide-trigger-zone"]')
      .first();
    await firstTrigger.click();

    // Wait for success toast
    await page.waitForFunction(
      () => {
        const toast = document.querySelector('[data-role="toast"]');
        return toast && toast.textContent?.includes("Triggered");
      },
      { timeout: 5_000 },
    );

    // Wait for bible preview to update with verse text (via WS event)
    await page.waitForFunction(
      () => {
        const preview = document.querySelector('[data-role="bible-preview"]');
        return (
          preview &&
          preview.getAttribute("data-active") === "true" &&
          !preview.textContent?.includes("No active passage")
        );
      },
      { timeout: 5_000 },
    );

    // Bible preview should contain verse text and reference
    const previewText = page.locator('[data-role="bible-preview-text"]');
    await expect(previewText).toBeVisible();
    const textContent = await previewText.textContent();
    expect(textContent).toBeTruthy();
    expect(textContent!.length).toBeGreaterThan(0);

    const previewRef = page.locator('[data-role="bible-preview-ref"]');
    await expect(previewRef).toBeVisible();
    const refContent = await previewRef.textContent();
    expect(refContent).toBeTruthy();

    // Clear the broadcast
    await page.evaluate(async (url: string) => {
      await fetch(`${url}/bible/clear`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: "{}",
      });
    }, baseURL);

    // Wait for preview to show empty state again
    await page.waitForFunction(
      () => {
        const preview = document.querySelector('[data-role="bible-preview"]');
        return (
          preview &&
          preview.getAttribute("data-active") === "false" &&
          preview.textContent?.includes("No active passage")
        );
      },
      { timeout: 5_000 },
    );

    await expect(biblePreview).toContainText("No active passage");

    // Switch to worship view — worship preview should show, bible preview hidden
    const worshipButton = page.locator(
      '[data-role="view-toggle"][data-view="worship"]',
    );
    await worshipButton.click();

    await page.waitForFunction(
      () => document.body.getAttribute("data-view") === "worship",
      { timeout: 5_000 },
    );

    await expect(worshipPreview).toBeVisible();
    await expect(biblePreview).toBeHidden();
  });

  test("trigger button in edit mode sends current (edited) text to stage", async ({
    page,
  }) => {
    await navigateToBible(page);
    await clearBroadcast();

    if (!(await hasBibleData(page))) {
      test.skip(true, "No Bible data available");
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
    await page.locator('[data-role="mode-toggle"][data-mode="edit"]').click();
    await page.waitForSelector('[data-role="slide-main-edit"]', {
      timeout: 5_000,
    });

    // Edit the text
    const mainEdit = page.locator('[data-role="slide-main-edit"]').first();
    await mainEdit.fill("EDITED TEXT FOR TRIGGER TEST");

    // Click the Trigger button (inside the edit header, NOT the trigger zone)
    const triggerBtn = page.locator('[data-role="slide-trigger"]').first();
    await triggerBtn.click();

    // Verify success toast
    await page.waitForFunction(
      () => {
        const toast = document.querySelector('[data-role="toast"]');
        return toast && toast.textContent?.includes("Triggered");
      },
      { timeout: 5_000 },
    );

    // Verify the active-slide has the EDITED text (not just toast)
    const activeSlide = await page.evaluate(async (url: string) => {
      const resp = await fetch(`${url}/bible/active-slide`);
      return resp.json();
    }, baseURL);

    expect(activeSlide).not.toBeNull();
    expect(activeSlide.mainText).toBe("EDITED TEXT FOR TRIGGER TEST");

    await clearBroadcast();
  });

  test("triggering from trigger zone does not toggle selection", async ({
    page,
  }) => {
    await navigateToBible(page);

    if (!(await hasBibleData(page))) {
      test.skip(true, "No Bible data available");
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

    // Click trigger zone on first slide
    const firstTrigger = page
      .locator('[data-role="slide-trigger-zone"]')
      .first();
    await firstTrigger.click();

    // Slide should NOT be selected (trigger zone is separate from select zone)
    const firstCard = page.locator('[data-role="slide-card"]').first();
    // Use poll to ensure state has settled after click
    await expect
      .poll(
        async () =>
          firstCard.evaluate((el) => el.classList.contains("is-selected")),
        { timeout: 3_000 },
      )
      .toBe(false);
  });

  // Clear broadcast button was removed from Bible UI (user clears via stage preview).
  // Clear functionality is verified via API in wasm-bible-stage.spec.ts.

  // -----------------------------------------------------------------------
  // Bible search
  // -----------------------------------------------------------------------

  test("bible search input is visible and functional", async ({ page }) => {
    await navigateToBible(page);

    const searchInput = page.locator('[data-role="bible-search-input"]');
    await expect(searchInput).toBeVisible();
  });

  test("search requires minimum 3 characters", async ({ page }) => {
    await navigateToBible(page);

    const searchInput = page.locator('[data-role="bible-search-input"]');
    await searchInput.fill("ab");

    // No results container should appear (wait for debounce to settle)
    await expect
      .poll(
        async () => page.locator('[data-role="bible-search-results"]').count(),
        { timeout: 3_000 },
      )
      .toBe(0);
  });

  test("search results appear with 3+ characters", async ({ page }) => {
    await navigateToBible(page);

    if (!(await hasBibleData(page))) {
      test.skip(true, "No Bible data available");
      return;
    }

    const searchInput = page.locator('[data-role="bible-search-input"]');
    await searchInput.fill("love");

    // Wait for search results to appear (debounce + API call)
    await page.waitForSelector('[data-role="bible-search-results"]', {
      timeout: 10_000,
    });

    const results = page.locator('[data-role="bible-search-results"]');
    await expect(results).toBeVisible();
  });

  test("clicking search result sets reference inputs", async ({ page }) => {
    await navigateToBible(page);

    if (!(await hasBibleData(page))) {
      test.skip(true, "No Bible data available");
      return;
    }

    const searchInput = page.locator('[data-role="bible-search-input"]');
    await searchInput.fill("love");

    // Wait for results
    await page.waitForSelector('[data-role="bible-search-result"]', {
      timeout: 10_000,
    });

    const firstResult = page
      .locator('[data-role="bible-search-result"]')
      .first();
    if ((await firstResult.count()) > 0) {
      await firstResult.click();

      // Search should be cleared
      await expect(searchInput).toHaveValue("", { timeout: 3_000 });

      // A book should be selected
      const activeBook = page.locator(
        '[data-role="book-item"][data-active="true"]',
      );
      const count = await activeBook.count();
      expect(count).toBeGreaterThanOrEqual(1);
    }
  });

  test("search clear button clears results", async ({ page }) => {
    await navigateToBible(page);

    if (!(await hasBibleData(page))) {
      test.skip(true, "No Bible data available");
      return;
    }

    const searchInput = page.locator('[data-role="bible-search-input"]');
    await searchInput.fill("love");

    await page.waitForSelector('[data-role="bible-search-results"]', {
      timeout: 10_000,
    });

    const clearBtn = page.locator('[data-role="bible-search-clear"]');
    await clearBtn.click();

    await expect(searchInput).toHaveValue("", { timeout: 3_000 });

    await expect
      .poll(
        async () => page.locator('[data-role="bible-search-results"]').count(),
        { timeout: 3_000 },
      )
      .toBe(0);
  });

  // -----------------------------------------------------------------------
  // Loaded passages history
  // -----------------------------------------------------------------------

  test("loading a passage adds it to history", async ({ page }) => {
    await navigateToBible(page);

    if (!(await hasBibleData(page))) {
      test.skip(true, "No Bible data available");
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

    // History should now appear
    const history = page.locator('[data-role="bible-history"]');
    await expect(history).toBeVisible();

    const historyItems = page.locator('[data-role="bible-history-item"]');
    const count = await historyItems.count();
    expect(count).toBeGreaterThanOrEqual(1);
  });

  test("clicking history item sets reference inputs", async ({ page }) => {
    await navigateToBible(page);

    if (!(await hasBibleData(page))) {
      test.skip(true, "No Bible data available");
      return;
    }

    // Load first passage
    const firstBook = page.locator('[data-role="book-item"]').first();
    await firstBook.click();
    await page.locator('[data-role="chapter-input"]').fill("2");
    await page
      .locator('[data-role="chapter-input"]')
      .evaluate((el: HTMLInputElement) =>
        el.dispatchEvent(new Event("change", { bubbles: true })),
      );
    await page.locator('[data-role="load-button"]').click();

    await page.waitForFunction(
      () => document.querySelectorAll('[data-role="slide-card"]').length > 0,
      { timeout: 15_000 },
    );

    // Change to chapter 1
    await page.locator('[data-role="chapter-input"]').fill("1");
    await page
      .locator('[data-role="chapter-input"]')
      .evaluate((el: HTMLInputElement) =>
        el.dispatchEvent(new Event("change", { bubbles: true })),
      );

    // Click history item (should be chapter 2)
    const historyItem = page
      .locator('[data-role="bible-history-item"]')
      .first();
    if ((await historyItem.count()) > 0) {
      await historyItem.click();

      // Chapter input should be set back to 2
      const chapterVal = await page
        .locator('[data-role="chapter-input"]')
        .inputValue();
      expect(chapterVal).toBe("2");
    }
  });

  // -----------------------------------------------------------------------
  // Delete slide from prepared presentation
  // -----------------------------------------------------------------------

  test("delete slide from prepared presentation", async ({ page }) => {
    await navigateToBible(page);

    if (!(await hasBibleData(page))) {
      test.skip(true, "No Bible data available");
      return;
    }

    // Create a presentation with slides
    const presResponse = await page.evaluate(async (url: string) => {
      const resp = await fetch(`${url}/bible/presentations`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ name: "Delete Slide Test" }),
      });
      return resp.json();
    }, baseURL);
    const presId = presResponse.id;

    // Add some slides
    await page.evaluate(
      async ({ url, id }: { url: string; id: string }) => {
        await fetch(`${url}/bible/presentations/${id}/append`, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            slides: [
              { main: "Slide 1", translation: "", stage: "" },
              { main: "Slide 2", translation: "", stage: "" },
              { main: "Slide 3", translation: "", stage: "" },
            ],
          }),
        });
      },
      { url: baseURL, id: presId },
    );

    // Switch to Prepared tab
    await page.locator('[data-role="bible-tab"][data-tab="prepared"]').click();

    // Wait for presentation list to include our new one and click it
    await page.waitForFunction(
      (id: string) => {
        const card = document.querySelector(`[data-presentation-id="${id}"]`);
        return card !== null;
      },
      presId,
      { timeout: 10_000 },
    );

    // Reload presentations by switching tabs
    await page.locator('[data-role="bible-tab"][data-tab="live"]').click();
    await page.locator('[data-role="bible-tab"][data-tab="prepared"]').click();

    await page.waitForFunction(
      (id: string) => {
        const card = document.querySelector(`[data-presentation-id="${id}"]`);
        return card !== null;
      },
      presId,
      { timeout: 10_000 },
    );

    await page.locator(`[data-presentation-id="${presId}"]`).click();

    // Wait for slides to load
    await page.waitForFunction(
      () => document.querySelectorAll('[data-role="slide-card"]').length >= 3,
      { timeout: 10_000 },
    );

    const countBefore = await page.locator('[data-role="slide-card"]').count();

    // Handle confirm dialog
    page.once("dialog", (dialog) => dialog.accept());

    // Click delete on first slide
    const deleteBtn = page.locator('[data-role="slide-delete"]').first();
    await deleteBtn.click();

    // Wait for slide count to decrease
    await page.waitForFunction(
      (expected: number) =>
        document.querySelectorAll('[data-role="slide-card"]').length < expected,
      countBefore,
      { timeout: 10_000 },
    );

    const countAfter = await page.locator('[data-role="slide-card"]').count();
    expect(countAfter).toBeLessThan(countBefore);

    // Cleanup
    await page.evaluate(
      async ({ url, id }: { url: string; id: string }) => {
        await fetch(`${url}/bible/presentations/${id}`, { method: "DELETE" });
      },
      { url: baseURL, id: presId },
    );
  });

  // -----------------------------------------------------------------------
  // Drag-drop reorder slides in prepared presentation
  // -----------------------------------------------------------------------

  test("prepared slides are draggable", async ({ page }) => {
    await navigateToBible(page);

    if (!(await hasBibleData(page))) {
      test.skip(true, "No Bible data available");
      return;
    }

    // Create a presentation with slides via API
    const presResponse = await page.evaluate(async (url: string) => {
      const resp = await fetch(`${url}/bible/presentations`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ name: "Drag Test" }),
      });
      return resp.json();
    }, baseURL);
    const presId = presResponse.id;

    await page.evaluate(
      async ({ url, id }: { url: string; id: string }) => {
        await fetch(`${url}/bible/presentations/${id}/append`, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            slides: [
              { main: "First", translation: "", stage: "" },
              { main: "Second", translation: "", stage: "" },
            ],
          }),
        });
      },
      { url: baseURL, id: presId },
    );

    // Switch to Prepared tab and select the presentation
    await page.locator('[data-role="bible-tab"][data-tab="prepared"]').click();

    // Reload to get fresh data
    await page.locator('[data-role="bible-tab"][data-tab="live"]').click();
    await page.locator('[data-role="bible-tab"][data-tab="prepared"]').click();

    await page.waitForFunction(
      (id: string) =>
        document.querySelector(`[data-presentation-id="${id}"]`) !== null,
      presId,
      { timeout: 10_000 },
    );

    await page.locator(`[data-presentation-id="${presId}"]`).click();

    await page.waitForFunction(
      () => document.querySelectorAll('[data-role="slide-card"]').length >= 2,
      { timeout: 10_000 },
    );

    // Verify slides have draggable attribute
    const firstSlide = page.locator('[data-role="slide-card"]').first();
    await expect(firstSlide).toHaveAttribute("draggable", "true");

    // Cleanup
    await page.evaluate(
      async ({ url, id }: { url: string; id: string }) => {
        await fetch(`${url}/bible/presentations/${id}`, { method: "DELETE" });
      },
      { url: baseURL, id: presId },
    );
  });

  // -----------------------------------------------------------------------
  // Stage output dropdown
  // -----------------------------------------------------------------------

  test("stage output dropdown has options and is visible in bible view", async ({
    page,
  }) => {
    await navigateToBible(page);

    const stageSelect = page.locator('[data-role="stage-layout-select"]');
    await expect(stageSelect).toBeVisible();

    // Wait for options to load
    await page.waitForFunction(
      () => {
        const select = document.querySelector(
          '[data-role="stage-layout-select"]',
        ) as HTMLSelectElement;
        return select && select.options.length > 0;
      },
      { timeout: 10_000 },
    );

    const optionCount = await stageSelect.locator("option").count();
    expect(optionCount).toBeGreaterThanOrEqual(1);
  });

  // -----------------------------------------------------------------------
  // End-to-end workflow
  // -----------------------------------------------------------------------

  test("full bible workflow: load, trigger, add to presentation, trigger prepared", async ({
    page,
  }) => {
    await navigateToBible(page);
    await clearBroadcast();

    if (!(await hasBibleData(page))) {
      test.skip(true, "No Bible data available");
      return;
    }

    // 1. Load a passage
    const firstBook = page.locator('[data-role="book-item"]').first();
    await firstBook.click();
    await page.locator('[data-role="load-button"]').click();

    await page.waitForFunction(
      () => document.querySelectorAll('[data-role="slide-card"]').length > 0,
      { timeout: 15_000 },
    );

    // 2. Trigger first slide
    const firstTrigger = page
      .locator('[data-role="slide-trigger-zone"]')
      .first();
    await firstTrigger.click();

    await page.waitForFunction(
      () => {
        const toast = document.querySelector('[data-role="toast"]');
        return toast && toast.textContent?.includes("Triggered");
      },
      { timeout: 5_000 },
    );

    // 3. Select all slides
    await page.locator('[data-role="select-all-slides"]').click();

    // 4. Click "Add to new presentation" — handle name prompt, then creates + appends
    page.once("dialog", (dialog) => dialog.accept("Full Workflow Pres"));
    await page.locator('[data-role="presentation-add"]').click();

    await page.waitForFunction(
      () => {
        const toast = document.querySelector('[data-role="toast"]');
        return toast && toast.textContent?.includes("Added");
      },
      { timeout: 10_000 },
    );

    // 5. Switch to Prepared tab and select the newly created presentation
    await page.locator('[data-role="bible-tab"][data-tab="prepared"]').click();

    // Wait for any presentation to appear
    await page.waitForFunction(
      () => document.querySelector("[data-presentation-id]") !== null,
      { timeout: 10_000 },
    );

    // Click the last presentation (the newly created one)
    const allPres = page.locator("[data-presentation-id]");
    await allPres.last().click();

    // 7. Wait for prepared slides to appear
    await page.waitForFunction(
      () => document.querySelectorAll('[data-role="slide-card"]').length > 0,
      { timeout: 10_000 },
    );

    // 8. Trigger first prepared slide
    await clearBroadcast();
    const preparedTrigger = page
      .locator('[data-role="slide-trigger-zone"]')
      .first();
    await preparedTrigger.click();

    await page.waitForFunction(
      () => {
        const toast = document.querySelector('[data-role="toast"]');
        return toast && toast.textContent?.includes("Triggered");
      },
      { timeout: 5_000 },
    );

    // 9. Verify stage content via active-slide API
    const activeSlide = await page.evaluate(async (url: string) => {
      const resp = await fetch(`${url}/bible/active-slide`);
      return resp.json();
    }, baseURL);
    expect(activeSlide).not.toBeNull();
    expect(activeSlide.mainText).toBeTruthy();

    // 10. Clear broadcast
    await page.locator('[data-role="bible-tab"][data-tab="live"]').click();
    await page.locator('[data-role="clear-broadcast"]').click();

    await page.waitForFunction(
      () => {
        const toast = document.querySelector('[data-role="toast"]');
        return toast && toast.textContent?.includes("cleared");
      },
      { timeout: 5_000 },
    );

    // Cleanup
    await page.evaluate(
      async ({ url, id }: { url: string; id: string }) => {
        await fetch(`${url}/bible/presentations/${id}`, { method: "DELETE" });
      },
      { url: baseURL, id: presId },
    );
  });

  // -----------------------------------------------------------------------
  // Preferences round-trip
  // -----------------------------------------------------------------------

  test("bible preferences API round-trip with all fields", async ({ page }) => {
    await navigateToBible(page);

    // Set specific preferences via API
    await page.evaluate(async (url: string) => {
      await fetch(`${url}/bible/preferences`, {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          characterLimit: 250,
        }),
      });
    }, baseURL);

    // Navigate to settings tab
    await page.locator('[data-role="bible-tab"][data-tab="settings"]').click();

    // Reload page to pick up saved preferences
    await navigateToBible(page);
    await page.locator('[data-role="bible-tab"][data-tab="settings"]').click();

    const charLimit = page.locator('[data-role="char-limit"]');
    const value = await charLimit.inputValue();
    expect(parseInt(value)).toBe(250);

    // Reset to default
    await page.evaluate(async (url: string) => {
      await fetch(`${url}/bible/preferences`, {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          characterLimit: 320,
        }),
      });
    }, baseURL);
  });

  // -----------------------------------------------------------------------
  // Add empty slide in prepared tab
  // -----------------------------------------------------------------------

  test("add empty slide button in prepared tab works", async ({ page }) => {
    await navigateToBible(page);

    if (!(await hasBibleData(page))) {
      test.skip(true, "No Bible data available");
      return;
    }

    // Create a presentation via API
    const presResponse = await page.evaluate(async (url: string) => {
      const resp = await fetch(`${url}/bible/presentations`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ name: "Empty Slide Test" }),
      });
      return resp.json();
    }, baseURL);
    const presId = presResponse.id;

    // Switch to Prepared tab
    await page.locator('[data-role="bible-tab"][data-tab="prepared"]').click();

    // Reload to get fresh data
    await page.locator('[data-role="bible-tab"][data-tab="live"]').click();
    await page.locator('[data-role="bible-tab"][data-tab="prepared"]').click();

    await page.waitForFunction(
      (id: string) =>
        document.querySelector(`[data-presentation-id="${id}"]`) !== null,
      presId,
      { timeout: 10_000 },
    );

    await page.locator(`[data-presentation-id="${presId}"]`).click();

    // Click add empty slide
    const addBtn = page.locator('[data-role="add-empty-slide"]');
    await addBtn.click();

    // Wait for slide to appear
    await page.waitForFunction(
      () => document.querySelectorAll('[data-role="slide-card"]').length > 0,
      { timeout: 10_000 },
    );

    const slideCount = await page.locator('[data-role="slide-card"]').count();
    expect(slideCount).toBeGreaterThanOrEqual(1);

    // Cleanup
    await page.evaluate(
      async ({ url, id }: { url: string; id: string }) => {
        await fetch(`${url}/bible/presentations/${id}`, { method: "DELETE" });
      },
      { url: baseURL, id: presId },
    );
  });

  // -----------------------------------------------------------------------
  // URL-based navigation
  // -----------------------------------------------------------------------

  test("direct navigation to /ui/operator/bible opens bible view", async ({
    page,
  }) => {
    await page.goto(`${baseURL}/ui/operator/bible`);
    await page.waitForSelector('[data-wasm-ready="true"]', { timeout: 30_000 });

    // Bible view should be active
    await page.waitForFunction(
      () => document.body.getAttribute("data-view") === "bible",
      { timeout: 5_000 },
    );

    // Bible tab should show as active
    const bibleButton = page.locator(
      '[data-role="view-toggle"][data-view="bible"]',
    );
    await expect(bibleButton).toHaveAttribute("data-active", "true");
  });

  test("clicking bible tab updates URL to /ui/operator/bible", async ({
    page,
  }) => {
    await page.goto(`${baseURL}/ui/operator`);
    await page.waitForSelector('[data-role="library-list"]', {
      timeout: 30_000,
    });

    // Click Bible tab
    const bibleButton = page.locator(
      '[data-role="view-toggle"][data-view="bible"]',
    );
    await bibleButton.click();
    await page.waitForFunction(
      () => document.body.getAttribute("data-view") === "bible",
      { timeout: 5_000 },
    );

    // URL should now end with /ui/operator/bible
    expect(page.url()).toContain("/ui/operator/bible");
  });

  test("clicking worship tab updates URL to /ui/operator", async ({ page }) => {
    // Start at bible view via URL
    await page.goto(`${baseURL}/ui/operator/bible`);
    await page.waitForSelector('[data-wasm-ready="true"]', { timeout: 30_000 });
    await page.waitForFunction(
      () => document.body.getAttribute("data-view") === "bible",
      { timeout: 5_000 },
    );

    // Click Worship tab
    const worshipButton = page.locator(
      '[data-role="view-toggle"][data-view="worship"]',
    );
    await worshipButton.click();
    await page.waitForFunction(
      () => document.body.getAttribute("data-view") === "worship",
      { timeout: 5_000 },
    );

    // URL should be /ui/operator (no subpath)
    const url = new URL(page.url());
    expect(url.pathname).toBe("/ui/operator");
  });

  test("browser back button returns to previous view and URL", async ({
    page,
  }) => {
    await page.goto(`${baseURL}/ui/operator`);
    await page.waitForSelector('[data-role="library-list"]', {
      timeout: 30_000,
    });

    // Navigate to bible via tab click
    const bibleButton = page.locator(
      '[data-role="view-toggle"][data-view="bible"]',
    );
    await bibleButton.click();
    await page.waitForFunction(
      () => document.body.getAttribute("data-view") === "bible",
      { timeout: 5_000 },
    );
    expect(page.url()).toContain("/ui/operator/bible");

    // Press browser back
    await page.goBack();

    // Should return to worship view
    await page.waitForFunction(
      () => document.body.getAttribute("data-view") === "worship",
      { timeout: 5_000 },
    );
    const url = new URL(page.url());
    expect(url.pathname).toBe("/ui/operator");
  });

  test("/ui/bible redirects to /ui/operator/bible", async ({ page }) => {
    // Navigate to legacy /ui/bible URL
    const response = await page.goto(`${baseURL}/ui/bible`);

    // Should end up at /ui/operator/bible after redirect
    expect(page.url()).toContain("/ui/operator/bible");

    // Bible view should be active (WASM loaded)
    await page.waitForSelector('[data-wasm-ready="true"]', { timeout: 30_000 });
    await page.waitForFunction(
      () => document.body.getAttribute("data-view") === "bible",
      { timeout: 5_000 },
    );
  });

  test("direct navigation to /ui/operator opens default worship view", async ({
    page,
  }) => {
    // Clear session storage to ensure no saved view
    await page.goto(`${baseURL}/ui/operator`);
    await page.waitForSelector('[data-wasm-ready="true"]', { timeout: 30_000 });
    await page.evaluate(() => sessionStorage.clear());

    // Reload to get fresh state
    await page.goto(`${baseURL}/ui/operator`);
    await page.waitForSelector('[data-wasm-ready="true"]', { timeout: 30_000 });

    // Should default to worship view
    await page.waitForFunction(
      () => document.body.getAttribute("data-view") === "worship",
      { timeout: 5_000 },
    );
    const url = new URL(page.url());
    expect(url.pathname).toBe("/ui/operator");
  });
});
