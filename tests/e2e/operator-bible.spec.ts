import { expect, test } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

let serverHandle: ServerHandle | undefined;
let baseURL = "";

test.describe.configure({ timeout: 180_000 });

test.beforeAll(async ({}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  baseURL = config.baseURL;
  await refreshDevData(config.dbUrl);
  serverHandle = await startTestServer(
    config.port,
    config.dbUrl,
    config.oscPort,
  );
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
});

test("operator bible surface drives live passage broadcast", async ({
  page,
  request,
}) => {
  await expect(async () => {
    const response = await request.get(`${baseURL}/healthz`, {
      timeout: 60_000,
    });
    expect(response.ok()).toBeTruthy();
  }).toPass({ timeout: 90_000 });

  await page.goto(`${baseURL}/ui/bible`);
  await expect(page).toHaveURL(/\/ui\/bible(\?.*)?$/);

  // Sub-tab nav should be visible with LIVE tab active by default
  const liveTab = page.locator('[data-role="bible-tab"][data-tab="live"]');
  await expect(liveTab).toBeVisible({ timeout: 30_000 });
  await expect(liveTab).toHaveAttribute("data-active", "true");

  // Settings tab contains translation dropdowns — switch there to verify
  const settingsTab = page.locator(
    '[data-role="bible-tab"][data-tab="settings"]',
  );
  await settingsTab.click();
  await expect(settingsTab).toHaveAttribute("data-active", "true");

  const mainTranslationDropdown = page.locator(
    '[data-role="main-translation"]',
  );
  await expect(mainTranslationDropdown).toBeVisible({ timeout: 30_000 });
  const secondaryTranslationDropdown = page.locator(
    '[data-role="secondary-translation"]',
  );
  await expect(secondaryTranslationDropdown).toBeVisible();

  const waitForToastVisible = async () => {
    await page.waitForFunction(
      () => {
        const toast = document.querySelector('[data-role="toast"]');
        return toast && toast.getAttribute("data-visible") === "true";
      },
      { timeout: 60_000 },
    );
  };
  const waitForToastHidden = async () => {
    await page.waitForFunction(
      () => {
        const toast = document.querySelector('[data-role="toast"]');
        return !toast || toast.getAttribute("data-visible") !== "true";
      },
      { timeout: 60_000 },
    );
  };

  // Verify translations API is accessible
  const translationsResponse = await request.get(
    `${baseURL}/bible/translations`,
  );
  expect(translationsResponse.ok()).toBeTruthy();
  const translations: Array<{ code: string; name: string; language?: string }> =
    await translationsResponse.json();
  expect(translations.length).toBeGreaterThan(0);

  // Verify main dropdown has correct options
  const mainOptions = await mainTranslationDropdown
    .locator("option")
    .allTextContents();
  expect(mainOptions.length).toBe(translations.length);

  // Verify secondary dropdown has "None" + all translations
  const secondaryOptions = await secondaryTranslationDropdown
    .locator("option")
    .allTextContents();
  expect(secondaryOptions.length).toBe(translations.length + 1);
  expect(secondaryOptions[0]).toBe("None");

  // Switch main translation via dropdown
  const stateSnapshot = await page.evaluate(
    () => (window as any).__presenterBibleState,
  );
  const activeCode =
    stateSnapshot?.preferences?.mainTranslation ?? translations[0]?.code ?? "";
  const targetTranslation =
    translations.find((t) => t.code !== activeCode) ?? translations[0];

  if (targetTranslation.code !== activeCode) {
    await mainTranslationDropdown.selectOption(targetTranslation.code);
    await expect(async () => {
      const mainTranslation = await page.evaluate(
        () =>
          (window as any).__presenterBibleState?.preferences?.mainTranslation,
      );
      expect(mainTranslation).toBe(targetTranslation.code);
    }).toPass({ timeout: 10_000 });
    await expect(mainTranslationDropdown).toHaveValue(targetTranslation.code);
  }

  // Switch to Slovak translation via dropdown for passage loading
  const slovakOption = translations.find((t) => t.code === "slk-seb");
  if (slovakOption) {
    await mainTranslationDropdown.selectOption("slk-seb");
    await expect(async () => {
      const mainTranslation = await page.evaluate(
        () =>
          (window as any).__presenterBibleState?.preferences?.mainTranslation,
      );
      expect(mainTranslation).toBe("slk-seb");
    }).toPass({ timeout: 10_000 });
    await expect(mainTranslationDropdown).toHaveValue("slk-seb");
  }

  // Switch back to LIVE tab for passage loading
  await liveTab.click();
  await expect(liveTab).toHaveAttribute("data-active", "true");

  // Search for a book and select it
  await page.locator('[data-role="book-filter"]').fill("Jan");
  const johnButton = page
    .locator('[data-role="book-list"] button[data-book-code="JHN"]')
    .first();
  await expect(johnButton).toBeVisible({ timeout: 30_000 });
  await johnButton.click();

  // Load a passage
  await page.locator('[data-role="chapter-input"]').fill("3");
  await page.locator('[data-role="verse-start"]').fill("16");
  await page.locator('[data-role="verse-end"]').fill("18");
  await page.locator('[data-role="load-button"]').click();
  await waitForToastVisible();
  await waitForToastHidden();

  // Verify slides were generated
  const slideCards = page.locator(".operator__slide-card");
  await expect(slideCards.first()).toBeVisible({ timeout: 60_000 });
  const slideCount = await slideCards.count();
  expect(slideCount).toBeGreaterThan(0);

  // Verify slide metadata
  const slideMetadata = await page.evaluate(() => {
    const slides = (window as any).__presenterBibleState?.slides ?? [];
    const first = slides[0];
    return first?.metadata?.bible ?? null;
  });
  expect(slideMetadata).toBeTruthy();
  expect(slideMetadata.book || slideMetadata.book_name).toBeTruthy();
  expect(slideMetadata.bookCode ?? slideMetadata.book_code).toBe("JHN");
  expect(slideMetadata.bookNumber ?? slideMetadata.book_number).toBe(43);

  const firstSlideId = await page.evaluate(() => {
    const slides = (window as any).__presenterBibleState?.slides ?? [];
    return slides[0]?.id ?? null;
  });
  expect(firstSlideId).toBeTruthy();

  // Click the select zone on first slide → toggles selection (blue outline)
  await slideCards.first().locator('[data-role="slide-select-zone"]').click();
  await expect(slideCards.first()).toHaveClass(/is-selected/);

  // Click the trigger zone on first slide → broadcasts the slide
  await slideCards.first().locator('[data-role="slide-trigger"]').click();

  await waitForToastVisible();
  const toastText = await page.locator('[data-role="toast"]').innerText();
  expect(toastText).toContain("Slide triggered");
  await waitForToastHidden();

  // Verify the active broadcast state
  await page.waitForFunction(
    () => {
      const active = (window as any).__presenterBibleState?.activeBroadcast;
      if (!active) return false;
      const ref = active.passage?.reference || {};
      const code = ref.book_code ?? ref.bookCode;
      const start = ref.verse_start ?? ref.verseStart;
      return code === "JHN" && start === 16;
    },
    { timeout: 60_000 },
  );

  // Verify via API
  const activeResponse = await request.get(`${baseURL}/bible/active`);
  expect(activeResponse.ok()).toBeTruthy();
  const activeJson = await activeResponse.json();
  expect(
    activeJson?.passage?.reference?.book_code ??
      activeJson?.passage?.reference?.bookCode,
  ).toBe("JHN");
  expect(
    activeJson?.passage?.reference?.verse_start ??
      activeJson?.passage?.reference?.verseStart,
  ).toBe(16);

  // Verify PREPARED tab: switch to it and verify presentations list
  const preparedTab = page.locator(
    '[data-role="bible-tab"][data-tab="prepared"]',
  );
  await preparedTab.click();
  await expect(preparedTab).toHaveAttribute("data-active", "true");
  await expect(page.locator('[data-role="presentations-list"]')).toBeVisible();
});

