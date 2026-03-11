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

test("operator manages Bible workflow end-to-end", async ({
  page,
  request,
}) => {
  await expect(async () => {
    const response = await request.get(
      new URL("/healthz", baseURL).toString(),
      {
        timeout: 120_000,
      },
    );
    expect(response.ok()).toBeTruthy();
  }).toPass({ timeout: 180_000 });

  await page.goto(new URL("/ui/bible", baseURL).toString());

  await page.waitForFunction(
    () => {
      const state = (window as any).__presenterBibleState;
      return !!state && Array.isArray(state.books) && state.books.length > 0;
    },
    { timeout: 120_000 },
  );

  await expect(async () => {
    const firstBook = await page.evaluate(
      () => window.__presenterBibleState.books[0],
    );
    expect(firstBook).toBeTruthy();
    expect(firstBook.code || firstBook.book_code).toBeTruthy();
    expect(typeof (firstBook.number ?? firstBook.book_number)).toBe("number");
    expect(Number(firstBook.number ?? firstBook.book_number)).toBeGreaterThan(
      0,
    );
  }).toPass();

  // LIVE tab is active by default — verify
  const liveTab = page.locator('[data-role="bible-tab"][data-tab="live"]');
  await expect(liveTab).toHaveAttribute("data-active", "true");

  // Translation dropdowns are now in the Live tab
  const mainTranslation = page.locator('[data-role="main-translation"]');
  await expect(mainTranslation).toBeVisible({ timeout: 30_000 });

  // Settings tab: go there to set char limit
  const settingsTab = page.locator(
    '[data-role="bible-tab"][data-tab="settings"]',
  );
  await settingsTab.click();
  await expect(settingsTab).toHaveAttribute("data-active", "true");
  await page.locator('[data-role="char-limit"]').fill("80");

  // Back to LIVE tab for book selection and passage loading
  await liveTab.click();

  await page.locator('[data-role="book-filter"]').fill("John");
  const johnButton = page
    .locator('[data-role="book-list"] button[data-book="John"]')
    .first();
  await expect(johnButton).toBeVisible({ timeout: 10_000 });
  await johnButton.click();
  await expect(async () => {
    const state = await page.evaluate(() => ({
      filteredCount: window.__presenterBibleState.filteredBooks.length,
      selected: window.__presenterBibleState.selectedBook,
    }));
    expect(state.filteredCount).toBe(1);
    expect(state.selected).toBe("John");
  }).toPass();
  await page.locator('[data-role="chapter-input"]').fill("3");
  await page.locator('[data-role="verse-start"]').fill("16");
  await page.locator('[data-role="verse-end"]').fill("18");

  await page.locator('[data-role="load-button"]').click();
  const slides = page.locator(".operator__slide-card");
  await slides.first().waitFor({ state: "visible" });
  const firstSlide = await page.evaluate(
    () => window.__presenterBibleState.slides[0],
  );
  expect(firstSlide).toBeTruthy();
  expect(firstSlide.main).not.toHaveLength(0);
  expect(firstSlide.mainReference).toBeTruthy();
  expect(firstSlide.metadata && firstSlide.metadata.bible).toBeTruthy();
  expect(
    firstSlide.metadata &&
      firstSlide.metadata.bible &&
      (firstSlide.metadata.bible.bookCode ||
        firstSlide.metadata.bible.book_code),
  ).toBe("JHN");
  expect(
    firstSlide.metadata &&
      firstSlide.metadata.bible &&
      (firstSlide.metadata.bible.bookNumber ??
        firstSlide.metadata.bible.book_number),
  ).toBe(43);

  await expect(async () => {
    const historyCount = await page.evaluate(
      () => window.__presenterBibleState.loadedPassages.length,
    );
    expect(historyCount).toBeGreaterThan(0);
  }).toPass();

  const storedPassage = await page.evaluate(
    () => window.__presenterBibleState.loadedPassages[0],
  );
  expect(storedPassage).toBeTruthy();
  expect(storedPassage.bookCode || storedPassage.book_code).toBe("JHN");
  expect(Number(storedPassage.bookNumber ?? storedPassage.book_number)).toBe(
    43,
  );

  const slideCount = await slides.count();
  expect(slideCount).toBeGreaterThan(0);

  // Switch to Edit mode via segmented toggle
  const modeToggle = page.locator(".operator__mode-toggle");
  const editModeBtn = modeToggle.locator('[data-mode="edit"]');
  const liveModeBtn = modeToggle.locator('[data-mode="live"]');
  await editModeBtn.click();
  await expect(editModeBtn).toHaveAttribute("data-active", "true");
  await expect(liveModeBtn).toHaveAttribute("data-active", "false");

  const customTranslation = "Custom translation for testing";
  const customReference = "John 3:16 custom";
  await page
    .locator('[data-role="slide-translation"]')
    .first()
    .fill(customTranslation);
  await page
    .locator('[data-role="slide-main-ref"]')
    .first()
    .fill(customReference);

  await expect(async () => {
    const slide = await page.evaluate(
      () => window.__presenterBibleState.slides[0],
    );
    expect(slide.translation).toBe(customTranslation);
    expect(slide.mainReference).toBe(customReference);
  }).toPass();

  // Switch back to Live mode via segmented toggle
  await liveModeBtn.click();
  await expect(liveModeBtn).toHaveAttribute("data-active", "true");
  await expect(editModeBtn).toHaveAttribute("data-active", "false");

  // Select first slide via select-zone click (new UI)
  await slides.first().locator('[data-role="slide-select-zone"]').click();
  await expect(slides.first()).toHaveClass(/is-selected/);

  const broadcast = await page.evaluate(() => {
    const slide = window.__presenterBibleState.slides[0];
    if (!slide || !slide.metadata || !slide.metadata.bible) {
      return null;
    }
    const bible = slide.metadata.bible;
    const verses = Array.isArray(bible.verses) ? bible.verses : [];
    if (!verses.length) {
      return null;
    }
    const start = verses[0].start;
    const end = verses[verses.length - 1].end;
    const translation = bible.translationCode || bible.translation_code || null;
    return {
      translation,
      book: bible.book,
      chapter: bible.chapter,
      start,
      end,
    };
  });
  expect(broadcast).not.toBeNull();
  expect(broadcast.translation).toBeTruthy();

  const triggerPayload: Record<string, string | number> = {
    translation: broadcast.translation as string,
    book: broadcast.book as string,
    chapter: broadcast.chapter as number,
    verseStart: broadcast.start as number,
  };
  if (broadcast.end !== broadcast.start) {
    triggerPayload.verseEnd = broadcast.end as number;
  }

  const triggerResponse = await request.post(
    new URL("/bible/trigger", baseURL).toString(),
    {
      data: triggerPayload,
    },
  );
  expect(triggerResponse.ok()).toBeTruthy();

  // Verify via API (active passage card removed from UI, rely on state + API)
  const activeResponse = await request.get(
    new URL("/bible/active", baseURL).toString(),
  );
  expect(activeResponse.ok()).toBeTruthy();
  const active = await activeResponse.json();
  expect(active).not.toBeNull();
  expect(active.passage.reference.book).toBe(broadcast.book);
  expect(active.passage.reference.chapter).toBe(broadcast.chapter);
  const activeVerseStart =
    active.passage.reference.verse_start ?? active.passage.reference.verseStart;
  const activeVerseEnd =
    active.passage.reference.verse_end ?? active.passage.reference.verseEnd;
  expect(activeVerseStart).toBe(broadcast.start);
  expect(activeVerseEnd).toBe(broadcast.end);

  await page.locator('[data-role="clear-button"]').click();
  const clearResponse = await request.get(
    new URL("/bible/active", baseURL).toString(),
  );
  expect(clearResponse.ok()).toBeTruthy();
  const clearedActive = await clearResponse.json();
  expect(clearedActive).toBeNull();

  // Deselect first slide then select all manually via select-zone clicks
  await slides.first().locator('[data-role="slide-select-zone"]').click();
  for (let i = 0; i < slideCount; i++) {
    await slides.nth(i).locator('[data-role="slide-select-zone"]').click();
  }
  // Selection count is now in the LIVE tab sidebar
  await expect(page.locator('[data-role="selection-count"]')).toHaveText(
    `${slideCount} selected`,
  );

  // Switch to PREPARED tab for presentation management
  const preparedTab = page.locator(
    '[data-role="bible-tab"][data-tab="prepared"]',
  );
  await preparedTab.click();

  // Create a new presentation via the "+" button
  const presentationName = `Automation Slides ${Date.now()}`;
  page.once("dialog", async (dialog) => {
    expect(dialog.type()).toBe("prompt");
    await dialog.accept(presentationName);
  });
  await page.locator('[data-role="presentation-create"]').click();

  const toast = page.locator('[data-role="toast"]');
  await expect(toast).toHaveAttribute("data-visible", "true");
  await expect(toast).toContainText("Presentation created");

  await expect(async () => {
    const listText = await page
      .locator('[data-role="presentations-list"]')
      .innerText();
    expect(listText).toContain(presentationName);
  }).toPass({ timeout: 10_000 });

  // Verify slide count displays correctly (uses slideCount from API)
  // New presentations start with 1 default placeholder slide
  await expect(async () => {
    const cardText = await page
      .locator('[data-role="presentations-list"]')
      .innerText();
    expect(cardText).toMatch(/\d+ slides?/);
  }).toPass({ timeout: 5_000 });

  // Switch back to LIVE tab to add slides via presentation dropdown
  await liveTab.click();

  const presentationsResponse = await request.get(
    new URL("/bible/presentations", baseURL).toString(),
  );
  expect(presentationsResponse.ok()).toBeTruthy();
  const presentations = await presentationsResponse.json();
  const createdSummary = presentations.find(
    (entry: any) => entry.name === presentationName,
  );
  expect(createdSummary).toBeTruthy();

  await page.selectOption(
    '[data-role="presentation-select"]',
    createdSummary.id,
  );
  await page.locator('[data-role="presentation-add"]').click();
  await expect(toast).toHaveAttribute("data-visible", "true");
  await expect(toast).toContainText("Added");

  const detailResponse = await request.get(
    new URL(`/bible/presentations/${createdSummary.id}`, baseURL).toString(),
  );
  expect(detailResponse.ok()).toBeTruthy();
  const detail = await detailResponse.json();
  expect(detail.slides.length).toBe(slideCount);
  expect(detail.slides[0].translation).toBe(customTranslation);
  const detailMainReference =
    detail.slides[0].main_reference ?? detail.slides[0].mainReference;
  expect(detailMainReference).toBe(customReference);

  // Switch to PREPARED tab and click on the presentation card
  await preparedTab.click();
  const presentationCard = page.locator(
    `article[data-presentation-id="${createdSummary.id}"]`,
  );
  // Click on the presentation name (strong element) to avoid hitting the edit button
  await presentationCard.locator("strong").click();
  await expect(async () => {
    const activeId = await page.evaluate(
      () => window.__presenterBibleState.activePresentationId,
    );
    expect(activeId).toBe(createdSummary.id);
  }).toPass({ timeout: 10_000 });
  await expect(presentationCard).toHaveClass(/is-active/);

  // Verify presentation slides are shown as simple cards (no trigger zone UI, no select zone)
  const presentationSlides = page.locator(".operator__slide-card");
  await expect(presentationSlides.first()).toBeVisible({ timeout: 10_000 });
  const presentationSlideCount = await presentationSlides.count();
  expect(presentationSlideCount).toBe(slideCount);
  // Prepared tab slides are clean cards (whole card clickable, no trigger zone or select zone)
  await expect(
    presentationSlides.first().locator(".operator__slide-trigger-zone--full"),
  ).toHaveCount(0);
  await expect(
    presentationSlides.first().locator('[data-role="slide-select-zone"]'),
  ).toHaveCount(0);
  // The card body is directly visible
  await expect(
    presentationSlides.first().locator(".operator__slide-bodies"),
  ).toBeVisible();

  // Verify slide count shows correct number after slides were added
  await expect(async () => {
    const cardText = await page
      .locator(`article[data-presentation-id="${createdSummary.id}"]`)
      .innerText();
    expect(cardText).toMatch(/\d+ slides?/);
    expect(cardText).not.toContain("0 slide");
  }).toPass({ timeout: 10_000 });

  // Rename the presentation via edit modal (pen icon)
  const renamedPresentation = `${presentationName} Renamed`;
  const editButton = page.locator(
    `[data-role="presentation-edit"][data-presentation-id="${createdSummary.id}"]`,
  );
  await editButton.waitFor({ state: "attached" });
  await editButton.click();

  // Verify edit modal is open
  const editModal = page.locator('[data-role="bible-presentation-edit-modal"]');
  await expect(editModal).toHaveAttribute("data-open", "true");

  // Fill new name and save
  const nameInput = page.locator('[data-role="bible-presentation-edit-name"]');
  await nameInput.fill(renamedPresentation);
  await page.locator('[data-role="bible-presentation-edit-save"]').click();

  await expect(toast).toContainText("Presentation renamed");
  await expect(editModal).toHaveAttribute("data-open", "false");

  await expect(async () => {
    const listText = await page
      .locator('[data-role="presentations-list"]')
      .innerText();
    expect(listText).toContain(renamedPresentation);
  }).toPass({ timeout: 10_000 });
  await expect(async () => {
    const state = await page.evaluate(
      () => window.__presenterBibleState.presentations,
    );
    const renamed = Array.isArray(state)
      ? state.find((entry: any) => entry && entry.name === renamedPresentation)
      : null;
    expect(renamed).toBeTruthy();
  }).toPass();
  const renamedDetailResponse = await request.get(
    new URL(`/bible/presentations/${createdSummary.id}`, baseURL).toString(),
  );
  expect(renamedDetailResponse.ok()).toBeTruthy();
  const renamedDetail = await renamedDetailResponse.json();
  expect(renamedDetail.name).toBe(renamedPresentation);

  // Delete the presentation via edit modal
  const deleteEditButton = page.locator(
    `[data-role="presentation-edit"][data-presentation-id="${createdSummary.id}"]`,
  );
  await deleteEditButton.click();
  await expect(editModal).toHaveAttribute("data-open", "true");

  page.once("dialog", async (dialog) => {
    expect(dialog.type()).toBe("confirm");
    await dialog.accept();
  });
  await page.locator('[data-role="bible-presentation-edit-delete"]').click();

  await expect(toast).toContainText("Presentation deleted");
  await expect(editModal).toHaveAttribute("data-open", "false");

  // Verify presentation is gone from list
  await expect(async () => {
    const listText = await page
      .locator('[data-role="presentations-list"]')
      .innerText();
    expect(listText).not.toContain(renamedPresentation);
  }).toPass({ timeout: 10_000 });

  // Verify via API
  const deletedResponse = await request.get(
    new URL(`/bible/presentations/${createdSummary.id}`, baseURL).toString(),
  );
  expect(deletedResponse.status()).toBe(404);

  // Change translation via dropdown (now in Live tab)
  await liveTab.click();
  const mainTranslationDropdown = page.locator(
    '[data-role="main-translation"]',
  );
  await expect(mainTranslationDropdown).toBeVisible({ timeout: 10_000 });
  await mainTranslationDropdown.selectOption("slk-seb");
  await expect(async () => {
    const preferences = await page.evaluate(
      () => window.__presenterBibleState.preferences,
    );
    expect(preferences.mainTranslation).toBe("slk-seb");
  }).toPass();
  await page.locator('[data-role="book-filter"]').fill("Ján");
  const janButton = page
    .locator('[data-role="book-list"] button[data-book="Ján"]')
    .first();
  await expect(janButton).toBeVisible({ timeout: 10_000 });
  await janButton.click();
  await expect(async () => {
    const state = await page.evaluate(() => ({
      filteredCount: window.__presenterBibleState.filteredBooks.length,
      selected: window.__presenterBibleState.selectedBook,
    }));
    expect(state.filteredCount).toBe(1);
    expect(state.selected).toBe("Ján");
  }).toPass();
  await page.locator('[data-role="chapter-input"]').fill("3");
  await page.locator('[data-role="verse-start"]').fill("1");
  await page.locator('[data-role="verse-end"]').fill("");
  await page.locator('[data-role="load-button"]').click();

  await expect(async () => {
    const state = await page.evaluate(() => {
      const slides = window.__presenterBibleState.slides;
      const first = slides[0];
      const translation =
        first?.metadata?.bible?.translation_code ??
        first?.metadata?.bible?.translationCode ??
        null;
      return {
        loading: window.__presenterBibleState.loadingSlides,
        translation,
        count: slides.length,
      };
    });
    expect(state.loading).toBeFalsy();
    expect(state.translation).toBe("slk-seb");
    expect(state.count).toBeGreaterThan(0);
  }).toPass({ timeout: 15_000 });

  const slovakSlides = page.locator(".operator__slide-card");
  await expect(slovakSlides.first()).toBeVisible();
  const lastVerseEnd = await page.evaluate(() => {
    const slides = window.__presenterBibleState.slides;
    const lastSlide = slides[slides.length - 1];
    const verseMeta = lastSlide?.metadata?.bible?.verses;
    return Array.isArray(verseMeta) && verseMeta.length
      ? verseMeta[verseMeta.length - 1].end
      : null;
  });
  expect(lastVerseEnd).toBe(36);
});

