import { test, expect, Page, BrowserContext } from '@playwright/test';
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from './support';

let serverHandle: ServerHandle | undefined;
let baseURL: string;
let dbUrl: string;
let port: number;

test.describe.configure({ timeout: 420_000 });

async function waitForOperatorReady(page: Page) {
  await page.goto(new URL('/ui/operator', baseURL).toString(), {
    waitUntil: 'domcontentloaded',
  });
  await page.waitForLoadState('networkidle');
  await page.waitForFunction(() => window.__presenterLiveConnected === true, {
    timeout: 30_000,
  });
}

async function openStageDisplay(context: BrowserContext, options?: { forceLegacy?: boolean }) {
  await context.request.post(new URL('/stage/layout', baseURL).toString(), {
    data: { code: 'worship-snv' },
  });
  const stagePage = await context.newPage();
  if (options?.forceLegacy) {
    await stagePage.addInitScript(() => {
      window.PRESENTER_STAGE_TEST_CONFIG = Object.assign({}, window.PRESENTER_STAGE_TEST_CONFIG, {
        forceLegacyClientId: true,
      });
    });
  }
  await stagePage.goto(new URL('/stage', baseURL).toString(), {
    waitUntil: 'domcontentloaded',
  });
  await stagePage.waitForFunction(() => window.__presenterStageConnectionState === 'connected', {
    timeout: 30_000,
  });
  return stagePage;
}

test.beforeAll(async ({}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  baseURL = config.baseURL;
  dbUrl = config.dbUrl;
  port = config.port;
  await refreshDevData(dbUrl);
  serverHandle = await startTestServer(port, dbUrl, config.oscPort);
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
});

async function expectMonitorToReport(page: Page, expected: { connected: number; issues: number }) {
  await page.waitForFunction(
    (minimumConnected) => {
      const el = document.querySelector('[data-role="stage-monitor"]');
      if (!el) return false;
      const connected = Number(el.getAttribute('data-connected') || '0');
      return connected >= minimumConnected;
    },
    expected.connected,
    { timeout: 30_000 },
  );

  const counts = await page.evaluate(() => window.__presenterOperatorTestHelpers?.stageMonitorCounts?.());
  expect(counts).toBeTruthy();
  expect(counts.connected).toBeGreaterThanOrEqual(expected.connected);
  expect(counts.issues).toBeGreaterThanOrEqual(expected.issues);

  const snapshot = await page.evaluate(() => fetch('/stage/connections').then((r) => r.json()));
  expect(Array.isArray(snapshot)).toBeTruthy();
  expect(snapshot.length).toBeGreaterThanOrEqual(expected.connected);
}

test('stage monitor updates when display connects', async ({ page, context }) => {
  const stagePage = await openStageDisplay(context);
  await waitForOperatorReady(page);
  await expectMonitorToReport(page, { connected: 1, issues: 0 });
  await stagePage.close();
  await page.waitForFunction(() => {
    const counts = window.__presenterOperatorTestHelpers?.stageMonitorCounts?.();
    return counts && counts.connected === 0 && counts.issues >= 1;
  });
  const afterClose = await page.evaluate(() => window.__presenterOperatorTestHelpers?.stageMonitorCounts?.());
  expect(afterClose).toBeTruthy();
  expect(afterClose.connected).toBe(0);
  expect(afterClose.issues).toBe(1);

  await page.waitForTimeout(2000);
  const afterDelay = await page.evaluate(() => window.__presenterOperatorTestHelpers?.stageMonitorCounts?.());
  expect(afterDelay).toBeTruthy();
  expect(afterDelay.connected).toBe(0);
  expect(afterDelay.issues).toBeGreaterThanOrEqual(1);

  await page.reload({ waitUntil: 'domcontentloaded' });
  await page.waitForLoadState('networkidle');
  await page.waitForFunction(() => window.__presenterLiveConnected === true, { timeout: 30_000 });
  await page.waitForFunction(() => {
    const counts = window.__presenterOperatorTestHelpers?.stageMonitorCounts?.();
    return counts && counts.connected === 0 && counts.issues >= 1;
  });
});

test('stage monitor handles legacy client identifiers', async ({ page, context }) => {
  const stagePage = await openStageDisplay(context, { forceLegacy: true });
  await waitForOperatorReady(page);
  await expectMonitorToReport(page, { connected: 1, issues: 0 });
  await stagePage.close();
});