test("operator header shows Bible preview when bible view is active", async ({
  page,
  request,
}) => {
  await expect(async () => {
    const response = await request.get(`${baseURL}/healthz`, {
      timeout: 60_000,
    });
    expect(response.ok()).toBeTruthy();
  }).toPass({ timeout: 90_000 });

  // Navigate to operator in Bible view FIRST (before triggering the broadcast)
  await page.goto(`${baseURL}/ui/operator/bible`);
  await expect(page).toHaveURL(/\/ui\/operator\/bible/);

  // Wait for WebSocket connection
  await page.waitForFunction(
    () => (window as any).__presenterLiveConnected === true,
    { timeout: 30_000 },
  );

  // Now trigger a broadcast. The operator page will receive it via WebSocket.
  // Get the active broadcast from test 1 (still in server state)
  const activeCheck = await request.get(`${baseURL}/bible/active`);
  const activeBroadcast = activeCheck.ok() ? await activeCheck.json() : null;

  let broadcastTriggered = false;
  if (activeBroadcast?.passage) {
    // Re-trigger by extracting ref info from the existing broadcast
    const ref = activeBroadcast.passage.reference;
    const trans = activeBroadcast.passage.translation;
    const triggerResponse = await request.post(`${baseURL}/bible/trigger`, {
      data: {
        translation: trans?.code || "slk-seb",
        book: ref?.book || ref?.book_name || "John",
        book_code: ref?.book_code || ref?.bookCode || "JHN",
        book_number: ref?.book_number || ref?.bookNumber || 43,
        chapter: ref?.chapter || 3,
        verse_start: ref?.verse_start || ref?.verseStart || 16,
        verse_end: ref?.verse_end || ref?.verseEnd || 16,
      },
    });
    broadcastTriggered = triggerResponse.ok();
  }
  if (!broadcastTriggered) {
    // Fallback: simulate a Bible broadcast via JS to test the rendering
    await page.evaluate(() => {
      const state = (window as any).__presenterOperatorState;
      if (state) {
        state.activeBibleBroadcast = {
          passage: {
            reference: {
              book: "John",
              chapter: 3,
              verse_start: 16,
              verseStart: 16,
              verse_end: 16,
              verseEnd: 16,
            },
            translation: { code: "TST", name: "Test Translation" },
            text: "For God so loved the world, that he gave his only begotten Son.",
          },
          triggeredAt: new Date().toISOString(),
        };
      }
      // Trigger renderStageStatus manually
      const fn = (window as any).__renderStageStatus;
      if (typeof fn === "function") fn();
    });
  }

  // The Bible preview panel should be visible
  const biblePreview = page.locator('[data-role="bible-preview"]');
  await expect(async () => {
    const display = await biblePreview.evaluate(
      (el) => window.getComputedStyle(el).display,
    );
    expect(display).not.toBe("none");
  }).toPass({ timeout: 15_000 });

  // Worship preview should be hidden
  const worshipPreview = page.locator('[data-role="worship-preview"]');
  await expect(async () => {
    const display = await worshipPreview.evaluate(
      (el) => window.getComputedStyle(el).display,
    );
    expect(display).toBe("none");
  }).toPass({ timeout: 5_000 });

  // The bible preview should contain verse text (not "No active passage")
  await expect(async () => {
    const text = await biblePreview.innerText();
    expect(text).not.toContain("No active passage");
    expect(text.length).toBeGreaterThan(5);
  }).toPass({ timeout: 15_000 });

  // Verify the reference info is present in the preview
  const refEl = biblePreview.locator(".operator__bible-preview-ref");
  await expect(refEl).toBeVisible();

  // Stage status container should indicate active state
  const stageStatus = page.locator('[data-role="stage-status"]');
  await expect(stageStatus).toHaveAttribute("data-active", "true");

  // Switch to worship view — worship preview should appear, bible preview should hide
  await page.locator('[data-role="view-toggle"][data-view="worship"]').click();
  await expect(async () => {
    const worshipDisplay = await worshipPreview.evaluate(
      (el) => window.getComputedStyle(el).display,
    );
    expect(worshipDisplay).not.toBe("none");
  }).toPass({ timeout: 5_000 });
  await expect(async () => {
    const bibleDisplay = await biblePreview.evaluate(
      (el) => window.getComputedStyle(el).display,
    );
    expect(bibleDisplay).toBe("none");
  }).toPass({ timeout: 5_000 });
});

