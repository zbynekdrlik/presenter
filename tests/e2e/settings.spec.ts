import { test, expect } from '@playwright/test';
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  startMockResolume,
  stopServer,
  type MockResolumeHandle,
  type ServerHandle,
} from './support';

let serverHandle: ServerHandle | undefined;
let baseURL: string;
let mockResolume: MockResolumeHandle | undefined;

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
  companionToggle: '[data-role="feature-companion-toggle"]',
  companionStatus: '[data-role="feature-status"]',
  companionPort: '[data-role="feature-companion-port"]',
  companionSubmit: '[data-role="feature-submit"]',
};

test.describe.configure({ timeout: 600_000 });

test.beforeAll(async ({}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  baseURL = config.baseURL;
  await refreshDevData(config.dbUrl);
  serverHandle = await startTestServer(config.port, config.dbUrl, config.oscPort);
  mockResolume = await startMockResolume();
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
  if (mockResolume) {
    await mockResolume.close();
    mockResolume = undefined;
  }
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

async function getFeatureFlags() {
  const response = await fetch(new URL('/settings/features', baseURL).toString(), {
    headers: { Accept: 'application/json' },
  });
  expect(response.ok).toBeTruthy();
  return (await response.json()) as {
    companionEnabled?: boolean;
    companion_enabled?: boolean;
    companionPort?: number;
    companion_port?: number;
  };
}

test('resolume settings CRUD with status feedback', async ({ page }) => {
  const testLabel = `Resolume Arena ${Date.now()}`;
  await page.goto(new URL('/ui/settings', baseURL).toString());
  await page.waitForLoadState('networkidle');

  // Ensure the list starts empty.
  const emptyState = page.locator('[data-role="host-empty"]');
  await expect(emptyState).toHaveText('No Resolume connections defined yet.');

  if (!mockResolume) {
    throw new Error('Mock Resolume server not started');
  }
  const mockHost = '127.0.0.1';
  const mockPort = String(mockResolume.port);

  // Create a new connection.
  await page.fill(selectors.labelInput, testLabel);
  await page.fill(selectors.hostInput, mockHost);
  await page.fill(selectors.portInput, mockPort);
  await page.check(selectors.enabledCheckbox);
  await page.click(selectors.submitButton);
  await waitForToast(page, 'Added Resolume connection.');

  const listItem = page.locator(`[data-role="resolume-host-list"] li[data-id]`).first();
  await expect(listItem).toContainText(testLabel);
  await expect(listItem).toContainText(mockHost);

  // Wait for the mock Resolume to report as connected.
  await expect.poll(async () => {
    const hosts = await getHostsViaApi(page);
    return hosts[0]?.status.state;
  }, { timeout: 30_000 }).toEqual('connected');

  // Simulate a disconnect and verify the UI reflects it.
  mockResolume.setOnline(false);
  await expect.poll(async () => {
    const hosts = await getHostsViaApi(page);
    return hosts[0]?.status.state;
  }, { timeout: 30_000 }).toEqual('error');
  await expect(listItem.locator('.settings__list-meta--warning')).toContainText('⚠', {
    timeout: 30_000,
  });
  mockResolume.setOnline(true);

  const hostsAfterCreate = await getHostsViaApi(page);
  expect(hostsAfterCreate).toHaveLength(1);
  expect(hostsAfterCreate[0].label).toBe(testLabel);
  expect(hostsAfterCreate[0].host).toBe(mockHost);
  const hostId = hostsAfterCreate[0].id;
  const hostRow = page.locator(`[data-role="resolume-host-list"] li[data-id="${hostId}"]`);

  // Edit the connection.
  await page.locator(`[data-role="host-edit"][data-id="${hostId}"]`).click();
  const updatedLabel = `${testLabel} Updated`;
  await page.fill(selectors.labelInput, updatedLabel);
  await page.fill(selectors.hostInput, mockHost);
  await page.fill(selectors.portInput, mockPort);
  await page.click(selectors.submitButton);
  await waitForToast(page, 'Updated Resolume connection.');
  await expect(hostRow).toContainText(updatedLabel);

  const hostsAfterUpdate = await getHostsViaApi(page);
  expect(hostsAfterUpdate[0].label).toBe(updatedLabel);
  expect(hostsAfterUpdate[0].host).toBe(mockHost);

  // Delete the connection.
  page.once('dialog', (dialog) => dialog.accept());
  await page.locator(`[data-role="host-delete"][data-id="${hostId}"]`).click();
  await waitForToast(page, 'Deleted Resolume connection.');
  await expect(page.locator(selectors.list)).toContainText('No Resolume connections defined yet.');

  const hostsAfterDelete = await getHostsViaApi(page);
  expect(hostsAfterDelete).toHaveLength(0);
});

test('companion settings reflect feature flags', async ({ page }) => {
  await page.goto(new URL('/ui/settings', baseURL).toString());
  await page.waitForLoadState('networkidle');

  const toggle = page.locator(selectors.companionToggle);
  const portInput = page.locator(selectors.companionPort);

  const initialFeatures = await getFeatureFlags();
  const initiallyEnabled = Boolean(
    initialFeatures.companionEnabled ?? initialFeatures.companion_enabled
  );
  const initialPort =
    initialFeatures.companionPort ?? initialFeatures.companion_port ?? 18175;

  const randomPort = () => 20000 + Math.floor(Math.random() * 10000);
  let desiredPort = randomPort();
  if (desiredPort === initialPort) {
    desiredPort += 1;
  }

  const updateFeatures = async (enabled: boolean, port: number) => {
    const response = await page.request.post(new URL('/settings/features', baseURL).toString(), {
      data: {
        companionEnabled: enabled,
        companionPort: port,
      },
    });
    if (!response.ok()) {
      const body = await response.text();
      throw new Error(`Failed to update features (${response.status()}): ${body}`);
    }
  };

  await updateFeatures(true, desiredPort);
  await page.reload();
  await expect(toggle).toBeChecked();
  await expect(portInput).toHaveValue(String(desiredPort));

  await updateFeatures(false, desiredPort);
  await page.reload();
  await expect(toggle).not.toBeChecked();

  await updateFeatures(initiallyEnabled, initialPort);
  await page.reload();
  await expect(toggle).toHaveJSProperty('checked', initiallyEnabled);
  await expect(portInput).toHaveValue(String(initialPort));
});
