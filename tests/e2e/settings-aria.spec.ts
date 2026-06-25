import { test, expect, type Page } from '@playwright/test';
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from './support';

// #459 — settings form accessibility (aria-*).
//
// The settings page was rewritten from an <iframe> to native Leptos panels in
// #462/PR #466 and shipped with ZERO aria wiring. This spec guards the a11y
// contract that the native cards must keep:
//   1. genuinely-required inputs carry `aria-required="true"`,
//   2. each field's `aria-describedby` points at an element that actually
//      EXISTS (the form-status <p> is reachable from the field),
//   3. `aria-invalid` flips to "true" on a rejected submit and the described-by
//      status <p> carries the error message, and
//   4. the browser console stays clean (browser-console-zero-errors.md).

let serverHandle: ServerHandle | undefined;
let baseURL: string;

const selectors = {
  // Resolume card (data-role / form-status id).
  resolumeLabel: '[data-role="host-label"]',
  resolumeHost: '[data-role="host-host"]',
  resolumePort: '[data-role="host-port"]',
  resolumeEnabled: '[data-role="host-enabled"]',
  resolumeSubmit: '[data-role="host-submit"]',
  resolumeStatusId: 'resolume-form-status',
  // Ableton card.
  abletonHost: '[data-role="ableset-host"]',
  abletonHttpPort: '[data-role="ableset-http-port"]',
  abletonLibrary: '[data-role="ableset-library"]',
  abletonOscPort: '[data-role="osc-port"]',
  abletonStatusId: 'ableton-form-status',
  // Android card.
  androidLabel: '[data-role="android-label"]',
  androidHost: '[data-role="android-host"]',
  androidPort: '[data-role="android-port"]',
  androidComponent: '[data-role="android-component"]',
  androidStatusId: 'android-form-status',
  // Companion card.
  companionPort: '[data-role="feature-companion-port"]',
  companionStatusId: 'feature-companion-status',
  // Video sources card (no inline status — toast-driven, aria-required only).
  videoLabel: '[data-role="video-source-label"]',
  videoNdiName: '[data-role="video-source-ndi-name"]',
};

test.describe.configure({ timeout: 180_000 });

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

async function gotoSettings(page: Page): Promise<void> {
  await page.goto(new URL('/ui/settings', baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  await page.waitForLoadState('networkidle');
}

/** A field's `aria-describedby` must reference an element that EXISTS. */
async function assertDescribedByReachable(page: Page, fieldSelector: string) {
  const field = page.locator(fieldSelector);
  const describedBy = await field.getAttribute('aria-describedby');
  expect(describedBy, `${fieldSelector} must have aria-describedby`).toBeTruthy();
  // The referenced element must be present in the document.
  const target = page.locator(`#${describedBy}`);
  await expect(
    target,
    `aria-describedby="${describedBy}" on ${fieldSelector} must reference an existing element`,
  ).toHaveCount(1);
}

test('settings required inputs carry aria-required across all cards', async ({ page }) => {
  const consoleMessages: string[] = [];
  page.on('console', (msg) => {
    if (msg.type() === 'error' || msg.type() === 'warning') {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  await gotoSettings(page);

  const requiredFields = [
    // Resolume
    selectors.resolumeLabel,
    selectors.resolumeHost,
    selectors.resolumePort,
    // Ableton
    selectors.abletonHost,
    selectors.abletonHttpPort,
    selectors.abletonLibrary,
    selectors.abletonOscPort,
    // Android
    selectors.androidLabel,
    selectors.androidHost,
    selectors.androidPort,
    selectors.androidComponent,
    // Companion
    selectors.companionPort,
    // Video sources (toast-driven, aria-required only)
    selectors.videoLabel,
    selectors.videoNdiName,
  ];

  for (const sel of requiredFields) {
    await expect(
      page.locator(sel),
      `${sel} must carry aria-required="true"`,
    ).toHaveAttribute('aria-required', 'true');
  }

  // Every described-by reference on the form-status cards must resolve to a
  // real element (the status <p> is reachable from each field).
  for (const sel of [
    selectors.resolumeLabel,
    selectors.resolumeHost,
    selectors.resolumePort,
    selectors.abletonHost,
    selectors.abletonHttpPort,
    selectors.abletonLibrary,
    selectors.abletonOscPort,
    selectors.androidLabel,
    selectors.androidHost,
    selectors.androidPort,
    selectors.androidComponent,
    selectors.companionPort,
  ]) {
    await assertDescribedByReachable(page, sel);
  }

  expect(consoleMessages).toEqual([]);
});

test('aria-invalid flips on a rejected submit and the described-by status carries the error', async ({
  page,
}) => {
  const consoleMessages: string[] = [];
  page.on('console', (msg) => {
    if (msg.type() === 'error' || msg.type() === 'warning') {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  await gotoSettings(page);

  // Before any rejection, aria-invalid is "false" (idle state).
  const resolumeLabel = page.locator(selectors.resolumeLabel);
  await expect(resolumeLabel).toHaveAttribute('aria-invalid', 'false');

  // Trigger a rejected submit on the Resolume card: an out-of-range port
  // (99999) is rejected by the Rust guard (parse_port_in_range, #455). fill()
  // injects the value exactly as a paste would, so on_submit runs and sets the
  // form_state to "error".
  await page.fill(selectors.resolumeLabel, `Aria Test ${Date.now()}`);
  await page.fill(selectors.resolumeHost, 'resolume.invalid');
  await page.fill(selectors.resolumePort, '99999');
  await page.check(selectors.resolumeEnabled);
  await page.click(selectors.resolumeSubmit);

  // The status <p> referenced by aria-describedby now carries the error and is
  // in the error state.
  const status = page.locator(`#${selectors.resolumeStatusId}`);
  await expect(status).toHaveText('Port must be between 1 and 65535.');
  await expect(status).toHaveAttribute('data-state', 'error');

  // aria-invalid flips to "true" on the fields once the form is in error state.
  await expect(resolumeLabel).toHaveAttribute('aria-invalid', 'true');
  await expect(page.locator(selectors.resolumeHost)).toHaveAttribute('aria-invalid', 'true');
  await expect(page.locator(selectors.resolumePort)).toHaveAttribute('aria-invalid', 'true');

  // Android card: the same out-of-range port path also flips aria-invalid.
  await expect(page.locator(selectors.androidLabel)).toHaveAttribute('aria-invalid', 'false');
  await page.fill(selectors.androidLabel, `Aria Display ${Date.now()}`);
  await page.fill(selectors.androidHost, 'stage.invalid');
  await page.fill(selectors.androidPort, '99999');
  await page.fill(selectors.androidComponent, 'com.tcl.browser');
  await page.locator('[data-role="android-submit"]').click();

  const androidStatus = page.locator(`#${selectors.androidStatusId}`);
  await expect(androidStatus).toHaveText('Port must be between 1 and 65535.');
  await expect(androidStatus).toHaveAttribute('data-state', 'error');
  await expect(page.locator(selectors.androidLabel)).toHaveAttribute('aria-invalid', 'true');
  await expect(page.locator(selectors.androidHost)).toHaveAttribute('aria-invalid', 'true');
  await expect(page.locator(selectors.androidPort)).toHaveAttribute('aria-invalid', 'true');
  await expect(page.locator(selectors.androidComponent)).toHaveAttribute('aria-invalid', 'true');

  expect(consoleMessages).toEqual([]);
});
