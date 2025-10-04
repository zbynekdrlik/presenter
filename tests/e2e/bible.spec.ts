import { test, expect } from '@playwright/test';
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from './support';

let serverHandle: ServerHandle | undefined;
let baseURL: string;

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

const formatReference = (reference: {
  book: string;
  chapter: number;
  verseStart: number;
  verseEnd: number;
}) => {
  if (reference.verseStart === reference.verseEnd) {
    return `${reference.book} ${reference.chapter}:${reference.verseStart}`;
  }
  return `${reference.book} ${reference.chapter}:${reference.verseStart}-${reference.verseEnd}`;
};

test('can search, trigger, and clear a Bible passage', async ({ page, request }) => {
  page.on('console', (msg) => {
    console.log('bible console', msg.type(), msg.text());
  });
  await expect(async () => {
    const response = await request.get(new URL('/healthz', baseURL).toString(), {
      timeout: 120_000,
    });
    expect(response.ok()).toBeTruthy();
  }).toPass({ timeout: 180_000 });

  await page.goto(new URL('/ui/bible', baseURL).toString());
  await expect(async () => {
    const optionCount = await page.locator('[data-role="translation-select"] option').count();
    expect(optionCount).toBeGreaterThan(0);
  }).toPass({ timeout: 30_000 });

  const translationCode = await page.locator('[data-role="translation-select"]').evaluate((element) => {
    const select = element as HTMLSelectElement;
    return select.value;
  });
  expect(translationCode).toBeTruthy();

  await page.locator('[data-role="query-input"]').fill('loved');
  await page.locator('form[data-role="search-form"] button[type="submit"]').click();

  const result = page.locator('.bible-result').first();
  await result.waitFor({ state: 'visible' });
  const referenceText = (await result.locator('strong').innerText()).trim();
  const passageText = (await result.locator('p').innerText()).trim();

  await result.locator('[data-role="trigger"]').click();

  await expect(page.locator('[data-role="active-reference"]')).toHaveText(referenceText, {
    timeout: 15_000,
  });
  await expect(page.locator('[data-role="active-text"]')).toHaveText(passageText, {
    timeout: 15_000,
  });

  const activeResponse = await request.get(new URL('/bible/active', baseURL).toString(), {
    timeout: 30_000,
  });
  expect(activeResponse.ok()).toBeTruthy();
  const active = await activeResponse.json();
  expect(active).toBeTruthy();
  expect(active.passage.translation.code).toBe(translationCode);
  const expectedReference = formatReference(active.passage.reference);
  expect(referenceText).toBe(expectedReference);
  expect(passageText).toBe(active.passage.text);

  await page.locator('[data-role="clear-button"]').click();
  await expect(page.locator('[data-role="active-reference"]')).toHaveText('No active passage', {
    timeout: 10_000,
  });

  const clearedResponse = await request.get(new URL('/bible/active', baseURL).toString(), {
    timeout: 30_000,
  });
  expect(clearedResponse.ok()).toBeTruthy();
  const cleared = await clearedResponse.json();
  expect(cleared).toBeNull();
});
