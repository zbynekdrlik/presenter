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

  // Wait for the main translation dropdown (replaces old translation-list)
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

  // Bible modal still accessible via All Bibles modal elements
  const bibleModal = page.locator('[data-role="bible-modal"]');
  const bibleEditModal = page.locator('[data-role="bible-edit-modal"]');

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
    // Verify dropdown value persisted
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

  // Select and trigger the first slide
  await slideCards
    .first()
    .locator('[data-role="slide-select"]')
    .check({ force: true });

  await page.evaluate(() => {
    const slides = (window as any).__presenterBibleState?.slides ?? [];
    const first = slides[0];
    if (!first) {
      throw new Error("No Bible slide available to trigger");
    }
    const card = document.querySelector(`[data-slide-id="${first.id}"]`);
    const trigger = card?.querySelector('[data-role="slide-trigger"]');
    if (!(trigger instanceof HTMLButtonElement)) {
      throw new Error("Bible slide trigger button missing");
    }
    trigger.click();
  });

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
});