test("bible tab edit mode works in live and prepared tabs", async ({
  page,
  request,
}) => {
  await expect(async () => {
    const response = await request.get(`${baseURL}/healthz`, {
      timeout: 60_000,
    });
    expect(response.ok()).toBeTruthy();
  }).toPass({ timeout: 90_000 });

  await page.goto(`${baseURL}/ui/bible`);
  const liveTab = page.locator('[data-role="bible-tab"][data-tab="live"]');
  await expect(liveTab).toBeVisible({ timeout: 30_000 });

  const waitForToastVisible = async () => {
    await page.waitForFunction(
      () => {
        const toast = document.querySelector('[data-role="toast"]');
        return toast && toast.getAttribute("data-visible") === "true";
      },
      { timeout: 60_000 },
    );
  };
  const waitForToastHidden = async () => {
    await page.waitForFunction(
      () => {
        const toast = document.querySelector('[data-role="toast"]');
        return !toast || toast.getAttribute("data-visible") !== "true";
      },
      { timeout: 60_000 },
    );
  };

  // Go to settings tab first to ensure translation is set and books load
  const settingsTab = page.locator(
    '[data-role="bible-tab"][data-tab="settings"]',
  );
  await settingsTab.click();
  await expect(settingsTab).toHaveAttribute("data-active", "true");
  const mainDropdown = page.locator('[data-role="main-translation"]');
  await expect(mainDropdown).toBeVisible({ timeout: 30_000 });

  // Select the Slovak translation to ensure books load
  const translations: Array<{ code: string }> = await (
    await request.get(`${baseURL}/bible/translations`)
  ).json();
  const hasSlovak = translations.some((t) => t.code === "slk-seb");
  if (hasSlovak) {
    await mainDropdown.selectOption("slk-seb");
    await expect(async () => {
      const mainTranslation = await page.evaluate(
        () =>
          (window as any).__presenterBibleState?.preferences?.mainTranslation,
      );
      expect(mainTranslation).toBe("slk-seb");
    }).toPass({ timeout: 10_000 });
  }

  // Switch back to live tab for passage loading
  await liveTab.click();
  await expect(liveTab).toHaveAttribute("data-active", "true");

  // Wait for books to load
  await expect(
    page.locator('[data-role="book-list"] button').first(),
  ).toBeVisible({ timeout: 60_000 });

  // Load a passage
  await page.locator('[data-role="book-filter"]').fill("Jan");
  const johnButton = page
    .locator('[data-role="book-list"] button[data-book-code="JHN"]')
    .first();
  await expect(johnButton).toBeVisible({ timeout: 30_000 });
  await johnButton.click();

  await page.locator('[data-role="chapter-input"]').fill("1");
  await page.locator('[data-role="verse-start"]').fill("1");
  await page.locator('[data-role="verse-end"]').fill("3");
  await page.locator('[data-role="load-button"]').click();
  await waitForToastVisible();
  await waitForToastHidden();

  const slideCards = page.locator(".operator__slide-card");
  await expect(slideCards.first()).toBeVisible({ timeout: 60_000 });

  // Edit mode toggle in live tab (use button selector to avoid matching modal div)
  const editBtn = page.locator('button[data-mode="edit"]');
  const liveBtn = page.locator('button[data-mode="live"]');
  await editBtn.click();

  // Verify textareas appear (edit mode renders textarea elements)
  await expect(page.locator('[data-role="slide-main"]').first()).toBeVisible({
    timeout: 10_000,
  });

  // Edit the main text inline
  const mainTextarea = page.locator('[data-role="slide-main"]').first();
  const originalText = await mainTextarea.inputValue();
  await mainTextarea.fill("Edited text in live tab");

  // Verify the in-memory slide was updated
  await expect(async () => {
    const firstMain = await page.evaluate(
      () => (window as any).__presenterBibleState?.slides?.[0]?.main,
    );
    expect(firstMain).toBe("Edited text in live tab");
  }).toPass({ timeout: 5_000 });

  // Switch back to live mode — textareas should disappear
  await liveBtn.click();
  await expect(page.locator('[data-role="slide-main"]')).toHaveCount(0, {
    timeout: 10_000,
  });

  // Now test prepared tab edit mode
  // First, select some slides and create a presentation
  await editBtn.click();
  // Restore original text so slide has valid content
  await page.locator('[data-role="slide-main"]').first().fill(originalText);
  await liveBtn.click();

  // Select all slides
  const selectZones = page.locator('[data-role="slide-select-zone"]');
  const selectCount = await selectZones.count();
  for (let i = 0; i < selectCount; i++) {
    await selectZones.nth(i).click();
  }

  // Switch to prepared tab and create a presentation
  const preparedTab = page.locator(
    '[data-role="bible-tab"][data-tab="prepared"]',
  );
  await preparedTab.click();
  await expect(preparedTab).toHaveAttribute("data-active", "true");

  const presentationName = `Edit Test ${Date.now()}`;
  page.once("dialog", async (dialog) => {
    await dialog.accept(presentationName);
  });
  await page.locator('[data-role="presentation-create"]').click();
  await waitForToastVisible();
  await waitForToastHidden();

  // Switch back to live and add slides to the presentation
  await liveTab.click();
  await expect(async () => {
    const listText = await page
      .locator('[data-role="presentations-list"]')
      .innerText();
    expect(listText).toContain(presentationName);
  }).toPass({ timeout: 10_000 });

  // Get the created presentation id
  const presentationsResponse = await request.get(
    `${baseURL}/bible/presentations`,
  );
  const presentations = await presentationsResponse.json();
  const created = presentations.find(
    (entry: any) => entry.name === presentationName,
  );
  expect(created).toBeTruthy();

  // Select the presentation in dropdown and add slides
  await page.selectOption('[data-role="presentation-select"]', created.id);
  await page.locator('[data-role="presentation-add"]').click();
  await waitForToastVisible();
  await waitForToastHidden();

  // Switch to prepared tab and click on the presentation
  await preparedTab.click();
  const presentationCard = page.locator(
    `article[data-presentation-id="${created.id}"]`,
  );
  await presentationCard.click();
  await expect(async () => {
    const activeId = await page.evaluate(
      () => (window as any).__presenterBibleState?.activePresentationId,
    );
    expect(activeId).toBe(created.id);
  }).toPass({ timeout: 10_000 });

  // Verify slides loaded in prepared tab (triggerOnly by default)
  const prepSlides = page.locator(".operator__slide-card");
  await expect(prepSlides.first()).toBeVisible({ timeout: 10_000 });

  // In live mode, slides should be triggerOnly (no textarea)
  await expect(page.locator('[data-role="slide-main"]')).toHaveCount(0, {
    timeout: 5_000,
  });

  // Switch to edit mode — textareas should appear in prepared tab too
  await editBtn.click();
  await expect(page.locator('[data-role="slide-main"]').first()).toBeVisible({
    timeout: 10_000,
  });

  // Edit a slide's main text to test auto-save
  const prepMainTextarea = page.locator('[data-role="slide-main"]').first();
  const editedValue = `Edited prepared ${Date.now()}`;
  await prepMainTextarea.fill(editedValue);

  // Wait for debounced save (toast "Slide saved")
  await expect(async () => {
    const toastText = await page.locator('[data-role="toast"]').innerText();
    expect(toastText).toContain("Slide saved");
  }).toPass({ timeout: 10_000 });

  // Verify the edit persisted by reloading the presentation detail via API
  await expect(async () => {
    const detailResponse = await request.get(
      `${baseURL}/bible/presentations/${created.id}`,
    );
    expect(detailResponse.ok()).toBeTruthy();
    const detail = await detailResponse.json();
    const firstSlideMain =
      detail.slides?.[0]?.main ?? detail.slides?.[0]?.main_text ?? "";
    expect(firstSlideMain).toBe(editedValue);
  }).toPass({ timeout: 10_000 });
});

