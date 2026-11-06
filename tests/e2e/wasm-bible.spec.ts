/**
 * WASM Operator Bible Tests
 *
 * Tests Bible functionality in the WASM operator: tab navigation, book/verse
 * selection, slide loading, trigger, selection, presentations, preferences.
 */

import { test, expect } from "@playwright/test";
import {
  assertTwoColumnLayout,
  attachConsoleErrorCollector,
  deriveTestConfig,
  refreshDevData,
  selectPassage,
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
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });

  // Navigate to bible view
  const bibleButton = page.locator(
    '[data-role="view-toggle"][data-view="bible"]',
  );
  if ((await bibleButton.count()) > 0) {
    await bibleButton.click();
  } else {
    const bibleTab = page.locator('button:has-text("Biblia")').first();
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

/** A button of the console's translation bar (`kind` = main | secondary, "" = žiadny). */
function translationButton(
  page: import("@playwright/test").Page,
  kind: "main" | "secondary",
  code: string,
) {
  return page.locator(
    `[data-role="console-translation"][data-kind="${kind}"][data-translation="${code}"]`,
  );
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
    await page.waitForSelector('body[data-wasm-ready="true"]', {
      timeout: 30_000,
    });

    const bibleButton = page.locator(
      '[data-role="view-toggle"][data-view="bible"]',
    );
    if ((await bibleButton.count()) > 0) {
      await expect(bibleButton).toBeVisible();
    } else {
      const bibleTab = page.locator('button:has-text("Biblia")').first();
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
  // Live tab: the console's translation buttons (#850 B2c replaced the selects)
  // -----------------------------------------------------------------------

  test("translation buttons are visible: a main row and a second row with žiadny", async ({
    page,
  }) => {
    await navigateToBible(page);

    const bar = page.locator('[data-role="console-translations"]');
    await expect(bar).toBeVisible({ timeout: 10_000 });
    const main = bar.locator('[data-role="console-translation"][data-kind="main"]');
    await expect(main.first()).toBeVisible();
    expect(await main.count()).toBeGreaterThanOrEqual(2);
    await expect(main.filter({ hasText: "SEB" })).toBeVisible();
    const none = bar.locator(
      '[data-role="console-translation"][data-kind="secondary"][data-translation=""]',
    );
    await expect(none).toHaveText("žiadny");
    // Exactly one main translation is on.
    await expect(main.and(page.locator('[aria-pressed="true"]'))).toHaveCount(1);
  });

  // -----------------------------------------------------------------------
  // Live tab: the console loads a passage into the slide grid
  // -----------------------------------------------------------------------

  test("loading a passage generates slides", async ({ page }) => {
    await navigateToBible(page);
    await selectPassage(page, "jan 1:1");
    const slideCount = await page.locator('[data-role="slide-card"]').count();
    expect(slideCount).toBeGreaterThan(0);
  });

  // -----------------------------------------------------------------------
  // Slide trigger
  // -----------------------------------------------------------------------

  test("slide trigger sends to stage", async ({ page }) => {
    await navigateToBible(page);
    await clearBroadcast();


    // Load a passage
    await selectPassage(page, "jan 1");

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
        return toast && toast.textContent?.includes("Zobrazené na stene");
      },
      { timeout: 5_000 },
    );

    // Verify actual stage content via active-slide API (poll for propagation)
    await expect(async () => {
      const activeSlide = await page.evaluate(async (url: string) => {
        const resp = await fetch(`${url}/bible/active-slide`);
        return resp.json();
      }, baseURL);
      expect(activeSlide).not.toBeNull();
      expect(activeSlide.mainText).toBeTruthy();
      expect(activeSlide.mainText.length).toBeGreaterThan(0);
      expect(activeSlide.mainReference).toBeTruthy();
    }).toPass({ timeout: 10_000, intervals: [500] });
  });

  // -----------------------------------------------------------------------
  // Slide selection
  // -----------------------------------------------------------------------

  test("slide selection toggles on click", async ({ page }) => {
    await navigateToBible(page);


    // Load a passage
    await selectPassage(page, "jan 1");

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
    await expect(selectionCount).toHaveText("Vybrané: 1");
  });

  test("select all selects all slides", async ({ page }) => {
    await navigateToBible(page);


    // Load a passage
    await selectPassage(page, "jan 1");

    await page.waitForFunction(
      () => document.querySelectorAll('[data-role="slide-card"]').length > 0,
      { timeout: 15_000 },
    );

    const totalSlides = await page.locator('[data-role="slide-card"]').count();

    // Click "Select all"
    await page.locator('[data-role="select-all-slides"]').click();

    const selectionCount = page.locator('[data-role="selection-count"]');
    await expect(selectionCount).toHaveText(`Vybrané: ${totalSlides}`);
  });

  // -----------------------------------------------------------------------
  // Edit mode
  // -----------------------------------------------------------------------

  test("edit mode shows textareas in slide cards", async ({ page }) => {
    await navigateToBible(page);


    // Load a passage
    await selectPassage(page, "jan 1");

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


    // Load a passage
    await selectPassage(page, "jan 1");

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


    await selectPassage(page, "jan 1:1");

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


    await selectPassage(page, "jan 1");

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
      .filter({ hasText: "Hlavný text" })
      .first();
    await expect(mainLabel).toBeVisible();

    const transLabel = editorSection
      .locator("label span")
      .filter({ hasText: "Preklad" })
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
    await expect(triggerBtn).toHaveText("Zobraziť");

    // Checkbox should be present in the DOM (may be visually hidden by custom label styling)
    const checkbox = firstCard.locator(
      '.operator__slide-header input[data-role="slide-select"]',
    );
    await expect(checkbox).toHaveCount(1);
  });

  test("translation text has blue italic styling with secondary class", async ({
    page,
  }) => {
    await navigateToBible(page);


    // SEB under main, KJV as the line below.
    await translationButton(page, "main", "slk-seb").click();
    await translationButton(page, "secondary", "eng-kjv").click();
    await expect(translationButton(page, "secondary", "eng-kjv")).toHaveAttribute(
      "aria-pressed",
      "true",
    );

    // Load a passage
    await selectPassage(page, "jan 1");

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
  // Unified slide editor (Prepared tab)
  // -----------------------------------------------------------------------

  test("prepared tab edit mode shows Main Reference and Translation Reference fields", async ({
    page,
  }) => {
    await navigateToBible(page);

    // Create a presentation with a slide via API
    const presResponse = await page.evaluate(async (url: string) => {
      const resp = await fetch(`${url}/bible/presentations`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ name: "Editor Fields Test" }),
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
              {
                bibleMain: "Test verse text",
                bibleTranslation: "Translated text",
                bibleMainReference: "Gen 1:1",
                bibleTranslationReference: "",
              },
            ],
          }),
        });
      },
      { url: baseURL, id: presId },
    );

    // Switch to Prepared tab and load presentation
    await page.locator('[data-role="bible-tab"][data-tab="prepared"]').click();
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
      () => document.querySelectorAll('[data-role="slide-card"]').length > 0,
      { timeout: 10_000 },
    );

    // Switch to edit mode
    await page.locator('[data-role="mode-toggle"][data-mode="edit"]').click();
    await page.waitForSelector('[data-role="slide-main-edit"]', {
      timeout: 5_000,
    });

    const firstCard = page.locator('[data-role="slide-card"]').first();

    // Main Reference and Translation Reference inputs should be visible
    const mainRefInput = firstCard.locator('[data-role="slide-main-ref"]');
    await expect(mainRefInput).toBeVisible();
    await expect(mainRefInput).toHaveAttribute("data-field", "main-ref");

    const transRefInput = firstCard.locator(
      '[data-role="slide-translation-ref"]',
    );
    await expect(transRefInput).toBeVisible();
    await expect(transRefInput).toHaveAttribute(
      "data-field",
      "translation-ref",
    );

    // Labels should include "Odkaz hlavného textu" and "Odkaz prekladu"
    const editorSection = firstCard.locator(".operator__slide-editor--bible");
    const mainRefLabel = editorSection
      .locator("label span")
      .filter({ hasText: "Odkaz hlavného textu" });
    await expect(mainRefLabel).toBeVisible();

    const transRefLabel = editorSection
      .locator("label span")
      .filter({ hasText: "Odkaz prekladu" });
    await expect(transRefLabel).toBeVisible();

    // There should be NO "Pódium" or "Skupina" fields
    const stageLabel = editorSection
      .locator("label span")
      .filter({ hasText: /^Pódium$/ });
    expect(await stageLabel.count()).toBe(0);

    const groupLabel = editorSection
      .locator("label span")
      .filter({ hasText: /^Skupina$/ });
    expect(await groupLabel.count()).toBe(0);

    // Cleanup
    await page.evaluate(
      async ({ url, id }: { url: string; id: string }) => {
        await fetch(`${url}/bible/presentations/${id}`, { method: "DELETE" });
      },
      { url: baseURL, id: presId },
    );
  });

  test("prepared slide editor matches live slide editor fields", async ({
    page,
  }) => {
    await navigateToBible(page);


    // Load a passage on Live tab
    await selectPassage(page, "jan 1");

    await page.waitForFunction(
      () => document.querySelectorAll('[data-role="slide-card"]').length > 0,
      { timeout: 15_000 },
    );

    // Switch to edit mode on Live tab and collect field labels
    await page.locator('[data-role="mode-toggle"][data-mode="edit"]').click();
    await page.waitForSelector('[data-role="slide-main-edit"]', {
      timeout: 5_000,
    });

    const liveCard = page.locator('[data-role="slide-card"]').first();
    const liveEditor = liveCard.locator(".operator__slide-editor--bible");
    const liveLabels = await liveEditor.locator("label span").allTextContents();
    const liveFieldSet = liveLabels.map((l) => l.trim()).sort();

    // Create a presentation with slides via API
    const presResponse = await page.evaluate(async (url: string) => {
      const resp = await fetch(`${url}/bible/presentations`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ name: "Field Match Test" }),
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
              {
                bibleMain: "Match test",
                bibleTranslation: "Trans",
                bibleMainReference: "Ref",
                bibleTranslationReference: "",
              },
            ],
          }),
        });
      },
      { url: baseURL, id: presId },
    );

    // Switch to Prepared tab and select the presentation
    await page.locator('[data-role="mode-toggle"][data-mode="live"]').click();
    await page.locator('[data-role="bible-tab"][data-tab="prepared"]').click();
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
      () => document.querySelectorAll('[data-role="slide-card"]').length > 0,
      { timeout: 10_000 },
    );

    // Switch to edit mode on Prepared tab and collect field labels
    await page.locator('[data-role="mode-toggle"][data-mode="edit"]').click();
    await page.waitForSelector('[data-role="slide-main-edit"]', {
      timeout: 5_000,
    });

    const prepCard = page.locator('[data-role="slide-card"]').first();
    const prepEditor = prepCard.locator(".operator__slide-editor--bible");
    const prepLabels = await prepEditor.locator("label span").allTextContents();
    const prepFieldSet = prepLabels.map((l) => l.trim()).sort();

    // Both tabs should have identical field sets
    expect(prepFieldSet).toEqual(liveFieldSet);
    expect(prepFieldSet).toContain("Hlavný text");
    expect(prepFieldSet).toContain("Preklad");
    expect(prepFieldSet).toContain("Odkaz hlavného textu");
    expect(prepFieldSet).toContain("Odkaz prekladu");

    // Cleanup
    await page.evaluate(
      async ({ url, id }: { url: string; id: string }) => {
        await fetch(`${url}/bible/presentations/${id}`, { method: "DELETE" });
      },
      { url: baseURL, id: presId },
    );
  });

  test("prepared slide saves main-ref to stage field via blur", async ({
    page,
  }) => {
    await navigateToBible(page);

    // Create a presentation with a slide
    const presResponse = await page.evaluate(async (url: string) => {
      const resp = await fetch(`${url}/bible/presentations`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ name: "Save Ref Test" }),
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
              {
                bibleMain: "Test text",
                bibleTranslation: "",
                bibleMainReference: "Original Ref",
                bibleTranslationReference: "",
              },
            ],
          }),
        });
      },
      { url: baseURL, id: presId },
    );

    // Switch to Prepared tab and select presentation
    await page.locator('[data-role="bible-tab"][data-tab="prepared"]').click();
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
      () => document.querySelectorAll('[data-role="slide-card"]').length > 0,
      { timeout: 10_000 },
    );

    // Switch to edit mode
    await page.locator('[data-role="mode-toggle"][data-mode="edit"]').click();
    await page.waitForSelector('[data-role="slide-main-ref"]', {
      timeout: 5_000,
    });

    // Edit the Main Reference input and blur to trigger save
    const mainRefInput = page.locator('[data-role="slide-main-ref"]').first();
    await mainRefInput.fill("Updated Reference 123");
    await mainRefInput.evaluate((el: HTMLInputElement) => el.blur());

    // Wait for save to complete
    await page.waitForTimeout(1_000);

    // Reload and verify persisted
    await page.reload({ waitUntil: "networkidle" });
    await page.waitForSelector('[data-role="bible-tab-nav"]', {
      timeout: 15_000,
    });

    await page.locator('[data-role="bible-tab"][data-tab="prepared"]').click();

    await page.waitForFunction(
      (id: string) =>
        document.querySelector(`[data-presentation-id="${id}"]`) !== null,
      presId,
      { timeout: 10_000 },
    );

    await page.locator(`[data-presentation-id="${presId}"]`).click();

    await page.waitForFunction(
      () => document.querySelectorAll('[data-role="slide-card"]').length > 0,
      { timeout: 10_000 },
    );

    // Switch to edit mode and verify persisted value
    await page.locator('[data-role="mode-toggle"][data-mode="edit"]').click();
    await page.waitForSelector('[data-role="slide-main-ref"]', {
      timeout: 5_000,
    });

    const savedValue = await page
      .locator('[data-role="slide-main-ref"]')
      .first()
      .inputValue();
    expect(savedValue).toBe("Updated Reference 123");

    // Cleanup
    await page.evaluate(
      async ({ url, id }: { url: string; id: string }) => {
        await fetch(`${url}/bible/presentations/${id}`, { method: "DELETE" });
      },
      { url: baseURL, id: presId },
    );
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

  test("creating a presentation via API refreshes prepared tab list", async ({
    page,
  }) => {
    await navigateToBible(page);
    const bp = biblePanel(page);

    // Switch to Prepared tab
    await page.locator('[data-role="bible-tab"][data-tab="prepared"]').click();

    // Count existing presentations
    const initialCount = await bp
      .locator('[data-role="presentation-card"]')
      .count();

    // Create a presentation via API (simulates what AI does via create_presentation())
    const presResponse = await page.evaluate(async (url: string) => {
      const resp = await fetch(`${url}/bible/presentations`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ name: "Auto Refresh Test" }),
      });
      return resp.json();
    }, baseURL);
    const presId = presResponse.id;

    // Wait for the presentation list to update WITHOUT page reload
    // The BibleSlidesChanged event should trigger a refresh
    await page.waitForFunction(
      (expected: number) => {
        const cards = document.querySelectorAll(
          '[data-view-panel="bible"] [data-role="presentation-card"]',
        );
        return cards.length > expected;
      },
      initialCount,
      { timeout: 15_000 },
    );

    const newCount2 = await bp
      .locator('[data-role="presentation-card"]')
      .count();
    expect(newCount2).toBeGreaterThan(initialCount);

    // Verify the new presentation is visible in the list
    const newPres = page.locator(`[data-presentation-id="${presId}"]`);
    await expect(newPres).toBeVisible();

    // Cleanup
    await page.evaluate(
      async ({ url, id }: { url: string; id: string }) => {
        await fetch(`${url}/bible/presentations/${id}`, { method: "DELETE" });
      },
      { url: baseURL, id: presId },
    );
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

    // Create a presentation via API, then reload to pick it up
    await page.evaluate(async () => {
      await fetch("/bible/presentations", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ name: "To Delete" }),
      });
    });
    await page.reload({ waitUntil: "networkidle" });
    await page.waitForSelector('[data-role="bible-tab-nav"]', {
      timeout: 15_000,
    });
    await page.locator('[data-role="bible-tab"][data-tab="prepared"]').click();
    await page.waitForFunction(
      () =>
        document.querySelectorAll(
          '[data-view-panel="bible"] [data-role="presentation-card"]',
        ).length > 0,
      { timeout: 15_000 },
    );

    // Select the last created presentation
    const presCards = bp.locator('[data-role="presentation-card"]');
    const countBefore = await presCards.count();
    const lastPres = presCards.last();
    await lastPres.click();
    await expect(lastPres).toHaveClass(/is-active/, { timeout: 5_000 });

    // Open the presentation edit modal
    await lastPres.locator('[data-role="presentation-edit"]').click();
    await page.waitForFunction(
      () => {
        const m = document.querySelector('[data-role="presentation-modal"]');
        return m && getComputedStyle(m).display !== "none";
      },
      { timeout: 5_000 },
    );

    // #874: the dialog archives instead of deleting (no confirm needed: the
    // presentation can be restored). Click via JS to avoid overlay hit-test issues.
    await page.evaluate(() => {
      const btn = document.querySelector(
        '[data-role="modal-archive"]',
      ) as HTMLButtonElement;
      btn?.click();
    });

    // Wait for card count to decrease
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


    // Load a passage
    await selectPassage(page, "jan 1");

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
        return toast && toast.textContent?.includes("Pridané snímky");
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
        return toast && toast.textContent?.includes("Nastavenie uložené");
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

    const kjv = translationButton(page, "secondary", "eng-kjv");
    await expect(kjv).toBeVisible({ timeout: 10_000 });
    await kjv.click();
    await expect(kjv).toHaveAttribute("aria-pressed", "true");
    const none = translationButton(page, "secondary", "");
    await expect(none).toHaveAttribute("aria-pressed", "false");
    await none.click();
    await expect(none).toHaveAttribute("aria-pressed", "true");
    await expect(kjv).toHaveAttribute("aria-pressed", "false");
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
    await expect(emptyState).toHaveText("Načítaj verše a zobrazia sa tu snímky.");
  });

  test("prepared tab slides column shows empty state", async ({ page }) => {
    await navigateToBible(page);
    const bp = biblePanel(page);

    // Switch to Prepared tab
    await page.locator('[data-role="bible-tab"][data-tab="prepared"]').click();

    const slidesColumn = bp.locator('[data-role="slides"]');
    const emptyState = slidesColumn.locator(".operator__slides-empty");
    await expect(emptyState).toHaveText(
      "Vyber prezentáciu vľavo.",
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


    // Load a passage
    await selectPassage(page, "jan 1");

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
        return toast && toast.textContent?.includes("Zobrazené na stene");
      },
      { timeout: 5_000 },
    );

    // Verify stage content via active-slide API (poll for propagation)
    await expect(async () => {
      const activeSlide = await page.evaluate(async (url: string) => {
        const resp = await fetch(`${url}/bible/active-slide`);
        return resp.json();
      }, baseURL);
      expect(activeSlide).not.toBeNull();
      expect(activeSlide.mainText).toBeTruthy();
      expect(activeSlide.mainText.length).toBeGreaterThan(0);
      expect(activeSlide.mainReference).toBeTruthy();
    }).toPass({ timeout: 10_000, intervals: [500] });
  });

  // -----------------------------------------------------------------------
  // Header Bible preview
  // -----------------------------------------------------------------------

  // #700: on the bible tab the header shows the pre-#460 VERSE preview
  // (`[data-role="bible-preview"]`, text-only, reading the active broadcast),
  // NOT the live stage-mirror iframe (which is worship-only). A triggered verse
  // appears in the verse-preview text; clearing returns it to the empty state.
  // The worship-only quick toggles stay behind `worship-preview` (worship-gated).
  test("bible header verse preview shows a triggered Bible verse", async ({
    page,
  }) => {
    const consoleMessages: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        if (msg.text().includes("crbug.com/981419")) return;
        consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
      }
    });
    await navigateToBible(page);
    await clearBroadcast();


    // #700: the stage-mirror iframe is worship-only — absent on the bible tab.
    await expect(page.locator("iframe.operator__stage-iframe")).toHaveCount(0);
    // The pre-#460 verse preview is shown instead.
    const versePreview = page.locator('[data-role="bible-preview"]');
    await expect(versePreview).toBeVisible();
    // Worship-only quick toggles are hidden in Bible view.
    await expect(page.locator('[data-role="worship-preview"]')).toBeHidden();

    // Load a passage and trigger the first slide via the operator UI. The verse
    // preview reads the active broadcast (pushed over the operator WS on
    // trigger) — no stage-layout change is needed (unlike the removed iframe).
    await selectPassage(page, "jan 1");

    await page.waitForFunction(
      () => document.querySelectorAll('[data-role="slide-card"]').length > 0,
      { timeout: 15_000 },
    );

    const firstTrigger = page
      .locator('[data-role="slide-trigger-zone"]')
      .first();
    await firstTrigger.click();

    // Wait for success toast.
    await page.waitForFunction(
      () => {
        const toast = document.querySelector('[data-role="toast"]');
        return toast && toast.textContent?.includes("Zobrazené na stene");
      },
      { timeout: 5_000 },
    );

    // The triggered verse text + reference appear in the header verse preview.
    const previewText = page.locator('[data-role="bible-preview-text"]');
    await expect(previewText).toBeVisible({ timeout: 15_000 });
    const textContent = await previewText.textContent();
    expect(textContent).toBeTruthy();
    expect(textContent!.length).toBeGreaterThan(0);

    const previewRef = page.locator('[data-role="bible-preview-ref"]');
    await expect(previewRef).toBeVisible();
    expect(await previewRef.textContent()).toBeTruthy();

    // The preview marks itself active while a broadcast is live.
    await expect(versePreview).toHaveAttribute("data-active", "true");

    // Clearing the broadcast returns the preview to its empty state.
    await clearBroadcast();
    await expect(versePreview).toHaveAttribute("data-active", "false", {
      timeout: 15_000,
    });

    // Switch to worship view — the worship-only toggles show again and the live
    // stage-mirror iframe returns (worship-only).
    const worshipButton = page.locator(
      '[data-role="view-toggle"][data-view="worship"]',
    );
    await worshipButton.click();
    await page.waitForFunction(
      () => document.body.getAttribute("data-view") === "worship",
      { timeout: 5_000 },
    );
    await expect(page.locator('[data-role="worship-preview"]')).toBeVisible();
    await expect(page.locator("iframe.operator__stage-iframe")).toBeVisible();

    expect(consoleMessages).toEqual([]);
  });

  test("trigger button in edit mode sends current (edited) text to stage", async ({
    page,
  }) => {
    await navigateToBible(page);
    await clearBroadcast();


    // Load a passage
    await selectPassage(page, "jan 1");

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
        return toast && toast.textContent?.includes("Zobrazené na stene");
      },
      { timeout: 5_000 },
    );

    // Verify the active-slide has the EDITED text (poll for propagation)
    await expect(async () => {
      const activeSlide = await page.evaluate(async (url: string) => {
        const resp = await fetch(`${url}/bible/active-slide`);
        return resp.json();
      }, baseURL);
      expect(activeSlide).not.toBeNull();
      expect(activeSlide.mainText).toBe("EDITED TEXT FOR TRIGGER TEST");
    }).toPass({ timeout: 10_000, intervals: [500] });

    await clearBroadcast();
  });

  test("triggering from trigger zone does not toggle selection", async ({
    page,
  }) => {
    await navigateToBible(page);


    // Load a passage
    await selectPassage(page, "jan 1");

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

  // -----------------------------------------------------------------------
  // Loaded passages history
  // -----------------------------------------------------------------------

  // NOTE: "loading a passage adds it to history" and "clicking history item
  // sets reference inputs" tests were removed because the LoadedPassagesHistory
  // component (bible_controls.rs) is defined but never mounted in any view.
  // The history UI is dead code — these tests can never pass.
  // TODO: Re-add tests when the history component is integrated into the Bible page.

  // -----------------------------------------------------------------------
  // Delete slide from prepared presentation
  // -----------------------------------------------------------------------

  test("delete slide from prepared presentation", async ({ page }) => {
    await navigateToBible(page);


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
              {
                bibleMain: "Slide 1",
                bibleTranslation: "",
                bibleMainReference: "",
                bibleTranslationReference: "",
              },
              {
                bibleMain: "Slide 2",
                bibleTranslation: "",
                bibleMainReference: "",
                bibleTranslationReference: "",
              },
              {
                bibleMain: "Slide 3",
                bibleTranslation: "",
                bibleMainReference: "",
                bibleTranslationReference: "",
              },
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

    // Switch to edit mode so delete buttons are visible
    await page.locator('[data-role="mode-toggle"][data-mode="edit"]').click();

    // Handle confirm dialog
    page.once("dialog", (dialog) => dialog.accept());

    // Click delete on first slide
    const deleteBtn = page.locator('[data-role="slide-delete"]').first();
    await expect(deleteBtn).toBeVisible({ timeout: 5_000 });
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
              {
                bibleMain: "First",
                bibleTranslation: "",
                bibleMainReference: "",
                bibleTranslationReference: "",
              },
              {
                bibleMain: "Second",
                bibleTranslation: "",
                bibleMainReference: "",
                bibleTranslationReference: "",
              },
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

    // Verify slides have a draggable handle
    const firstDragHandle = page.locator('[data-role="slide-drag-handle"]').first();
    await expect(firstDragHandle).toBeVisible({ timeout: 5_000 });
    await expect(firstDragHandle).toHaveAttribute("draggable", "true");

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


    // 1. Load a passage
    await selectPassage(page, "jan 1");

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
        return toast && toast.textContent?.includes("Zobrazené na stene");
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
        return toast && toast.textContent?.includes("Pridané snímky");
      },
      { timeout: 10_000 },
    );

    // 5. Switch to Prepared tab and select the newly created presentation
    await page.locator('[data-role="bible-tab"][data-tab="prepared"]').click();

    // #874: a new presentation goes to the TOP of the active list, and the
    // archive below it carries `data-presentation-id` too — pick the new
    // card by its name (the newest of that name is the first).
    const newPres = biblePanel(page)
      .locator('[data-role="presentation-card"]', { hasText: "Full Workflow Pres" })
      .first();
    await expect(newPres).toBeVisible({ timeout: 10_000 });
    await newPres.click();

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
        return toast && toast.textContent?.includes("Zobrazené na stene");
      },
      { timeout: 5_000 },
    );

    // 9. Verify stage content via active-slide API (poll for propagation)
    await expect(async () => {
      const activeSlide = await page.evaluate(async (url: string) => {
        const resp = await fetch(`${url}/bible/active-slide`);
        return resp.json();
      }, baseURL);
      expect(activeSlide).not.toBeNull();
      expect(activeSlide.mainText).toBeTruthy();
    }).toPass({ timeout: 10_000, intervals: [500] });

    // 10. Clear broadcast via API (ClearBroadcastButton component is not mounted in current UI)
    await page.evaluate(async (url: string) => {
      await fetch(`${url}/bible/clear`, { method: "POST" });
    }, baseURL);

    // Cleanup — get the presentation ID from the prepared tab
    const lastPresId = await newPres.getAttribute("data-presentation-id");
    if (lastPresId) {
      await page.evaluate(
        async ({ url, id }: { url: string; id: string }) => {
          await fetch(`${url}/bible/presentations/${id}`, { method: "DELETE" });
        },
        { url: baseURL, id: lastPresId },
      );
    }
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
    await page.waitForSelector('body[data-wasm-ready="true"]', {
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
    await page.waitForSelector('body[data-wasm-ready="true"]', {
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

  // -----------------------------------------------------------------------
  // Bug fixes (#219)
  // -----------------------------------------------------------------------

  test("translation selection persists after reload (#219 bug 2)", async ({
    page,
  }) => {
    const consoleMessages: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        if (msg.text().includes("crbug.com/981419")) return;
        consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
      }
    });

    await navigateToBible(page);

    // The button that is not on now, then a reload.
    const main = page.locator('[data-role="console-translation"][data-kind="main"]');
    await expect(main.first()).toBeVisible({ timeout: 10_000 });
    const off = main.and(page.locator('[aria-pressed="false"]')).first();
    const chosen = await off.getAttribute("data-translation");
    expect(chosen).toBeTruthy();
    // The server default is written too; the reload must not cut it off.
    const saved = page.waitForResponse(
      (res) => new URL(res.url()).pathname === "/bible/preferences" && res.request().method() === "PUT",
    );
    await off.click();
    await saved;
    // This browser remembers it (#701) before the reload.
    await expect
      .poll(() => page.evaluate(() => localStorage.getItem("presenter:bibleMainTranslation")))
      .toBe(JSON.stringify(chosen));

    await navigateToBible(page);
    await expect(page.locator('[data-role="bible-console"]')).toHaveAttribute(
      "data-books-translation",
      chosen!,
    );
    await expect(translationButton(page, "main", chosen!)).toHaveAttribute(
      "aria-pressed",
      "true",
    );

    expect(consoleMessages).toEqual([]);
  });

  test("broom button clears bible broadcast (#219 bug 3)", async ({
    page,
  }) => {
    const consoleMessages: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        if (msg.text().includes("crbug.com/981419")) return;
        consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
      }
    });

    await navigateToBible(page);

    const panel = biblePanel(page);

    // Select a book and load verses to create a broadcast
    await selectPassage(page, "jan 1");

    // Trigger the first slide to broadcast it
    const slideCard = page
      .locator('[data-role="slide-card"]')
      .first();
    if ((await slideCard.count()) > 0) {
      await slideCard.click();
      await page.waitForTimeout(1000);
    }

    // Click broom button
    const broomButton = page.locator('[data-role="clear-slide"]');
    await expect(broomButton).toBeVisible({ timeout: 5_000 });
    await broomButton.click();

    // Wait for toast confirmation
    await page.waitForTimeout(500);

    // Verify bible broadcast is cleared via API
    const response = await page.evaluate(async (url) => {
      const resp = await fetch(`${url}/bible/active`);
      return resp.json();
    }, baseURL);

    // Endpoint returns null when no broadcast is active
    expect(response).toBeNull();

    expect(consoleMessages).toEqual([]);
  });

  test("bible verse text is not truncated with ellipsis (#219 bug 4)", async ({
    page,
  }) => {
    const consoleMessages: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        if (msg.text().includes("crbug.com/981419")) return;
        consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
      }
    });

    await navigateToBible(page);

    const panel = biblePanel(page);

    // Select a book and load verses
    await selectPassage(page, "jan 1");

    // Check that slide text uses pre-wrap, not ellipsis
    const slideText = page.locator(".operator__slide-card--bible .operator__slide-text").first();
    if ((await slideText.count()) > 0) {
      const styles = await slideText.evaluate((el) => {
        const cs = window.getComputedStyle(el);
        return {
          whiteSpace: cs.whiteSpace,
          textOverflow: cs.textOverflow,
        };
      });
      expect(styles.whiteSpace).toBe("pre-wrap");
      expect(styles.textOverflow).not.toBe("ellipsis");
    }

    expect(consoleMessages).toEqual([]);
  });

  test("stage preview is legible when inactive (#219 bug 5)", async ({
    page,
  }) => {
    const consoleMessages: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        if (msg.text().includes("crbug.com/981419")) return;
        consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
      }
    });

    await navigateToBible(page);

    const preview = page.locator(".operator__stage-preview");
    if ((await preview.count()) > 0) {
      const opacity = await preview.evaluate((el) => {
        return parseFloat(window.getComputedStyle(el).opacity);
      });
      // Should be full opacity (1.0) — no dimming
      expect(opacity).toBe(1);
    }

    expect(consoleMessages).toEqual([]);
  });

  test("bible edit mode textareas are not too small (#219 bug 6)", async ({
    page,
  }) => {
    const consoleMessages: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        if (msg.text().includes("crbug.com/981419")) return;
        consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
      }
    });

    await navigateToBible(page);

    const panel = biblePanel(page);

    // Load some verses
    await selectPassage(page, "jan 1");

    // Switch to edit mode
    const editButton = page.locator(
      '[data-role="mode-toggle"][data-mode="edit"]',
    );
    await editButton.click();
    await page.waitForTimeout(500);

    // Textareas use field-sizing: content — they auto-size to their text.
    // Main textarea with verses should be taller than an empty one.
    const mainTextarea = page
      .locator(".operator--bible .operator__slide-editor textarea[data-field='main']")
      .first();
    const transTextarea = page
      .locator(".operator--bible .operator__slide-editor textarea[data-field='translation']")
      .first();
    if ((await mainTextarea.count()) > 0) {
      const mainHeight = await mainTextarea.evaluate((el) => el.getBoundingClientRect().height);
      // Main textarea with verse text should be at least 30px (visible)
      expect(mainHeight).toBeGreaterThan(30);
      // If translation is empty, it should be smaller than main
      if ((await transTextarea.count()) > 0) {
        const transHeight = await transTextarea.evaluate((el) => el.getBoundingClientRect().height);
        const transValue = await transTextarea.inputValue();
        if (!transValue) {
          expect(transHeight).toBeLessThan(mainHeight);
        }
      }
    }

    // Switch back to live mode
    const liveButton = page.locator(
      '[data-role="mode-toggle"][data-mode="live"]',
    );
    await liveButton.click();

    expect(consoleMessages).toEqual([]);
  });
});

