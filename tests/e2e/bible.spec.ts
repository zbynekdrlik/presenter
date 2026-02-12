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

  // Settings tab: go there to set char limit
  const settingsTab = page.locator(
    '[data-role="bible-tab"][data-tab="settings"]',
  );
  await settingsTab.click();
  await expect(settingsTab).toHaveAttribute("data-active", "true");

  const mainTranslation = page.locator('[data-role="main-translation"]');
  await expect(mainTranslation).toBeVisible({ timeout: 30_000 });

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
  await presentationCard.click();
  await expect(async () => {
    const activeId = await page.evaluate(
      () => window.__presenterBibleState.activePresentationId,
    );
    expect(activeId).toBe(createdSummary.id);
  }).toPass({ timeout: 10_000 });
  await expect(presentationCard).toHaveClass(/is-active/);

  // Verify presentation slides are shown as triggerOnly cards (no select zone)
  const presentationSlides = page.locator(".operator__slide-card");
  await expect(presentationSlides.first()).toBeVisible({ timeout: 10_000 });
  const presentationSlideCount = await presentationSlides.count();
  expect(presentationSlideCount).toBe(slideCount);
  // TriggerOnly slides have full trigger zone, no select zone
  await expect(
    presentationSlides.first().locator(".operator__slide-trigger-zone--full"),
  ).toBeVisible();
  await expect(
    presentationSlides.first().locator('[data-role="slide-select-zone"]'),
  ).toHaveCount(0);

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

  // Switch to SETTINGS tab and change translation via dropdown
  await settingsTab.click();
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

  // Switch back to LIVE tab and load Slovak passage
  await liveTab.click();
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

  // Switch to SETTINGS tab where translation dropdowns live
  const settingsTab = page.locator(
    '[data-role="bible-tab"][data-tab="settings"]',
  );
  await settingsTab.click();

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
  ).toBeVisible();

  // Now remove the secondary translation via SETTINGS tab
  const settingsTab = page.locator(
    '[data-role="bible-tab"][data-tab="settings"]',
  );
  await settingsTab.click();
  await page.locator('[data-role="secondary-translation"]').selectOption("");
  await page.locator('[data-role="save-preferences"]').click();

  // Switch back to LIVE tab and reload passage
  const liveTab = page.locator('[data-role="bible-tab"][data-tab="live"]');
  await liveTab.click();

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