test("bible tab select all button selects and deselects all slides", async ({
  page,
  request,
}) => {
  await expect(async () => {
    const response = await request.get(`${baseURL}/healthz`, {
      timeout: 60_000,
    });
    expect(response.ok()).toBeTruthy();
  }).toPass({ timeout: 90_000 });

  await page.goto(`${baseURL}/ui/bible`);
  const liveTab = page.locator('[data-role="bible-tab"][data-tab="live"]');
  await expect(liveTab).toBeVisible({ timeout: 30_000 });

  const waitForToastVisible = async () => {
    await page.waitForFunction(
      () => {
        const toast = document.querySelector('[data-role="toast"]');
        return toast && toast.getAttribute("data-visible") === "true";
      },
      { timeout: 60_000 },
    );
  };
  const waitForToastHidden = async () => {
    await page.waitForFunction(
      () => {
        const toast = document.querySelector('[data-role="toast"]');
        return !toast || toast.getAttribute("data-visible") !== "true";
      },
      { timeout: 60_000 },
    );
  };

  // Ensure translation is set
  const settingsTab = page.locator(
    '[data-role="bible-tab"][data-tab="settings"]',
  );
  await settingsTab.click();
  const mainDropdown = page.locator('[data-role="main-translation"]');
  await expect(mainDropdown).toBeVisible({ timeout: 30_000 });

  const translations: Array<{ code: string }> = await (
    await request.get(`${baseURL}/bible/translations`)
  ).json();
  const hasSlovak = translations.some((t) => t.code === "slk-seb");
  if (hasSlovak) {
    await mainDropdown.selectOption("slk-seb");
    await expect(async () => {
      const mainTranslation = await page.evaluate(
        () =>
          (window as any).__presenterBibleState?.preferences?.mainTranslation,
      );
      expect(mainTranslation).toBe("slk-seb");
    }).toPass({ timeout: 10_000 });
  }

  // Switch to live tab and load a passage
  await liveTab.click();
  await expect(liveTab).toHaveAttribute("data-active", "true");

  await expect(
    page.locator('[data-role="book-list"] button').first(),
  ).toBeVisible({ timeout: 60_000 });

  await page.locator('[data-role="book-filter"]').fill("Jan");
  const johnButton = page
    .locator('[data-role="book-list"] button[data-book-code="JHN"]')
    .first();
  await expect(johnButton).toBeVisible({ timeout: 30_000 });
  await johnButton.click();

  await page.locator('[data-role="chapter-input"]').fill("1");
  await page.locator('[data-role="verse-start"]').fill("1");
  await page.locator('[data-role="verse-end"]').fill("5");
  await page.locator('[data-role="load-button"]').click();
  await waitForToastVisible();
  await waitForToastHidden();

  const slideCards = page.locator(".operator__slide-card");
  await expect(slideCards.first()).toBeVisible({ timeout: 60_000 });
  const slideCount = await slideCards.count();
  expect(slideCount).toBeGreaterThan(0);

  // Verify "Select all" button is visible
  const selectAllBtn = page.locator('[data-role="select-all-slides"]');
  await expect(selectAllBtn).toBeVisible();

  // Click "Select all" — all slides should get is-selected class
  await selectAllBtn.click();

  await expect(async () => {
    const selectedCount = await page.evaluate(
      () => (window as any).__presenterBibleState?.selectedSlides?.size ?? 0,
    );
    const totalSlides = await page.evaluate(
      () => (window as any).__presenterBibleState?.slides?.length ?? 0,
    );
    expect(selectedCount).toBe(totalSlides);
    expect(selectedCount).toBeGreaterThan(0);
  }).toPass({ timeout: 5_000 });

  // All slide cards should have is-selected class
  for (let i = 0; i < slideCount; i++) {
    await expect(slideCards.nth(i)).toHaveClass(/is-selected/);
  }

  // Selection count label should show correct number
  const selectionLabel = page.locator('[data-role="selection-count"]');
  await expect(selectionLabel).toContainText(`${slideCount} selected`);

  // Click "Select all" again — should deselect all
  await selectAllBtn.click();

  await expect(async () => {
    const selectedCount = await page.evaluate(
      () => (window as any).__presenterBibleState?.selectedSlides?.size ?? 0,
    );
    expect(selectedCount).toBe(0);
  }).toPass({ timeout: 5_000 });

  // No slide cards should have is-selected class
  for (let i = 0; i < slideCount; i++) {
    await expect(slideCards.nth(i)).not.toHaveClass(/is-selected/);
  }

  await expect(selectionLabel).toContainText("0 selected");
});