test("bible preferences API round-trip", async ({ request }) => {
  await expect(async () => {
    const response = await request.get(
      new URL("/healthz", baseURL).toString(),
      {
        timeout: 120_000,
      },
    );
    expect(response.ok()).toBeTruthy();
  }).toPass({ timeout: 180_000 });

  // GET returns valid preferences (may have been set by previous test)
  const initialResponse = await request.get(
    new URL("/bible/preferences", baseURL).toString(),
  );
  expect(initialResponse.ok()).toBeTruthy();
  const initial = await initialResponse.json();
  expect(typeof initial.characterLimit).toBe("number");

  // PUT preferences with specific values
  const putResponse = await request.put(
    new URL("/bible/preferences", baseURL).toString(),
    {
      data: {
        mainTranslation: "eng-kjv",
        secondaryTranslation: "slk-seb",
        characterLimit: 200,
      },
    },
  );
  expect(putResponse.status()).toBe(204);

  // GET persisted preferences
  const savedResponse = await request.get(
    new URL("/bible/preferences", baseURL).toString(),
  );
  expect(savedResponse.ok()).toBeTruthy();
  const saved = await savedResponse.json();
  expect(saved.mainTranslation).toBe("eng-kjv");
  expect(saved.secondaryTranslation).toBe("slk-seb");
  expect(saved.characterLimit).toBe(200);

  // Partial update via PUT (only change character limit)
  const partialResponse = await request.put(
    new URL("/bible/preferences", baseURL).toString(),
    {
      data: {
        characterLimit: 400,
      },
    },
  );
  expect(partialResponse.status()).toBe(204);

  // Verify partial update kept existing fields
  const afterPartial = await request.get(
    new URL("/bible/preferences", baseURL).toString(),
  );
  expect(afterPartial.ok()).toBeTruthy();
  const partial = await afterPartial.json();
  expect(partial.mainTranslation).toBe("eng-kjv");
  expect(partial.secondaryTranslation).toBe("slk-seb");
  expect(partial.characterLimit).toBe(400);
});

