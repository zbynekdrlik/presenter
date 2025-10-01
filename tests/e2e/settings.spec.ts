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

const selectors = {
  form: '[data-role="host-form"]',
  labelInput: '[data-role="host-label"]',
  hostInput: '[data-role="host-host"]',
  portInput: '[data-role="host-port"]',
  enabledCheckbox: '[data-role="host-enabled"]',
  submitButton: '[data-role="host-submit"]',
  resetButton: '[data-role="host-reset"]',
  toast: '[data-role="toast"]',
  list: '[data-role="resolume-host-list"]',
};

test.describe.configure({ timeout: 600_000 });

test.beforeAll(async ({}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  baseURL = config.baseURL;
  await refreshDevData(config.dbUrl);
  serverHandle = await startTestServer(config.port, config.dbUrl);
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
});

async function waitForToast(page, expected: string) {
  const toast = page.locator(selectors.toast);
  await expect(toast).toHaveAttribute('data-visible', 'true', { timeout: 20_000 });
  await expect(toast).toHaveText(expected);
  await expect(toast).toHaveAttribute('data-visible', 'false');
}

async function getHostsViaApi(page) {
  const response = await page.request.get(new URL('/integrations/resolume/hosts', baseURL).toString(), {
    timeout: 60_000,
  });
  expect(response.ok()).toBeTruthy();
  return (await response.json()) as Array<{
    id: string;
    label: string;
    host: string;
    status: { state: string };
  }>;
}

test('resolume settings CRUD with status feedback', async ({ page }) => {
  const testLabel = `Resolume Arena ${Date.now()}`;
  await page.goto(new URL('/ui/settings', baseURL).toString());
  await page.waitForLoadState('networkidle');

  // Ensure the list starts empty.
  const emptyState = page.locator('[data-role="host-empty"]');
  await expect(emptyState).toHaveText('No Resolume connections defined yet.');

  // Create a new connection.
  await page.fill(selectors.labelInput, testLabel);
  await page.fill(selectors.hostInput, 'settings-test.invalid');
  await page.fill(selectors.portInput, '8090');
  await page.check(selectors.enabledCheckbox);
  await page.click(selectors.submitButton);
  await waitForToast(page, 'Added Resolume connection.');

  const listItem = page.locator(`[data-role="resolume-host-list"] li[data-id]`).first();
  await expect(listItem).toContainText(testLabel);
  await expect(listItem).toContainText('settings-test.invalid');

  // Status should transition from connecting to error because Resolume is offline in tests.
  await expect.poll(async () => {
    const hosts = await getHostsViaApi(page);
    return hosts[0]?.status.state;
  }, { timeout: 30_000 }).toEqual('error');
  await expect(listItem.locator('.settings__list-meta--warning')).toContainText('⚠', {
    timeout: 30_000,
  });

  const hostsAfterCreate = await getHostsViaApi(page);
  expect(hostsAfterCreate).toHaveLength(1);
  expect(hostsAfterCreate[0].label).toBe(testLabel);
  const hostId = hostsAfterCreate[0].id;
  const hostRow = page.locator(`[data-role="resolume-host-list"] li[data-id="${hostId}"]`);

  // Edit the connection.
  await page.locator(`[data-role="host-edit"][data-id="${hostId}"]`).click();
  const updatedLabel = `${testLabel} Updated`;
  await page.fill(selectors.labelInput, updatedLabel);
  await page.fill(selectors.hostInput, 'settings-test-updated.invalid');
  await page.click(selectors.submitButton);
  await waitForToast(page, 'Updated Resolume connection.');
  await expect(hostRow).toContainText(updatedLabel);

  const hostsAfterUpdate = await getHostsViaApi(page);
  expect(hostsAfterUpdate[0].label).toBe(updatedLabel);
  expect(hostsAfterUpdate[0].host).toBe('settings-test-updated.invalid');

  // Delete the connection.
  page.once('dialog', (dialog) => dialog.accept());
  await page.locator(`[data-role="host-delete"][data-id="${hostId}"]`).click();
  await waitForToast(page, 'Deleted Resolume connection.');
  await expect(page.locator(selectors.list)).toContainText('No Resolume connections defined yet.');

  const hostsAfterDelete = await getHostsViaApi(page);
  expect(hostsAfterDelete).toHaveLength(0);
});