test("operator edit/live mode toggle propagates to bible iframe", async ({
  page,
  request,
}) => {
  await expect(async () => {
    const response = await request.get(`${baseURL}/healthz`, {
      timeout: 60_000,
    });
    expect(response.ok()).toBeTruthy();
  }).toPass({ timeout: 90_000 });

  // Navigate to operator page in Bible view
  await page.goto(`${baseURL}/ui/operator/bible`);
  await expect(page).toHaveURL(/\/ui\/operator\/bible/);

  // Wait for the Bible iframe to load
  const bibleIframe = page.locator('[data-view-panel="bible"] iframe');
  await expect(bibleIframe).toBeVisible({ timeout: 30_000 });
  const bibleFrame = page.frameLocator('[data-view-panel="bible"] iframe');

  // Wait for Bible script to initialise inside the iframe
  await expect(async () => {
    const ready = await bibleFrame
      .locator("body")
      .evaluate(() => !!(window as any).__presenterBibleState);
    expect(ready).toBeTruthy();
  }).toPass({ timeout: 30_000 });

  // Set translation inside the iframe
  const settingsTab = bibleFrame.locator(
    '[data-role="bible-tab"][data-tab="settings"]',
  );
  await settingsTab.click();
  const mainDropdown = bibleFrame.locator('[data-role="main-translation"]');
  await expect(mainDropdown).toBeVisible({ timeout: 30_000 });

  const translations: Array<{ code: string }> = await (
    await request.get(`${baseURL}/bible/translations`)
  ).json();
  const hasSlovak = translations.some((t) => t.code === "slk-seb");
  if (hasSlovak) {
    await mainDropdown.selectOption("slk-seb");
    await expect(async () => {
      const mainTranslation = await bibleFrame
        .locator("body")
        .evaluate(
          () =>
            (window as any).__presenterBibleState?.preferences?.mainTranslation,
        );
      expect(mainTranslation).toBe("slk-seb");
    }).toPass({ timeout: 10_000 });
  }

  // Switch to live tab inside the iframe and load a passage
  const liveTab = bibleFrame.locator(
    '[data-role="bible-tab"][data-tab="live"]',
  );
  await liveTab.click();
  await expect(liveTab).toHaveAttribute("data-active", "true");

  // Wait for books to load
  await expect(
    bibleFrame.locator('[data-role="book-list"] button').first(),
  ).toBeVisible({ timeout: 60_000 });

  await bibleFrame.locator('[data-role="book-filter"]').fill("Jan");
  const johnButton = bibleFrame
    .locator('[data-role="book-list"] button[data-book-code="JHN"]')
    .first();
  await expect(johnButton).toBeVisible({ timeout: 30_000 });
  await johnButton.click();

  await bibleFrame.locator('[data-role="chapter-input"]').fill("3");
  await bibleFrame.locator('[data-role="verse-start"]').fill("16");
  await bibleFrame.locator('[data-role="verse-end"]').fill("17");
  await bibleFrame.locator('[data-role="load-button"]').click();

  // Wait for slides to appear
  const slideCards = bibleFrame.locator(".operator__slide-card");
  await expect(slideCards.first()).toBeVisible({ timeout: 60_000 });

  // Verify we start in live mode — no textareas in the iframe
  await expect(bibleFrame.locator('[data-role="slide-main"]')).toHaveCount(0, {
    timeout: 5_000,
  });

  // Click the PARENT page's Edit button
  const parentEditBtn = page.locator(
    'button[data-role="mode-toggle"][data-mode="edit"]',
  );
  await parentEditBtn.click();

  // Verify textareas APPEAR inside the Bible iframe (edit mode propagated)
  await expect(
    bibleFrame.locator('[data-role="slide-main"]').first(),
  ).toBeVisible({ timeout: 10_000 });

  // Verify the iframe state was updated
  await expect(async () => {
    const editMode = await bibleFrame
      .locator("body")
      .evaluate(() => (window as any).__presenterBibleState?.editMode);
    expect(editMode).toBe(true);
  }).toPass({ timeout: 5_000 });

  // Click the PARENT page's Live button
  const parentLiveBtn = page.locator(
    'button[data-role="mode-toggle"][data-mode="live"]',
  );
  await parentLiveBtn.click();

  // Verify textareas DISAPPEAR inside the Bible iframe (live mode propagated)
  await expect(bibleFrame.locator('[data-role="slide-main"]')).toHaveCount(0, {
    timeout: 10_000,
  });

  // Verify the iframe state was updated back to live
  await expect(async () => {
    const editMode = await bibleFrame
      .locator("body")
      .evaluate(() => (window as any).__presenterBibleState?.editMode);
    expect(editMode).toBe(false);
  }).toPass({ timeout: 5_000 });
});

test("operator header search switches to Bible in Bible view", async ({
  page,
  request,
}) => {
  await expect(async () => {
    const response = await request.get(`${baseURL}/healthz`, {
      timeout: 60_000,
    });
    expect(response.ok()).toBeTruthy();
  }).toPass({ timeout: 90_000 });

  // Navigate to operator in Bible view
  await page.goto(`${baseURL}/ui/operator/bible`);
  await expect(page).toHaveURL(/\/ui\/operator\/bible/);

  // Wait for operator script to initialise
  await page.waitForFunction(() => !!(window as any).__presenterOperatorState, {
    timeout: 30_000,
  });

  // Verify placeholder says "Bible"
  const searchInput = page.locator(
    '.operator__header [data-role="global-search-query"]',
  );
  await expect(searchInput).toBeVisible({ timeout: 10_000 });
  await expect(searchInput).toHaveAttribute("placeholder", /Bible/);

  // Wait for the Bible iframe to be ready with books loaded
  const bibleFrame = page.frameLocator('[data-view-panel="bible"] iframe');
  await expect(async () => {
    const ready = await bibleFrame.locator("body").evaluate(() => {
      const s = (window as any).__presenterBibleState;
      return s && Array.isArray(s.books) && s.books.length > 0;
    });
    expect(ready).toBeTruthy();
  }).toPass({ timeout: 60_000 });

  // Type a Bible search query (min 3 chars)
  await searchInput.fill("God so loved");
  // Wait for search results to appear
  const searchResults = page.locator('[data-role="global-search-results"]');
  await expect(async () => {
    const visible = await searchResults.getAttribute("data-visible");
    expect(visible).toBe("true");
  }).toPass({ timeout: 15_000 });

  // Verify results contain Bible-specific content
  await expect(searchResults.locator("h3")).toHaveText("Bible Verses");
  const resultButtons = searchResults.locator(
    '[data-role="search-result"][data-kind="bible"]',
  );
  await expect(resultButtons.first()).toBeVisible({ timeout: 10_000 });
  const resultCount = await resultButtons.count();
  expect(resultCount).toBeGreaterThan(0);

  // Verify result has reference, translation, and snippet
  const firstResult = resultButtons.first();
  await expect(
    firstResult.locator(".operator__search-result-title"),
  ).toBeVisible();
  await expect(
    firstResult.locator(".operator__search-result-meta"),
  ).toBeVisible();
  await expect(
    firstResult.locator(".operator__search-result-snippet"),
  ).toBeVisible();

  // Get the data attributes from the first result for verification after click
  const bookCode = await firstResult.getAttribute("data-book-code");
  const chapter = await firstResult.getAttribute("data-chapter");
  const verseStart = await firstResult.getAttribute("data-verse-start");

  // Click the first result
  await firstResult.click();

  // Search should be cleared
  await expect(searchInput).toHaveValue("");
  await expect(async () => {
    const visible = await searchResults.getAttribute("data-visible");
    expect(visible).toBe("false");
  }).toPass({ timeout: 5_000 });

  // Verify the Bible iframe received the passage and loaded slides
  await expect(async () => {
    const bibleState = await bibleFrame.locator("body").evaluate(() => {
      const s = (window as any).__presenterBibleState;
      return {
        bookCode: s?.selectedBookCode,
        chapter: s?.selectedChapter,
        verseStart: s?.verseStart,
      };
    });
    expect(bibleState.bookCode).toBe(bookCode);
    expect(String(bibleState.chapter)).toBe(chapter);
    expect(String(bibleState.verseStart)).toBe(verseStart);
  }).toPass({ timeout: 30_000 });

  // Verify slides were generated in the iframe
  await expect(bibleFrame.locator(".operator__slide-card").first()).toBeVisible(
    {
      timeout: 30_000,
    },
  );
});