test("bible preferences persist across page reloads", async ({
  page,
  request,
}) => {
  await expect(async () => {
    const response = await request.get(
      new URL("/healthz", baseURL).toString(),
      {
        timeout: 120_000,
      },
    );
    expect(response.ok()).toBeTruthy();
  }).toPass({ timeout: 180_000 });

  // Set preferences via API first to ensure clean state
  await request.put(new URL("/bible/preferences", baseURL).toString(), {
    data: {
      mainTranslation: "eng-kjv",
      secondaryTranslation: "slk-seb",
      characterLimit: 250,
    },
  });

  // Load the Bible page
  await page.goto(new URL("/ui/bible", baseURL).toString());

  await page.waitForFunction(
    () => {
      const state = (window as any).__presenterBibleState;
      return !!state && Array.isArray(state.books) && state.books.length > 0;
    },
    { timeout: 120_000 },
  );

  // Verify preferences were loaded from API
  await expect(async () => {
    const prefs = await page.evaluate(
      () => window.__presenterBibleState.preferences,
    );
    expect(prefs.mainTranslation).toBe("eng-kjv");
    expect(prefs.secondaryTranslation).toBe("slk-seb");
    expect(prefs.characterLimit).toBe(250);
  }).toPass({ timeout: 15_000 });

  // Switch to SETTINGS tab to verify the character limit input
  const settingsTab = page.locator(
    '[data-role="bible-tab"][data-tab="settings"]',
  );
  await settingsTab.click();
  await expect(page.locator('[data-role="char-limit"]')).toHaveValue("250");

  // Reload the page
  await page.reload();

  await page.waitForFunction(
    () => {
      const state = (window as any).__presenterBibleState;
      return !!state && Array.isArray(state.books) && state.books.length > 0;
    },
    { timeout: 120_000 },
  );

  // Verify preferences survived the reload
  await expect(async () => {
    const prefs = await page.evaluate(
      () => window.__presenterBibleState.preferences,
    );
    expect(prefs.mainTranslation).toBe("eng-kjv");
    expect(prefs.secondaryTranslation).toBe("slk-seb");
    expect(prefs.characterLimit).toBe(250);
  }).toPass({ timeout: 15_000 });
});

