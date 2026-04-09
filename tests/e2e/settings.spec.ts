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
  androidForm: '[data-role="android-form"]',
  androidLabel: '[data-role="android-label"]',
  androidHost: '[data-role="android-host"]',
  androidPort: '[data-role="android-port"]',
  androidComponent: '[data-role="android-component"]',
  androidEnabled: '[data-role="android-enabled"]',
  androidSubmit: '[data-role="android-submit"]',
  androidReset: '[data-role="android-reset"]',
  androidList: '[data-role="android-display-list"]',
};

test.describe.configure({ timeout: 180_000 });

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

async function waitForToast(page, expected: string | RegExp) {
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
    status: { state: string; consecutiveFailures?: number };
  }>;
}

async function getAndroidDisplaysViaApi(page) {
  const response = await page.request.get(
    new URL('/integrations/android-stage/displays', baseURL).toString(),
    { timeout: 60_000 }
  );
  expect(response.ok()).toBeTruthy();
  return (await response.json()) as Array<{
    id: string;
    label: string;
    host: string;
    port: number;
    launchComponent: string;
    launch_component?: string;
    isEnabled?: boolean;
    is_enabled?: boolean;
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

  await expect(page.locator('[data-role="osc-port"]')).toHaveCount(1);
  await expect(page.locator('[data-role="ableset-form"] [data-role="osc-port"]').first()).toHaveValue(/\d+/);

  const abletonToggle = page.locator('[data-role="ableset-enabled"]');
  await abletonToggle.check();
  await page.click('[data-role="ableset-submit"]');
  await waitForToast(page, 'Ableton settings saved.');
  await expect(page.locator('[data-role="ableset-status-indicator"]').first()).toHaveAttribute('data-state', /(enabled|tracking)/);

  // Ensure the list starts empty.
  await expect(page.locator('[data-role="resolume-host-list"]').first()).toContainText('No Resolume connections defined yet.');

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

test('resolume connection diagnostics and test button', async ({ page }) => {
  const consoleMessages: string[] = [];
  page.on('console', (msg) => {
    if (msg.type() === 'error' || msg.type() === 'warning') {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  if (!mockResolume) {
    throw new Error('Mock Resolume server not started');
  }

  const mockHost = '127.0.0.1';
  const mockPort = String(mockResolume.port);

  await page.goto(new URL('/ui/settings', baseURL).toString());
  await page.waitForLoadState('networkidle');

  // Create a connection pointing at mock Resolume
  const testLabel = `Diag Test ${Date.now()}`;
  await page.fill(selectors.labelInput, testLabel);
  await page.fill(selectors.hostInput, mockHost);
  await page.fill(selectors.portInput, mockPort);
  await page.check(selectors.enabledCheckbox);
  await page.click(selectors.submitButton);
  await waitForToast(page, 'Added Resolume connection.');

  // Wait for connected state
  await expect.poll(async () => {
    const hosts = await getHostsViaApi(page);
    return hosts[0]?.status.state;
  }, { timeout: 30_000 }).toEqual('connected');

  // Test Connection button should work
  const hostsAfter = await getHostsViaApi(page);
  const hostId = hostsAfter[0].id;

  // Reload to get the Test button rendered
  await page.reload();
  await page.waitForLoadState('networkidle');

  // Click test button
  const testBtn = page.locator(`[data-role="host-test"][data-id="${hostId}"]`);
  await expect(testBtn).toBeVisible({ timeout: 10_000 });
  await testBtn.click();
  await waitForToast(page, /Connection OK/);

  // Take connection offline — verify diagnostics appear
  mockResolume.setOnline(false);

  // Wait for error state with consecutive failures
  await expect.poll(async () => {
    const hosts = await getHostsViaApi(page);
    const status = hosts[0]?.status;
    return status?.state === 'error' && (status?.consecutiveFailures ?? 0) > 0;
  }, { timeout: 30_000 }).toBeTruthy();

  // Verify the error detail is displayed in UI
  await page.reload();
  await page.waitForLoadState('networkidle');
  const errorDetail = page.locator('[data-role="host-error-detail"]');
  await expect(errorDetail).toContainText('Retrying', { timeout: 20_000 });
  await expect(errorDetail).toContainText('failure', { timeout: 5_000 });

  // Test connection while offline should fail
  const testBtnAfterReload = page.locator(`[data-role="host-test"][data-id="${hostId}"]`);
  await testBtnAfterReload.click();
  await waitForToast(page, /Connection failed/);

  // Bring back online — should recover
  mockResolume.setOnline(true);
  await expect.poll(async () => {
    const hosts = await getHostsViaApi(page);
    return hosts[0]?.status.state;
  }, { timeout: 30_000 }).toEqual('connected');

  // Verify diagnostics reset
  await expect.poll(async () => {
    const hosts = await getHostsViaApi(page);
    return hosts[0]?.status.consecutiveFailures;
  }, { timeout: 10_000 }).toEqual(0);

  // Clean up — delete the host
  page.once('dialog', (dialog) => dialog.accept());
  await page.locator(`[data-role="host-delete"][data-id="${hostId}"]`).click();
  await waitForToast(page, 'Deleted Resolume connection.');

  // Clean console check
  expect(consoleMessages).toEqual([]);
});

test('android stage launchers CRUD', async ({ page }) => {
  await page.goto(new URL('/ui/settings', baseURL).toString());
  await page.waitForLoadState('networkidle');

  await expect(page.locator(selectors.androidList).first()).toContainText(
    'No Android stage displays configured yet.'
  );

  const label = `Stage Display ${Date.now()}`;
  await page.fill(selectors.androidLabel, label);
  await page.fill(selectors.androidHost, 'sd1l.lan');
  await page.fill(selectors.androidPort, '5555');
  await page.fill(
    selectors.androidComponent,
    'com.fullykiosk.videokiosk/de.ozerov.fully.MainActivity'
  );
  await page.check(selectors.androidEnabled);
  await page.click(selectors.androidSubmit);
  await waitForToast(page, 'Added Android stage display.');

  const displaysAfterCreate = await getAndroidDisplaysViaApi(page);
  expect(displaysAfterCreate).toHaveLength(1);
  const created = displaysAfterCreate[0];
  expect(created.label).toBe(label);
  expect(created.host).toBe('sd1l.lan');
  const androidListItem = page.locator(
    `${selectors.androidList} li[data-id="${created.id}"]`
  );
  await expect(androidListItem).toContainText(label);

  // Edit display details.
  await page.locator(`[data-role="android-edit"][data-id="${created.id}"]`).click();
  const updatedLabel = `${label} Updated`;
  await page.fill(selectors.androidLabel, updatedLabel);
  await page.fill(selectors.androidHost, 'sd2l.lan');
  await page.fill(selectors.androidPort, '5566');
  await page.fill(selectors.androidComponent, 'com.example/.Main');
  await page.uncheck(selectors.androidEnabled);
  await page.click(selectors.androidSubmit);
  await waitForToast(page, 'Saved Android stage display.');
  await expect(androidListItem).toContainText(updatedLabel);
  await expect(androidListItem).toContainText('sd2l.lan');

  const displaysAfterUpdate = await getAndroidDisplaysViaApi(page);
  expect(displaysAfterUpdate[0].label).toBe(updatedLabel);
  expect(displaysAfterUpdate[0].host).toBe('sd2l.lan');
  expect(displaysAfterUpdate[0].port).toBe(5566);
  expect(
    displaysAfterUpdate[0].launchComponent ?? displaysAfterUpdate[0].launch_component
  ).toBe('com.example/.Main');

  // Delete the display.
  page.once('dialog', (dialog) => dialog.accept());
  await page
    .locator(`[data-role="android-delete"][data-id="${created.id}"]`)
    .click();
  await waitForToast(page, 'Deleted Android stage display.');

  const displaysAfterDelete = await getAndroidDisplaysViaApi(page);
  expect(displaysAfterDelete).toHaveLength(0);
  await expect(page.locator('[data-role="android-display-list"] [data-role="android-empty"]').first()).toHaveText(
    'No Android stage displays configured yet.'
  );
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