test("search placeholder changes between views", async ({ page, request }) => {
  await expect(async () => {
    const response = await request.get(`${baseURL}/healthz`, {
      timeout: 60_000,
    });
    expect(response.ok()).toBeTruthy();
  }).toPass({ timeout: 90_000 });

  // Start at worship view
  await page.goto(`${baseURL}/ui/operator`);
  await page.waitForFunction(() => !!(window as any).__presenterOperatorState, {
    timeout: 30_000,
  });

  const searchInput = page.locator(
    '.operator__header [data-role="global-search-query"]',
  );
  await expect(searchInput).toBeVisible({ timeout: 10_000 });

  // Worship placeholder
  await expect(searchInput).toHaveAttribute(
    "placeholder",
    /libraries, songs, slides/,
  );

  // Switch to Bible view
  await page.locator('[data-role="view-toggle"][data-view="bible"]').click();
  await expect(searchInput).toHaveAttribute("placeholder", /Bible/);

  // Switch back to worship
  await page.locator('[data-role="view-toggle"][data-view="worship"]').click();
  await expect(searchInput).toHaveAttribute(
    "placeholder",
    /libraries, songs, slides/,
  );
});

test("switching views clears active search", async ({ page, request }) => {
  await expect(async () => {
    const response = await request.get(`${baseURL}/healthz`, {
      timeout: 60_000,
    });
    expect(response.ok()).toBeTruthy();
  }).toPass({ timeout: 90_000 });

  await page.goto(`${baseURL}/ui/operator`);
  await page.waitForFunction(() => !!(window as any).__presenterOperatorState, {
    timeout: 30_000,
  });

  const searchInput = page.locator(
    '.operator__header [data-role="global-search-query"]',
  );
  await expect(searchInput).toBeVisible({ timeout: 10_000 });

  // Type a search query in worship view
  await searchInput.fill("test search");

  // Wait for search results dropdown to appear
  const searchResults = page.locator('[data-role="global-search-results"]');
  await expect(async () => {
    const visible = await searchResults.getAttribute("data-visible");
    expect(visible).toBe("true");
  }).toPass({ timeout: 15_000 });

  // Switch to Bible view
  await page.locator('[data-role="view-toggle"][data-view="bible"]').click();

  // Input should be cleared and dropdown hidden
  await expect(searchInput).toHaveValue("");
  await expect(async () => {
    const visible = await searchResults.getAttribute("data-visible");
    expect(visible).toBe("false");
  }).toPass({ timeout: 5_000 });
});

test("Bible search requires minimum 3 characters", async ({
  page,
  request,
}) => {
  await expect(async () => {
    const response = await request.get(`${baseURL}/healthz`, {
      timeout: 60_000,
    });
    expect(response.ok()).toBeTruthy();
  }).toPass({ timeout: 90_000 });

  await page.goto(`${baseURL}/ui/operator/bible`);
  await page.waitForFunction(() => !!(window as any).__presenterOperatorState, {
    timeout: 30_000,
  });

  const searchInput = page.locator(
    '.operator__header [data-role="global-search-query"]',
  );
  await expect(searchInput).toBeVisible({ timeout: 10_000 });

  const searchResults = page.locator('[data-role="global-search-results"]');

  // Type 2 chars — should NOT trigger search dropdown
  await searchInput.fill("ab");
  // Wait a bit to ensure no results appear
  await page.waitForTimeout(500);
  const visibleAfter2 = await searchResults.getAttribute("data-visible");
  expect(visibleAfter2).not.toBe("true");

  // Type 3 chars — should trigger search
  await searchInput.fill("abc");
  await expect(async () => {
    const visible = await searchResults.getAttribute("data-visible");
    expect(visible).toBe("true");
  }).toPass({ timeout: 15_000 });
});

test("worship search still works after visiting Bible view", async ({
  page,
  request,
}) => {
  await expect(async () => {
    const response = await request.get(`${baseURL}/healthz`, {
      timeout: 60_000,
    });
    expect(response.ok()).toBeTruthy();
  }).toPass({ timeout: 90_000 });

  // Start on Bible view and do a search
  await page.goto(`${baseURL}/ui/operator/bible`);
  await page.waitForFunction(() => !!(window as any).__presenterOperatorState, {
    timeout: 30_000,
  });

  const searchInput = page.locator(
    '.operator__header [data-role="global-search-query"]',
  );
  await expect(searchInput).toBeVisible({ timeout: 10_000 });
  await searchInput.fill("God");
  const searchResults = page.locator('[data-role="global-search-results"]');
  await expect(async () => {
    const visible = await searchResults.getAttribute("data-visible");
    expect(visible).toBe("true");
  }).toPass({ timeout: 15_000 });

  // Verify Bible results
  await expect(
    searchResults
      .locator('[data-role="search-result"][data-kind="bible"]')
      .first(),
  ).toBeVisible({ timeout: 10_000 });

  // Switch to worship view
  await page.locator('[data-role="view-toggle"][data-view="worship"]').click();

  // Search should be cleared
  await expect(searchInput).toHaveValue("");

  // Do a worship search
  await searchInput.fill("test");
  await expect(async () => {
    const visible = await searchResults.getAttribute("data-visible");
    expect(visible).toBe("true");
  }).toPass({ timeout: 15_000 });

  // Worship results should NOT contain Bible kind
  await expect(async () => {
    const bibleResults = await searchResults
      .locator('[data-role="search-result"][data-kind="bible"]')
      .count();
    expect(bibleResults).toBe(0);
  }).toPass({ timeout: 5_000 });
});