test("main translation dropdown selects translation and loads books", async ({
  page,
  request,
}) => {
  await expect(async () => {
    const response = await request.get(
      new URL("/healthz", baseURL).toString(),
      { timeout: 120_000 },
    );
    expect(response.ok()).toBeTruthy();
  }).toPass({ timeout: 180_000 });

  await page.goto(new URL("/ui/bible", baseURL).toString());

  await page.waitForFunction(
    () => {
      const state = (window as any).__presenterBibleState;
      return !!state && Array.isArray(state.books) && state.books.length > 0;
    },
    { timeout: 120_000 },
  );

  // Translation dropdowns are now in the Live tab (the default)
  // Main translation dropdown should be visible
  const mainDropdown = page.locator('[data-role="main-translation"]');
  await expect(mainDropdown).toBeVisible({ timeout: 10_000 });

  // Secondary translation dropdown should be visible with None option
  const secondaryDropdown = page.locator('[data-role="secondary-translation"]');
  await expect(secondaryDropdown).toBeVisible();

  // Verify "Loaded verses" section is removed
  await expect(page.locator(".operator__group--passages")).toHaveCount(0);

  // Select a different translation via the main dropdown
  await mainDropdown.selectOption("slk-seb");

  await expect(async () => {
    const prefs = await page.evaluate(
      () => window.__presenterBibleState.preferences,
    );
    expect(prefs.mainTranslation).toBe("slk-seb");
  }).toPass({ timeout: 15_000 });

  // Verify dropdown reflects the selected translation
  await expect(mainDropdown).toHaveValue("slk-seb");

  // Verify books loaded for the new translation
  await expect(async () => {
    const bookCount = await page.evaluate(
      () => window.__presenterBibleState.books.length,
    );
    expect(bookCount).toBeGreaterThan(0);
  }).toPass({ timeout: 15_000 });

  // Switch back to English via dropdown
  await mainDropdown.selectOption("eng-kjv");
  await expect(async () => {
    const prefs = await page.evaluate(
      () => window.__presenterBibleState.preferences,
    );
    expect(prefs.mainTranslation).toBe("eng-kjv");
  }).toPass({ timeout: 15_000 });

  // Verify the dropdown reflects the selection
  await expect(mainDropdown).toHaveValue("eng-kjv");
});

