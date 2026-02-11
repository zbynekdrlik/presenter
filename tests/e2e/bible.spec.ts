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

  const translationButtons = page.locator(
    '[data-role="translation-list"] button',
  );
  await expect(translationButtons.first()).toBeVisible({ timeout: 30_000 });

  await page.locator('[data-role="char-limit"]').fill("80");
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

  const loadedPassageItems = page.locator(
    '[data-role="loaded-passages"] .operator__list-item',
  );
  await expect(loadedPassageItems.first()).toBeVisible();

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

  const toggleMode = page.locator('[data-role="toggle-mode"]');
  await toggleMode.click();
  await expect(toggleMode).toHaveText("Switch to Live Mode");

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

  await toggleMode.click();
  await expect(toggleMode).toHaveText("Switch to Edit Mode");

  await page.locator('[data-role="slide-select"]').first().check();

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

  const referenceLabel =
    broadcast.start === broadcast.end
      ? `${broadcast.book} ${broadcast.chapter}:${broadcast.start}`
      : `${broadcast.book} ${broadcast.chapter}:${broadcast.start}-${broadcast.end}`;

  await expect(page.locator(".operator__active-card strong")).toHaveText(
    referenceLabel,
    {
      timeout: 15_000,
    },
  );

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
  await expect(page.locator(".operator__active-card strong")).toHaveText(
    "No active passage",
    {
      timeout: 10_000,
    },
  );

  await page
    .locator('[data-role="slide-select"]')
    .first()
    .uncheck({ force: true });
  await page.locator('[data-role="select-all-slides"]').click();
  await expect(page.locator('[data-role="selection-count"]')).toHaveText(
    `${slideCount} selected`,
  );

  const presentationName = `Automation Slides ${Date.now()}`;
  await page.locator('[data-role="presentation-name"]').fill(presentationName);
  await page.locator('[data-role="presentation-add"]').click();

  const toast = page.locator('[data-role="toast"]');
  await expect(toast).toHaveAttribute("data-visible", "true");
  await expect(toast).toContainText("Added");

  await expect(async () => {
    const listText = await page
      .locator('[data-role="presentations-list"]')
      .innerText();
    expect(listText).toContain(presentationName);
  }).toPass({ timeout: 10_000 });

  const presentationsResponse = await request.get(
    new URL("/bible/presentations", baseURL).toString(),
  );
  expect(presentationsResponse.ok()).toBeTruthy();
  const presentations = await presentationsResponse.json();
  const createdSummary = presentations.find(
    (entry: any) => entry.name === presentationName,
  );
  expect(createdSummary).toBeTruthy();

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

  await page
    .locator('[data-role="slide-select"]')
    .first()
    .uncheck({ force: true });
  const checkboxCount = await page
    .locator('[data-role="slide-select"]')
    .count();
  const targetCheckbox =
    checkboxCount > 1
      ? page.locator('[data-role="slide-select"]').nth(1)
      : page.locator('[data-role="slide-select"]').first();
  await targetCheckbox.check({ force: true });
  await page.selectOption(
    '[data-role="presentation-select"]',
    createdSummary.id,
  );
  await page.locator('[data-role="presentation-add"]').click();
  await expect(page.locator('[data-role="toast"]')).toHaveAttribute(
    "data-visible",
    "true",
  );

  const appendedDetailResponse = await request.get(
    new URL(`/bible/presentations/${createdSummary.id}`, baseURL).toString(),
  );
  expect(appendedDetailResponse.ok()).toBeTruthy();
  const appendedDetail = await appendedDetailResponse.json();
  expect(appendedDetail.slides.length).toBeGreaterThan(slideCount);

  const renamedPresentation = `${presentationName} Renamed`;
  const renameButton = page.locator(
    `[data-role="presentation-rename"][data-presentation-id="${createdSummary.id}"]`,
  );
  await renameButton.waitFor({ state: "attached" });
  page.once("dialog", async (dialog) => {
    expect(dialog.type()).toBe("prompt");
    await dialog.accept(renamedPresentation);
  });
  await renameButton.evaluate((node) => {
    (node as HTMLElement).click();
  });
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

  const slovakTranslationButton = page
    .locator('[data-role="translation-list"] button')
    .filter({ hasText: /Slovenský ekumenický preklad/ });
  await slovakTranslationButton.click();
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

  // Verify the character limit input reflects the saved value
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