// ---------- Feature 1: Trigger sends edited text ----------
test("triggering edited Bible slide sends edited text", async ({
  page,
  request,
}) => {
  await expect(async () => {
    const response = await request.get(`${baseURL}/healthz`, {
      timeout: 60_000,
    });
    expect(response.ok()).toBeTruthy();
  }).toPass({ timeout: 90_000 });

  await page.goto(`${baseURL}/ui/bible`);
  const liveTab = page.locator('[data-role="bible-tab"][data-tab="live"]');
  await expect(liveTab).toBeVisible({ timeout: 30_000 });

  // Set translation
  const settingsTab = page.locator(
    '[data-role="bible-tab"][data-tab="settings"]',
  );
  await settingsTab.click();
  const mainDropdown = page.locator('[data-role="main-translation"]');
  await expect(mainDropdown).toBeVisible({ timeout: 30_000 });
  const translations: Array<{ code: string }> = await (
    await request.get(`${baseURL}/bible/translations`)
  ).json();
  if (translations.some((t) => t.code === "slk-seb")) {
    await mainDropdown.selectOption("slk-seb");
    await expect(async () => {
      const v = await page.evaluate(
        () =>
          (window as any).__presenterBibleState?.preferences?.mainTranslation,
      );
      expect(v).toBe("slk-seb");
    }).toPass({ timeout: 10_000 });
  }

  // Load a passage
  await liveTab.click();
  await expect(
    page.locator('[data-role="book-list"] button').first(),
  ).toBeVisible({ timeout: 60_000 });
  await page.locator('[data-role="book-filter"]').fill("Jan");
  const johnButton = page
    .locator('[data-role="book-list"] button[data-book-code="JHN"]')
    .first();
  await expect(johnButton).toBeVisible({ timeout: 30_000 });
  await johnButton.click();
  await page.locator('[data-role="chapter-input"]').fill("3");
  await page.locator('[data-role="verse-start"]').fill("16");
  await page.locator('[data-role="verse-end"]').fill("16");
  await page.locator('[data-role="load-button"]').click();
  await page.waitForFunction(
    () => {
      const toast = document.querySelector('[data-role="toast"]');
      return toast && toast.getAttribute("data-visible") === "true";
    },
    { timeout: 60_000 },
  );
  await page.waitForFunction(
    () => {
      const toast = document.querySelector('[data-role="toast"]');
      return !toast || toast.getAttribute("data-visible") !== "true";
    },
    { timeout: 60_000 },
  );

  // Switch to edit mode and modify the text
  const editBtn = page.locator('button[data-mode="edit"]');
  await editBtn.click();
  const mainTextarea = page.locator('[data-role="slide-main"]').first();
  await expect(mainTextarea).toBeVisible({ timeout: 10_000 });
  const editedText = "EDITED TEXT FOR TRIGGER TEST";
  await mainTextarea.fill(editedText);

  // Verify in-memory state updated
  await expect(async () => {
    const firstMain = await page.evaluate(
      () => (window as any).__presenterBibleState?.slides?.[0]?.main,
    );
    expect(firstMain).toBe(editedText);
  }).toPass({ timeout: 5_000 });

  // Trigger the slide
  await page.locator('[data-role="slide-trigger"]').first().click();
  await page.waitForFunction(
    () => {
      const toast = document.querySelector('[data-role="toast"]');
      return (
        toast &&
        toast.getAttribute("data-visible") === "true" &&
        toast.textContent?.includes("Slide triggered")
      );
    },
    { timeout: 60_000 },
  );

  // Verify the broadcast contains the edited text
  const activeResponse = await request.get(`${baseURL}/bible/active`);
  expect(activeResponse.ok()).toBeTruthy();
  const activeJson = await activeResponse.json();
  expect(activeJson?.passage?.text).toBe(editedText);
});

// ---------- Feature 2: Add empty slides in prepared tab ----------
test("add empty slide in prepared tab", async ({ page, request }) => {
  await expect(async () => {
    const response = await request.get(`${baseURL}/healthz`, {
      timeout: 60_000,
    });
    expect(response.ok()).toBeTruthy();
  }).toPass({ timeout: 90_000 });

  await page.goto(`${baseURL}/ui/bible`);
  const preparedTab = page.locator(
    '[data-role="bible-tab"][data-tab="prepared"]',
  );
  await expect(preparedTab).toBeVisible({ timeout: 30_000 });

  // Create a new presentation
  await preparedTab.click();
  const presentationName = `EmptySlide ${Date.now()}`;
  page.once("dialog", async (dialog) => {
    await dialog.accept(presentationName);
  });
  await page.locator('[data-role="presentation-create"]').click();
  await page.waitForFunction(
    () => {
      const toast = document.querySelector('[data-role="toast"]');
      return toast && toast.getAttribute("data-visible") === "true";
    },
    { timeout: 60_000 },
  );
  await page.waitForFunction(
    () => {
      const toast = document.querySelector('[data-role="toast"]');
      return !toast || toast.getAttribute("data-visible") !== "true";
    },
    { timeout: 60_000 },
  );

  // Click on the presentation to activate it
  const presentations = await (
    await request.get(`${baseURL}/bible/presentations`)
  ).json();
  const created = presentations.find(
    (entry: any) => entry.name === presentationName,
  );
  expect(created).toBeTruthy();
  const presentationCard = page.locator(
    `article[data-presentation-id="${created.id}"]`,
  );
  await presentationCard.click();
  await expect(async () => {
    const activeId = await page.evaluate(
      () => (window as any).__presenterBibleState?.activePresentationId,
    );
    expect(activeId).toBe(created.id);
  }).toPass({ timeout: 10_000 });

  // Click "Add empty slide" button
  const addEmptyBtn = page.locator('[data-role="add-empty-slide"]');
  await expect(addEmptyBtn).toBeVisible();
  await addEmptyBtn.click();
  await page.waitForFunction(
    () => {
      const toast = document.querySelector('[data-role="toast"]');
      return (
        toast &&
        toast.getAttribute("data-visible") === "true" &&
        toast.textContent?.includes("Empty slide added")
      );
    },
    { timeout: 60_000 },
  );

  // Verify slide count increased
  await expect(async () => {
    const slideCount = await page.evaluate(
      () =>
        (window as any).__presenterBibleState?.activePresentationSlides
          ?.length ?? 0,
    );
    expect(slideCount).toBeGreaterThanOrEqual(1);
  }).toPass({ timeout: 10_000 });

  // Add another empty slide
  await page.waitForFunction(
    () => {
      const toast = document.querySelector('[data-role="toast"]');
      return !toast || toast.getAttribute("data-visible") !== "true";
    },
    { timeout: 60_000 },
  );
  await addEmptyBtn.click();
  await expect(async () => {
    const slideCount = await page.evaluate(
      () =>
        (window as any).__presenterBibleState?.activePresentationSlides
          ?.length ?? 0,
    );
    expect(slideCount).toBeGreaterThanOrEqual(2);
  }).toPass({ timeout: 10_000 });
});

// ---------- Feature 3: Delete slides from prepared presentation ----------
test("delete slide from prepared presentation", async ({ page, request }) => {
  await expect(async () => {
    const response = await request.get(`${baseURL}/healthz`, {
      timeout: 60_000,
    });
    expect(response.ok()).toBeTruthy();
  }).toPass({ timeout: 90_000 });

  // Create a presentation with slides via API
  const presentationName = `Delete Test ${Date.now()}`;
  const createRes = await request.post(`${baseURL}/bible/presentations`, {
    data: { name: presentationName },
  });
  expect(createRes.ok()).toBeTruthy();
  const presentation = await createRes.json();

  // Add 2 empty slides via API
  await request.post(`${baseURL}/presentations/${presentation.id}/slides`, {
    data: {},
  });
  await request.post(`${baseURL}/presentations/${presentation.id}/slides`, {
    data: {},
  });

  await page.goto(`${baseURL}/ui/bible`);
  const preparedTab = page.locator(
    '[data-role="bible-tab"][data-tab="prepared"]',
  );
  await expect(preparedTab).toBeVisible({ timeout: 30_000 });
  await preparedTab.click();

  // Click on the presentation
  const presentationCard = page.locator(
    `article[data-presentation-id="${presentation.id}"]`,
  );
  await expect(presentationCard).toBeVisible({ timeout: 30_000 });
  await presentationCard.click();
  await expect(async () => {
    const activeId = await page.evaluate(
      () => (window as any).__presenterBibleState?.activePresentationId,
    );
    expect(activeId).toBe(presentation.id);
  }).toPass({ timeout: 10_000 });

  // Switch to edit mode to see the delete buttons
  const editBtn = page.locator('button[data-mode="edit"]');
  await editBtn.click();

  // Get initial slide count
  const initialCount = await page.evaluate(
    () =>
      (window as any).__presenterBibleState?.activePresentationSlides?.length ??
      0,
  );
  expect(initialCount).toBeGreaterThanOrEqual(2);

  // Click delete on the first slide
  const deleteBtn = page.locator('[data-role="delete-slide"]').first();
  await expect(deleteBtn).toBeVisible({ timeout: 10_000 });
  await deleteBtn.click();
  await page.waitForFunction(
    () => {
      const toast = document.querySelector('[data-role="toast"]');
      return (
        toast &&
        toast.getAttribute("data-visible") === "true" &&
        toast.textContent?.includes("Slide deleted")
      );
    },
    { timeout: 60_000 },
  );

  // Verify slide count decreased
  await expect(async () => {
    const count = await page.evaluate(
      () =>
        (window as any).__presenterBibleState?.activePresentationSlides
          ?.length ?? 0,
    );
    expect(count).toBe(initialCount - 1);
  }).toPass({ timeout: 10_000 });
});