test("create-new presentation from LIVE tab dropdown", async ({
  page,
  request,
}) => {
  await expect(async () => {
    const response = await request.get(
      new URL("/healthz", baseURL).toString(),
      { timeout: 120_000 },
    );
    expect(response.ok()).toBeTruthy();
  }).toPass({ timeout: 180_000 });

  await page.goto(new URL("/ui/bible", baseURL).toString());

  await page.waitForFunction(
    () => {
      const state = (window as any).__presenterBibleState;
      return !!state && Array.isArray(state.books) && state.books.length > 0;
    },
    { timeout: 120_000 },
  );

  // Load a passage first so we have slides to add
  await page.locator('[data-role="book-filter"]').fill("John");
  const johnButton = page
    .locator('[data-role="book-list"] button[data-book="John"]')
    .first();
  await expect(johnButton).toBeVisible({ timeout: 10_000 });
  await johnButton.click();
  await page.locator('[data-role="chapter-input"]').fill("1");
  await page.locator('[data-role="verse-start"]').fill("1");
  await page.locator('[data-role="verse-end"]').fill("3");
  await page.locator('[data-role="load-button"]').click();

  const slides = page.locator(".operator__slide-card");
  await slides.first().waitFor({ state: "visible" });
  const slideCount = await slides.count();
  expect(slideCount).toBeGreaterThan(0);

  // Select all slides via select-zone clicks
  for (let i = 0; i < slideCount; i++) {
    await slides.nth(i).locator('[data-role="slide-select-zone"]').click();
  }

  // Verify the dropdown has "+ New presentation" option
  const presentationSelect = page.locator('[data-role="presentation-select"]');
  await expect(
    presentationSelect.locator('option[value="__new__"]'),
  ).toHaveCount(1);

  // Select the __new__ option — dialog will prompt for name
  const newPresentationName = `Inline Created ${Date.now()}`;
  page.once("dialog", async (dialog) => {
    expect(dialog.type()).toBe("prompt");
    await dialog.accept(newPresentationName);
  });
  await presentationSelect.selectOption("__new__");
  await page.locator('[data-role="presentation-add"]').click();

  const toast = page.locator('[data-role="toast"]');
  await expect(toast).toHaveAttribute("data-visible", "true");
  await expect(toast).toContainText("Added");

  // Verify presentation was created via API
  await expect(async () => {
    const response = await request.get(
      new URL("/bible/presentations", baseURL).toString(),
    );
    expect(response.ok()).toBeTruthy();
    const presentations = await response.json();
    const found = presentations.find(
      (entry: any) => entry.name === newPresentationName,
    );
    expect(found).toBeTruthy();
    expect(found.slideCount).toBe(slideCount);
  }).toPass({ timeout: 10_000 });

  // Cleanup: delete the created presentation
  const listResponse = await request.get(
    new URL("/bible/presentations", baseURL).toString(),
  );
  const allPresentations = await listResponse.json();
  const created = allPresentations.find(
    (entry: any) => entry.name === newPresentationName,
  );
  if (created) {
    await request.delete(
      new URL(`/bible/presentations/${created.id}`, baseURL).toString(),
    );
  }
});