test.describe("Bible workflow fixes (issue #256)", () => {
  test("#875 a toast goes away by itself, and a new one gets its full time", async ({
    page,
  }) => {
    const consoleMessages: string[] = [];
    attachConsoleErrorCollector(page, consoleMessages);
    const names: string[] = [];
    page.on("dialog", (dialog) => {
      const name = `Toast ${Date.now()}`;
      names.push(name);
      void dialog.accept(name);
    });
    await navigateToBible(page);
    await page.locator('[data-role="bible-tab"][data-tab="prepared"]').click();
    const create = biblePanel(page).locator('[data-role="presentation-create"]');
    const toast = page.locator('[data-role="toast"]');

    await create.click();
    await expect(toast).toHaveAttribute("data-visible", "true");
    await expect(toast).toHaveAttribute("data-visible", "false");

    // The second toast outlives the moment the first one would have ended.
    await create.click();
    await expect(toast).toHaveAttribute("data-visible", "true");
    await page.waitForTimeout(2_500);
    await create.click();
    await expect(toast).toContainText(names[2]);
    await page.waitForTimeout(2_000);
    await expect(toast).toHaveAttribute("data-visible", "true");
    await expect(toast).toHaveAttribute("data-visible", "false");

    const stored: { id: string; name: string }[] = await (
      await page.request.get(`${baseURL}/bible/presentations`)
    ).json();
    for (const presentation of stored.filter((p) => names.includes(p.name))) {
      await page.request.delete(`${baseURL}/bible/presentations/${presentation.id}`);
    }
    expect(consoleMessages).toEqual([]);
  });
});