// ---------- Feature 4: Reorder slides in prepared presentation ----------
test("reorder slides in prepared presentation", async ({ page, request }) => {
  await expect(async () => {
    const response = await request.get(`${baseURL}/healthz`, {
      timeout: 60_000,
    });
    expect(response.ok()).toBeTruthy();
  }).toPass({ timeout: 90_000 });

  // Create a presentation and add 3 slides via API
  const presentationName = `Reorder Test ${Date.now()}`;
  const createRes = await request.post(`${baseURL}/bible/presentations`, {
    data: { name: presentationName },
  });
  expect(createRes.ok()).toBeTruthy();
  const presentation = await createRes.json();

  // Add empty slides (presentation may already have a default blank slide)
  for (let i = 0; i < 2; i++) {
    await request.post(`${baseURL}/presentations/${presentation.id}/slides`, {
      data: {},
    });
  }

  // Get the slide IDs
  const detailRes = await request.get(
    `${baseURL}/bible/presentations/${presentation.id}`,
  );
  const detail = await detailRes.json();
  const slideIds = detail.slides.map((s: any) => s.id);
  expect(slideIds.length).toBeGreaterThanOrEqual(3);

  // Reorder via API: reverse the order
  const reversed = [...slideIds].reverse();
  const reorderRes = await request.post(
    `${baseURL}/presentations/${presentation.id}/slides/reorder`,
    { data: { slideIds: reversed } },
  );
  expect(reorderRes.ok()).toBeTruthy();

  // Verify the new order persists
  const detailAfter = await request.get(
    `${baseURL}/bible/presentations/${presentation.id}`,
  );
  const afterDetail = await detailAfter.json();
  const afterIds = afterDetail.slides.map((s: any) => s.id);
  expect(afterIds).toEqual(reversed);

  // Also verify via UI
  await page.goto(`${baseURL}/ui/bible`);
  const preparedTab = page.locator(
    '[data-role="bible-tab"][data-tab="prepared"]',
  );
  await expect(preparedTab).toBeVisible({ timeout: 30_000 });
  await preparedTab.click();
  const presentationCard = page.locator(
    `article[data-presentation-id="${presentation.id}"]`,
  );
  await expect(presentationCard).toBeVisible({ timeout: 30_000 });
  await presentationCard.click();

  await expect(async () => {
    const uiSlideIds = await page.evaluate(
      () =>
        (window as any).__presenterBibleState?.activePresentationSlides?.map(
          (s: any) => s.id,
        ) ?? [],
    );
    expect(uiSlideIds).toEqual(reversed);
  }).toPass({ timeout: 10_000 });
});

// ---------- Feature 5: Character limit auto-save and sync ----------
test("character limit auto-saves and syncs", async ({ page, request }) => {
  await expect(async () => {
    const response = await request.get(`${baseURL}/healthz`, {
      timeout: 60_000,
    });
    expect(response.ok()).toBeTruthy();
  }).toPass({ timeout: 90_000 });

  await page.goto(`${baseURL}/ui/bible`);
  const settingsTab = page.locator(
    '[data-role="bible-tab"][data-tab="settings"]',
  );
  await expect(settingsTab).toBeVisible({ timeout: 30_000 });
  await settingsTab.click();

  const charLimitInput = page.locator('[data-role="char-limit"]');
  await expect(charLimitInput).toBeVisible({ timeout: 10_000 });

  // Set a distinctive value
  const testValue = 777;
  await charLimitInput.fill(String(testValue));

  // Wait for debounced auto-save (500ms) + API round trip
  await expect(async () => {
    const prefsRes = await request.get(`${baseURL}/bible/preferences`);
    const prefs = await prefsRes.json();
    expect(prefs.characterLimit).toBe(testValue);
  }).toPass({ timeout: 15_000 });
});

test("character limit used from server on resolve", async ({
  page,
  request,
}) => {
  await expect(async () => {
    const response = await request.get(`${baseURL}/healthz`, {
      timeout: 60_000,
    });
    expect(response.ok()).toBeTruthy();
  }).toPass({ timeout: 90_000 });

  // Set char limit to a small value via API
  await request.put(`${baseURL}/bible/preferences`, {
    data: { characterLimit: 50 },
  });

  // Resolve a passage WITHOUT sending characterLimit — server should use its own
  const resolveRes = await request.post(`${baseURL}/bible/resolve`, {
    data: {
      mainTranslation: "slk-seb",
      book: "Jan",
      bookCode: "JHN",
      chapter: 3,
      verseStart: 16,
      verseEnd: 18,
    },
  });
  expect(resolveRes.ok()).toBeTruthy();
  const resolved = await resolveRes.json();
  // With a 50 char limit and 3 verses, should produce multiple slides
  expect(resolved.slides.length).toBeGreaterThan(1);

  // Now set char limit to large value
  await request.put(`${baseURL}/bible/preferences`, {
    data: { characterLimit: 4000 },
  });

  // Resolve again — should produce fewer slides
  const resolveRes2 = await request.post(`${baseURL}/bible/resolve`, {
    data: {
      mainTranslation: "slk-seb",
      book: "Jan",
      bookCode: "JHN",
      chapter: 3,
      verseStart: 16,
      verseEnd: 18,
    },
  });
  expect(resolveRes2.ok()).toBeTruthy();
  const resolved2 = await resolveRes2.json();
  expect(resolved2.slides.length).toBeLessThanOrEqual(resolved.slides.length);

  // Restore sensible default
  await request.put(`${baseURL}/bible/preferences`, {
    data: { characterLimit: 320 },
  });
});