test("translation text hidden when no secondary bible selected", async ({
  page,
  request,
}) => {
  await expect(async () => {
    const response = await request.get(
      new URL("/healthz", baseURL).toString(),
      { timeout: 120_000 },
    );
    expect(response.ok()).toBeTruthy();
  }).toPass({ timeout: 180_000 });

  // Set preferences with a secondary translation
  await request.put(new URL("/bible/preferences", baseURL).toString(), {
    data: {
      mainTranslation: "eng-kjv",
      secondaryTranslation: "slk-seb",
      characterLimit: 320,
    },
  });

  await page.goto(new URL("/ui/bible", baseURL).toString());

  await page.waitForFunction(
    () => {
      const state = (window as any).__presenterBibleState;
      return !!state && Array.isArray(state.books) && state.books.length > 0;
    },
    { timeout: 120_000 },
  );

  // Load a passage
  await page.locator('[data-role="book-filter"]').fill("John");
  const johnButton = page
    .locator('[data-role="book-list"] button[data-book="John"]')
    .first();
  await expect(johnButton).toBeVisible({ timeout: 10_000 });
  await johnButton.click();
  await page.locator('[data-role="chapter-input"]').fill("3");
  await page.locator('[data-role="verse-start"]').fill("16");
  await page.locator('[data-role="verse-end"]').fill("16");
  await page.locator('[data-role="load-button"]').click();

  const slides = page.locator(".operator__slide-card");
  await slides.first().waitFor({ state: "visible" });

  // With secondary translation set, translation text should be visible
  await expect(async () => {
    const hasTranslation = await page.evaluate(() => {
      const state = window.__presenterBibleState;
      return (
        state.slides.length > 0 &&
        !!state.slides[0].translation &&
        state.slides[0].translation.trim().length > 0
      );
    });
    expect(hasTranslation).toBeTruthy();
  }).toPass({ timeout: 15_000 });

  await expect(
    slides.first().locator(".operator__slide-text--translation"),
  ).not.toBeEmpty();

  // Now remove the secondary translation (dropdowns are in Live tab)
  await page.locator('[data-role="secondary-translation"]').selectOption("");

  // Wait for auto-save to persist the preference
  await expect(async () => {
    const prefs = await page.evaluate(
      () => window.__presenterBibleState.preferences,
    );
    expect(prefs.secondaryTranslation).toBe("");
  }).toPass({ timeout: 5_000 });

  await page.locator('[data-role="load-button"]').click();
  await slides.first().waitFor({ state: "visible" });

  // Translation text should now be hidden
  await expect(
    slides.first().locator(".operator__slide-text--translation"),
  ).toHaveCount(0);

  // Translation reference should also be hidden
  await expect(
    slides.first().locator(".operator__slide-reference--secondary"),
  ).toHaveCount(0);
});

