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

test.describe.configure({ timeout: 300_000 });

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
    expect(triggerResponse.ok()).toBeTruthy();
  } else {
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
