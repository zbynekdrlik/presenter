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

  await page.goto(`${baseURL}/ui/operator`);
  await page.locator('[data-role="view-toggle"][data-view="bible"]').click();
  await expect(page.locator('body')).toHaveAttribute('data-view', 'bible');

  const biblePanel = page.locator('section[data-view-panel="bible"]');
  await expect(biblePanel).toBeVisible();

  await expect(async () => {
    const frame = page.frame({ url: /\/ui\/bible/ });
    expect(frame).toBeTruthy();
  }).toPass({ timeout: 60_000 });
  const bibleFrameHandle = page.frame({ url: /\/ui\/bible/ });
  if (!bibleFrameHandle) {
    throw new Error('Bible iframe not attached');
  }

  const bibleFrame = page.frameLocator('section[data-view-panel="bible"] iframe');
  await expect(bibleFrame.locator('header.operator__header')).toHaveCount(0);

  await expect(async () => {
    const state = await bibleFrame.locator('body').evaluate(() => (window as any).__presenterBibleState);
    expect(state).toBeTruthy();
    expect(Array.isArray(state.books)).toBeTruthy();
  }).toPass({ timeout: 90_000 });

  const slovakButton = bibleFrame.locator('[data-role="translation-list"] button[data-translation-code="slk-seb"]');
  if (await slovakButton.count()) {
    await slovakButton.first().click();
    await expect(async () => {
      const mainTranslation = await bibleFrame.locator('body').evaluate(() => (window as any).__presenterBibleState?.preferences?.mainTranslation);
      expect(mainTranslation).toBe('slk-seb');
    }).toPass();
  }

  const filterInput = bibleFrame.locator('[data-role="book-filter"]');
  await filterInput.fill('Jan');
  const johnButton = bibleFrame.locator('[data-role="book-list"] button[data-book-code="JHN"]').first();
  await expect(johnButton).toBeVisible({ timeout: 30_000 });
  await johnButton.click();

  await bibleFrame.locator('[data-role="chapter-input"]').fill('3');
  await bibleFrame.locator('[data-role="verse-start"]').fill('16');
  await bibleFrame.locator('[data-role="verse-end"]').fill('18');
  await bibleFrame.locator('[data-role="load-button"]').click();
  await bibleFrameHandle.waitForFunction(() => {
    const toast = document.querySelector('[data-role="toast"]');
    return toast && toast.getAttribute('data-visible') === 'true';
  }, undefined, { timeout: 60_000 });
  await bibleFrameHandle.waitForFunction(() => {
    const toast = document.querySelector('[data-role="toast"]');
    return !toast || toast.getAttribute('data-visible') !== 'true';
  }, undefined, { timeout: 60_000 });

  const slideCards = bibleFrame.locator('.operator__slide-card');
  await expect(slideCards.first()).toBeVisible({ timeout: 60_000 });
  const slideCount = await slideCards.count();
  expect(slideCount).toBeGreaterThan(0);

  const slideMetadata = await bibleFrameHandle.evaluate(() => {
    const slides = (window as any).__presenterBibleState?.slides ?? [];
    const first = slides[0];
    return first?.metadata?.bible ?? null;
  });
  expect(slideMetadata).toBeTruthy();
  expect(slideMetadata.book || slideMetadata.book_name).toBeTruthy();
  expect(slideMetadata.bookCode ?? slideMetadata.book_code).toBe('JHN');
  expect(slideMetadata.bookNumber ?? slideMetadata.book_number).toBe(43);


  const firstSlideId = await bibleFrameHandle.evaluate(() => {
    const slides = (window as any).__presenterBibleState?.slides ?? [];
    return slides[0]?.id ?? null;
  });
  expect(firstSlideId).toBeTruthy();

  await slideCards.first().locator('[data-role="slide-select"]').check({ force: true });

  await bibleFrameHandle.evaluate(() => {
    const slides = (window as any).__presenterBibleState?.slides ?? [];
    const first = slides[0];
    if (!first) {
      throw new Error('No Bible slide available to trigger');
    }
    const card = document.querySelector(`[data-slide-id="${first.id}"]`);
    const trigger = card?.querySelector('[data-role="slide-trigger"]') as HTMLButtonElement | null;
    if (!trigger) {
      throw new Error('Bible slide trigger button missing');
    }
    trigger.click();
  });

  await bibleFrameHandle.waitForFunction(() => {
    const toast = document.querySelector('[data-role="toast"]');
    return toast && toast.getAttribute('data-visible') === 'true';
  }, undefined, { timeout: 60_000 });
  const toastText = await bibleFrameHandle.locator('[data-role="toast"]').innerText();
  expect(toastText).toContain('Slide triggered');
  await bibleFrameHandle.waitForFunction(() => {
    const toast = document.querySelector('[data-role="toast"]');
    return !toast || toast.getAttribute('data-visible') !== 'true';
  }, undefined, { timeout: 60_000 });

  await bibleFrameHandle.waitForFunction(() => {
    const active = (window as any).__presenterBibleState?.activeBroadcast;
    if (!active) return false;
    const ref = active.passage?.reference || {};
    const code = ref.book_code ?? ref.bookCode;
    const start = ref.verse_start ?? ref.verseStart;
    return code === 'JHN' && start === 16;
  }, undefined, { timeout: 60_000 });

  const activeResponse = await request.get(`${baseURL}/bible/active`);
  expect(activeResponse.ok()).toBeTruthy();
  const activeJson = await activeResponse.json();
  expect(activeJson?.passage?.reference?.book_code ?? activeJson?.passage?.reference?.bookCode).toBe('JHN');
  expect(activeJson?.passage?.reference?.verse_start ?? activeJson?.passage?.reference?.verseStart).toBe(16);
});