test("content search across translations finds and loads verse", async ({
  page,
  request,
}) => {
  await expect(async () => {
    const response = await request.get(
      new URL("/healthz", baseURL).toString(),
      { timeout: 120_000 },
    );
    expect(response.ok()).toBeTruthy();
  }).toPass({ timeout: 180_000 });

  // API-level test: cross-translation search (no translation param)
  const crossResponse = await request.get(
    new URL(
      "/bible/search?query=God%20so%20loved&limit=10",
      baseURL,
    ).toString(),
  );
  expect(crossResponse.ok()).toBeTruthy();
  const crossResults = await crossResponse.json();
  expect(crossResults.length).toBeGreaterThan(0);
  // Results should include passages from at least one translation
  const translationCodes = new Set(
    crossResults.map(
      (p: any) => p.translation?.code || p.translation_code || "",
    ),
  );
  expect(translationCodes.size).toBeGreaterThan(0);

  // API-level test: backwards compat — translation param still works
  const kjvResponse = await request.get(
    new URL(
      "/bible/search?translation=eng-kjv&query=God%20so%20loved&limit=10",
      baseURL,
    ).toString(),
  );
  expect(kjvResponse.ok()).toBeTruthy();
  const kjvResults = await kjvResponse.json();
  expect(kjvResults.length).toBeGreaterThan(0);
  for (const passage of kjvResults) {
    const code = passage.translation?.code || passage.translation_code || "";
    expect(code).toBe("eng-kjv");
  }

  // UI test: load bible page and use content search
  await page.goto(new URL("/ui/bible", baseURL).toString());

  await page.waitForFunction(
    () => {
      const state = (window as any).__presenterBibleState;
      return !!state && Array.isArray(state.books) && state.books.length > 0;
    },
    { timeout: 120_000 },
  );

  // Verify header search input is visible
  const searchInput = page.locator('[data-role="global-search-query"]');
  await expect(searchInput).toBeVisible({ timeout: 10_000 });

  // Type a phrase and wait for dropdown results
  await searchInput.fill("God so loved");
  await expect(async () => {
    const resultCount = await page
      .locator(".operator__search-result button")
      .count();
    expect(resultCount).toBeGreaterThan(0);
  }).toPass({ timeout: 15_000 });

  // Verify dropdown is visible
  const dropdown = page.locator('[data-role="global-search-results"]');
  await expect(dropdown).toHaveAttribute("data-visible", "true");

  // Verify result items show reference, translation, and snippet with content
  const firstResult = page.locator(".operator__search-result button").first();
  await expect(
    firstResult.locator(".operator__search-result-title"),
  ).not.toBeEmpty();
  await expect(
    firstResult.locator(".operator__search-result-meta"),
  ).not.toBeEmpty();
  await expect(
    firstResult.locator(".operator__search-result-snippet"),
  ).not.toBeEmpty();

  // Click first result
  await firstResult.click();

  // Verify search input is cleared and dropdown is closed
  await expect(searchInput).toHaveValue("");
  await expect(dropdown).toHaveAttribute("data-visible", "false");

  // Verify slides were loaded
  await expect(async () => {
    const slideCount = await page.evaluate(
      () => window.__presenterBibleState.slides.length,
    );
    expect(slideCount).toBeGreaterThan(0);
  }).toPass({ timeout: 15_000 });

  const slides = page.locator(".operator__slide-card");
  await slides.first().waitFor({ state: "visible" });

  // Verify the book/chapter/verse were populated
  await expect(async () => {
    const state = await page.evaluate(() => ({
      book: window.__presenterBibleState.selectedBook,
      chapter: window.__presenterBibleState.selectedChapter,
      verseStart: window.__presenterBibleState.verseStart,
    }));
    expect(state.book).toBeTruthy();
    expect(state.chapter).toBeGreaterThan(0);
    expect(state.verseStart).toBeGreaterThan(0);
  }).toPass();
});

