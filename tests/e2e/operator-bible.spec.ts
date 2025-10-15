import { expect, test } from '@playwright/test';
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from './support';

let serverHandle: ServerHandle | undefined;
let baseURL = '';

test.describe.configure({ timeout: 300_000 });

test.beforeAll(async ({}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  baseURL = config.baseURL;
  await refreshDevData(config.dbUrl);
  serverHandle = await startTestServer(config.port, config.dbUrl, config.oscPort);
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
});

test('operator bible surface drives live passage broadcast', async ({ page, request }) => {
  await expect(async () => {
    const response = await request.get(`${baseURL}/healthz`, { timeout: 60_000 });
    expect(response.ok()).toBeTruthy();
  }).toPass({ timeout: 90_000 });

  await page.goto(`${baseURL}/ui/bible`);
  await expect(page).toHaveURL(/\/ui\/bible(\?.*)?$/);
  await page.waitForSelector('[data-role="translation-list"]');
  const translationButtons = page.locator('[data-role="translation-list"] .operator__list-button');
  await expect(translationButtons.first()).toBeVisible();
  const activeTranslationButton = page.locator('[data-role="translation-list"] .operator__list-button[data-active="true"]');
  await expect(activeTranslationButton).toHaveCount(1);

  const waitForToastVisible = async () => {
    await page.waitForFunction(() => {
      const toast = document.querySelector('[data-role="toast"]');
      return toast && toast.getAttribute('data-visible') === 'true';
    }, { timeout: 60_000 });
  };
  const waitForToastHidden = async () => {
    await page.waitForFunction(() => {
      const toast = document.querySelector('[data-role="toast"]');
      return !toast || toast.getAttribute('data-visible') !== 'true';
    }, { timeout: 60_000 });
  };

  const translationsHeader = page.locator('.operator__group--translations h2');
  await expect(translationsHeader).toHaveText('Bibles');

  const bibleCountButton = page.locator('[data-role="bible-dashboard"]');
  await expect(bibleCountButton).toBeVisible();
  await expect(bibleCountButton).toHaveText(/\(\d+\)/);
  await expect(page.locator('[data-role="translation-list"] .operator__list-favorite')).toHaveCount(0);

  const translationsResponse = await request.get(`${baseURL}/bible/translations`);
  expect(translationsResponse.ok()).toBeTruthy();
  const translations: Array<{ code: string; name: string; language?: string }> = await translationsResponse.json();
  const stateSnapshot = await page.evaluate(() => (window as any).__presenterBibleState);
  const activeCode = stateSnapshot?.preferences?.mainTranslation ?? (translations[0]?.code ?? '');
  const targetTranslation = translations.find((translation) => translation.code !== activeCode) ?? translations[0];
  const usingActiveTranslation = targetTranslation.code === activeCode;
  const originalLabel = targetTranslation.language && targetTranslation.language.length
    ? `${targetTranslation.name} (${targetTranslation.language})`
    : targetTranslation.name;
  const translationName = targetTranslation.name;
  const translationListHas = async (needle: string) => {
    const labels = await page.locator('[data-role="translation-list"] .operator__list-label').allTextContents();
    return labels.some((text) => text.includes(needle));
  };
  const expectTranslationPresence = async (present: boolean) => {
    await expect.poll(() => translationListHas(translationName)).toBe(present);
  };
  const bibleModal = page.locator('[data-role="bible-modal"]');
  const targetIndex = translations.findIndex((translation) => translation.code === targetTranslation.code);
  const modalRow = page.locator('[data-role="bible-row"]').nth(targetIndex >= 0 ? targetIndex : 0);
  const modalStar = modalRow.locator('[data-action="bible-dashboard-toggle"]');
  const modalEditButton = modalRow.locator('[data-action="bible-edit"]');

  await bibleCountButton.click();
  await expect(bibleModal).toBeVisible();
  await page.waitForSelector('[data-role="bible-modal-list"] [data-role="bible-row"]');
  const modalStars = page.locator('[data-role="bible-modal-list"] [data-action="bible-dashboard-toggle"]');
  await expect(modalStars).toHaveCount(translations.length);
  for (let index = 0; index < translations.length; index += 1) {
    await expect(modalStars.nth(index)).toHaveAttribute('aria-pressed', 'true');
  }
  await expect(modalStar).toHaveAttribute('aria-pressed', 'true');
  await modalStar.click();
  await expect(modalStar).toHaveAttribute('aria-pressed', 'false');
  await page.locator('[data-role="bible-modal-close"]').click();
  await expect(bibleModal).toHaveAttribute('data-open', 'false');
  if (!usingActiveTranslation) {
    await expectTranslationPresence(false);
  }

  await bibleCountButton.click();
  await expect(bibleModal).toBeVisible();
  await modalStar.click();
  await expect(modalStar).toHaveAttribute('aria-pressed', 'true');
  await page.locator('[data-role="bible-modal-close"]').click();
  await expectTranslationPresence(true);

  await bibleCountButton.click();
  await expect(bibleModal).toBeVisible();
  await modalEditButton.click();
  const bibleEditModal = page.locator('[data-role="bible-edit-modal"]');
  await expect(bibleEditModal).toHaveAttribute('data-open', 'true');

  const updatedName = `${targetTranslation.name} (Edited)`;
  const languageBase = targetTranslation.language && targetTranslation.language.trim().length
    ? targetTranslation.language
    : 'Language';
  const updatedLanguage = `${languageBase} (Edited)`;
  await page.locator('[data-role="bible-edit-name"]').fill(updatedName);
  await page.locator('[data-role="bible-edit-language"]').fill(updatedLanguage);
  const editDashboardCheckbox = page.locator('[data-role="bible-edit-dashboard"]');
  await editDashboardCheckbox.uncheck();
  await page.locator('[data-role="bible-edit-save"]').click();
  await waitForToastVisible();
  await waitForToastHidden();

  await expect(modalRow.locator('.operator__list-label')).toHaveText(`${updatedName} (${updatedLanguage})`);
  await expect(modalStar).toHaveAttribute('aria-pressed', 'false');
  await page.locator('[data-role="bible-modal-close"]').click();
  if (!usingActiveTranslation) {
    await expectTranslationPresence(false);
  }

  await bibleCountButton.click();
  await expect(bibleModal).toBeVisible();
  await modalEditButton.click();
  await expect(bibleEditModal).toHaveAttribute('data-open', 'true');
  await page.locator('[data-role="bible-edit-name"]').fill(targetTranslation.name);
  await page.locator('[data-role="bible-edit-language"]').fill(targetTranslation.language ?? '');
  await editDashboardCheckbox.check();
  await page.locator('[data-role="bible-edit-save"]').click();
  await waitForToastVisible();
  await waitForToastHidden();
  await expect(modalRow.locator('.operator__list-label')).toHaveText(originalLabel);
  await expect(modalStar).toHaveAttribute('aria-pressed', 'true');
  await page.locator('[data-role="bible-modal-close"]').click();
  await expect(bibleModal).toHaveAttribute('data-open', 'false');
  await expectTranslationPresence(true);

  const bibleImportButton = page.locator('[data-role="bible-import"]');
  await expect(bibleImportButton).toBeVisible();

  const slovakButton = page.locator('[data-role="translation-list"] .operator__list-button[data-translation-code="slk-seb"]');
  if (await slovakButton.count()) {
    await slovakButton.first().click();
    await expect(async () => {
      const mainTranslation = await page.evaluate(() => (window as any).__presenterBibleState?.preferences?.mainTranslation);
      expect(mainTranslation).toBe('slk-seb');
    }).toPass();
    const activeAfterSwitch = page.locator('[data-role="translation-list"] .operator__list-button[data-active="true"]');
    await expect(activeAfterSwitch).toHaveAttribute('data-translation-code', 'slk-seb');
  }

  await page.locator('[data-role="book-filter"]').fill('Jan');
  const johnButton = page.locator('[data-role="book-list"] button[data-book-code="JHN"]').first();
  await expect(johnButton).toBeVisible({ timeout: 30_000 });
  await johnButton.click();

  await page.locator('[data-role="chapter-input"]').fill('3');
  await page.locator('[data-role="verse-start"]').fill('16');
  await page.locator('[data-role="verse-end"]').fill('18');
  await page.locator('[data-role="load-button"]').click();
  await waitForToastVisible();
  await waitForToastHidden();

  const slideCards = page.locator('.operator__slide-card');
  await expect(slideCards.first()).toBeVisible({ timeout: 60_000 });
  const slideCount = await slideCards.count();
  expect(slideCount).toBeGreaterThan(0);

  const slideMetadata = await page.evaluate(() => {
    const slides = (window as any).__presenterBibleState?.slides ?? [];
    const first = slides[0];
    return first?.metadata?.bible ?? null;
  });
  expect(slideMetadata).toBeTruthy();
  expect(slideMetadata.book || slideMetadata.book_name).toBeTruthy();
  expect(slideMetadata.bookCode ?? slideMetadata.book_code).toBe('JHN');
  expect(slideMetadata.bookNumber ?? slideMetadata.book_number).toBe(43);

  const firstSlideId = await page.evaluate(() => {
    const slides = (window as any).__presenterBibleState?.slides ?? [];
    return slides[0]?.id ?? null;
  });
  expect(firstSlideId).toBeTruthy();

  await slideCards.first().locator('[data-role="slide-select"]').check({ force: true });

  await page.evaluate(() => {
    const slides = (window as any).__presenterBibleState?.slides ?? [];
    const first = slides[0];
    if (!first) {
      throw new Error('No Bible slide available to trigger');
    }
    const card = document.querySelector(`[data-slide-id="${first.id}"]`);
    const trigger = card?.querySelector('[data-role="slide-trigger"]');
    if (!(trigger instanceof HTMLButtonElement)) {
      throw new Error('Bible slide trigger button missing');
    }
    trigger.click();
  });

  await waitForToastVisible();
  const toastText = await page.locator('[data-role="toast"]').innerText();
  expect(toastText).toContain('Slide triggered');
  await waitForToastHidden();

  await page.waitForFunction(() => {
    const active = (window as any).__presenterBibleState?.activeBroadcast;
    if (!active) return false;
    const ref = active.passage?.reference || {};
    const code = ref.book_code ?? ref.bookCode;
    const start = ref.verse_start ?? ref.verseStart;
    return code === 'JHN' && start === 16;
  }, { timeout: 60_000 });

  const activeResponse = await request.get(`${baseURL}/bible/active`);
  expect(activeResponse.ok()).toBeTruthy();
  const activeJson = await activeResponse.json();
  expect(activeJson?.passage?.reference?.book_code ?? activeJson?.passage?.reference?.bookCode).toBe('JHN');
  expect(activeJson?.passage?.reference?.verse_start ?? activeJson?.passage?.reference?.verseStart).toBe(16);

});