test("content search minimum character validation", async ({ request }) => {
  await expect(async () => {
    const response = await request.get(
      new URL("/healthz", baseURL).toString(),
      { timeout: 120_000 },
    );
    expect(response.ok()).toBeTruthy();
  }).toPass({ timeout: 180_000 });

  // Single char query — 400 error
  const singleCharResponse = await request.get(
    new URL("/bible/search?query=a", baseURL).toString(),
  );
  expect(singleCharResponse.status()).toBe(400);

  // Empty query — 400 error
  const emptyResponse = await request.get(
    new URL("/bible/search?query=", baseURL).toString(),
  );
  expect(emptyResponse.status()).toBe(400);

  // Two char query — 200 OK
  const twoCharResponse = await request.get(
    new URL("/bible/search?query=of", baseURL).toString(),
  );
  expect(twoCharResponse.ok()).toBeTruthy();
});

test("header search dropdown closes on Escape and click-outside", async ({
  page,
  request,
}) => {
  await expect(async () => {
    const response = await request.get(
      new URL("/healthz", baseURL).toString(),
      { timeout: 120_000 },
    );
    expect(response.ok()).toBeTruthy();
  }).toPass({ timeout: 180_000 });

  await page.goto(new URL("/ui/bible", baseURL).toString());

  await page.waitForFunction(
    () => {
      const state = (window as any).__presenterBibleState;
      return !!state && Array.isArray(state.books) && state.books.length > 0;
    },
    { timeout: 120_000 },
  );

  const searchInput = page.locator('[data-role="global-search-query"]');
  const dropdown = page.locator('[data-role="global-search-results"]');

  // Type a query and wait for results
  await searchInput.fill("God so loved");
  await expect(async () => {
    const count = await page.locator(".operator__search-result button").count();
    expect(count).toBeGreaterThan(0);
  }).toPass({ timeout: 15_000 });
  await expect(dropdown).toHaveAttribute("data-visible", "true");

  // Press Escape — dropdown should close and input should be cleared
  await searchInput.press("Escape");
  await expect(dropdown).toHaveAttribute("data-visible", "false");
  await expect(searchInput).toHaveValue("");

  // Type again and wait for results
  await searchInput.fill("God so loved");
  await expect(async () => {
    const count = await page.locator(".operator__search-result button").count();
    expect(count).toBeGreaterThan(0);
  }).toPass({ timeout: 15_000 });
  await expect(dropdown).toHaveAttribute("data-visible", "true");

  // Click outside — dropdown should close, input cleared
  await page.locator("h1").click();
  await expect(dropdown).toHaveAttribute("data-visible", "false");
  await expect(searchInput).toHaveValue("");
});

test("header search clear button", async ({ page, request }) => {
  await expect(async () => {
    const response = await request.get(
      new URL("/healthz", baseURL).toString(),
      { timeout: 120_000 },
    );
    expect(response.ok()).toBeTruthy();
  }).toPass({ timeout: 180_000 });

  await page.goto(new URL("/ui/bible", baseURL).toString());

  await page.waitForFunction(
    () => {
      const state = (window as any).__presenterBibleState;
      return !!state && Array.isArray(state.books) && state.books.length > 0;
    },
    { timeout: 120_000 },
  );

  const searchInput = page.locator('[data-role="global-search-query"]');
  const clearButton = page.locator('[data-role="global-search-clear"]');
  const dropdown = page.locator('[data-role="global-search-results"]');

  // Clear button is hidden initially
  await expect(clearButton).toBeHidden();

  // Type query — clear button becomes visible
  await searchInput.fill("God so loved");
  await expect(clearButton).toBeVisible();

  // Wait for results
  await expect(async () => {
    const count = await page.locator(".operator__search-result button").count();
    expect(count).toBeGreaterThan(0);
  }).toPass({ timeout: 15_000 });

  // Click clear — input empty, dropdown hidden, clear button hidden
  await clearButton.click();
  await expect(searchInput).toHaveValue("");
  await expect(dropdown).toHaveAttribute("data-visible", "false");
  await expect(clearButton).toBeHidden();
});
