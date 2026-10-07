import { test, expect, type Page } from '@playwright/test';
import {
  attachConsoleErrorCollector,
  deriveTestConfig,
  refreshDevData,
  startMockResolume,
  startTestServer,
  stopServer,
  type MockResolumeHandle,
  type ServerHandle,
} from './support';

// #819 — operator Settings tab: compact fields + inline row editing.
//
// Before: a second, global `.settings__form-row { flex-direction: column }` turned
// every label's 220 px flex-basis into a 220 px HEIGHT (Companion card ~720 px for
// two fields), and a Resolume / Android row's Edit only loaded ONE shared form at
// the top of the card — measured 1170 px above the clicked row. Rows were also
// keyed on their live status, so every 5 s poll that changed a status rebuilt them.
//
// These tests drive the operator's embedded Settings panel (`/ui/operator/settings`)
// in a real browser: Edit opens IN the clicked row, Cancel restores, Save persists
// (API read-back), "+ Add …" opens the same editor at the top of the list, a status
// poll neither rebuilds a row nor resets a typed draft, and no form row is taller
// than its content.

let serverHandle: ServerHandle | undefined;
let baseURL: string;
let mockResolume: MockResolumeHandle | undefined;

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

type ResolumeHost = {
  id: string;
  label: string;
  host: string;
  port: number;
  isEnabled: boolean;
  status?: { state?: string };
};

type AndroidDisplay = {
  id: string;
  label: string;
  host: string;
  port: number;
  launchComponent: string;
  isEnabled: boolean;
};

const RESOLUME_HOSTS = '/integrations/resolume/hosts';
const ANDROID_DISPLAYS = '/integrations/android-stage/displays';

function url(path: string): string {
  return new URL(path, baseURL).toString();
}

async function createVia<T>(page: Page, path: string, data: Record<string, unknown>): Promise<T> {
  const response = await page.request.post(url(path), { data });
  expect(response.ok(), `POST ${path} → ${response.status()}`).toBeTruthy();
  return (await response.json()) as T;
}

async function listVia<T>(page: Page, path: string): Promise<T[]> {
  const response = await page.request.get(url(path));
  expect(response.ok(), `GET ${path} → ${response.status()}`).toBeTruthy();
  return (await response.json()) as T[];
}

async function deleteVia(page: Page, path: string): Promise<void> {
  await page.request.delete(url(path));
}

async function resolumeHost(page: Page, id: string): Promise<ResolumeHost> {
  const host = (await listVia<ResolumeHost>(page, RESOLUME_HOSTS)).find((h) => h.id === id);
  expect(host, `Resolume host ${id} exists`).toBeDefined();
  return host!;
}

async function androidDisplay(page: Page, id: string): Promise<AndroidDisplay> {
  const display = (await listVia<AndroidDisplay>(page, ANDROID_DISPLAYS)).find((d) => d.id === id);
  expect(display, `Android display ${id} exists`).toBeDefined();
  return display!;
}

async function gotoOperatorSettings(page: Page): Promise<void> {
  await page.goto(url('/ui/operator/settings'));
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  await expect(page.locator('[data-view-panel="settings"] .settings__card').first()).toBeVisible();
}

/** The embedded settings panel's toast (`settings-toast`, #462). */
async function waitForToast(page: Page, expected: string | RegExp): Promise<void> {
  const toast = page.locator('[data-role="settings-toast"]');
  await expect(toast).toHaveAttribute('data-visible', 'true', { timeout: 20_000 });
  await expect(toast).toHaveText(expected);
  await expect(toast).toHaveAttribute('data-visible', 'false');
}

/**
 * Wait for the Resolume card's own 5 s status poll to come back — optionally one whose
 * body matches `accept`. Deterministic stand-in for "wait longer than a poll".
 */
async function waitForHostsPoll(
  page: Page,
  accept: (hosts: ResolumeHost[]) => boolean = () => true,
): Promise<void> {
  await page.waitForResponse(
    async (response) => {
      if (response.request().method() !== 'GET') return false;
      if (new URL(response.url()).pathname !== RESOLUME_HOSTS) return false;
      try {
        return accept((await response.json()) as ResolumeHost[]);
      } catch {
        return false;
      }
    },
    { timeout: 20_000 },
  );
}

function resolumeRow(page: Page, id: string) {
  return page.locator(`[data-role="resolume-host-list"] li[data-id="${id}"]`);
}

function androidRow(page: Page, id: string) {
  return page.locator(`[data-role="android-display-list"] li[data-id="${id}"]`);
}

test('#819 Resolume: Edit opens in the clicked row, Cancel restores, Save persists', async ({
  page,
}) => {
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  const stamp = Date.now();
  const a = await createVia<ResolumeHost>(page, RESOLUME_HOSTS, {
    label: `InlineA${stamp}`,
    host: 'arena-a.invalid',
    port: 8090,
    isEnabled: false,
  });
  const b = await createVia<ResolumeHost>(page, RESOLUME_HOSTS, {
    label: `InlineB${stamp}`,
    host: 'arena-b.invalid',
    port: 8091,
    isEnabled: false,
  });

  try {
    await gotoOperatorSettings(page);
    const rowA = resolumeRow(page, a.id);
    const rowB = resolumeRow(page, b.id);
    await expect(rowB.locator('.settings__host-label')).toHaveText(b.label);
    await expect(page.locator('[data-role="host-editor"]')).toHaveCount(0);

    // Edit opens IN the clicked row, loaded with that row's values and focused.
    const editB = page.locator(`[data-role="host-edit"][data-id="${b.id}"]`);
    await editB.scrollIntoViewIfNeeded();
    const clickedAt = await editB.boundingBox();
    await editB.click();
    const editorB = rowB.locator('[data-role="host-editor"]');
    await expect(editorB).toBeVisible();
    await expect(page.locator('[data-role="host-editor"]')).toHaveCount(1);
    await expect(rowB).toHaveAttribute('data-editing', 'true');
    await expect(rowA).toHaveAttribute('data-editing', 'false');
    await expect(editorB.locator('[data-role="host-label"]')).toHaveValue(b.label);
    await expect(editorB.locator('[data-role="host-host"]')).toHaveValue('arena-b.invalid');
    await expect(editorB.locator('[data-role="host-port"]')).toHaveValue('8091');
    await expect(editorB.locator('[data-role="host-enabled"]')).not.toBeChecked();
    await expect(editorB.locator('[data-role="host-label"]')).toBeFocused();
    // The editor is where the operator clicked — not a screen away (the 1170 px jump).
    await expect(editorB).toBeInViewport();
    const editorAt = await editorB.boundingBox();
    expect(clickedAt).not.toBeNull();
    expect(editorAt).not.toBeNull();
    expect(Math.abs(editorAt!.y - clickedAt!.y)).toBeLessThan(150);

    // One editor per card: opening row A closes row B's.
    await page.locator(`[data-role="host-edit"][data-id="${a.id}"]`).click();
    await expect(rowA.locator('[data-role="host-editor"]')).toBeVisible();
    await expect(rowB.locator('[data-role="host-editor"]')).toHaveCount(0);
    await expect(page.locator('[data-role="host-editor"]')).toHaveCount(1);

    // Cancel restores: the typed change is dropped, row and server keep the old label.
    await page.fill('[data-role="host-label"]', `${a.label}Discarded`);
    await page.click('[data-role="host-cancel"]');
    await expect(page.locator('[data-role="host-editor"]')).toHaveCount(0);
    await expect(rowA.locator('.settings__host-label')).toHaveText(a.label);
    expect((await resolumeHost(page, a.id)).label).toBe(a.label);
    // Re-opening shows the stored value, not the discarded draft; Escape cancels too.
    await page.locator(`[data-role="host-edit"][data-id="${a.id}"]`).click();
    await expect(page.locator('[data-role="host-label"]')).toHaveValue(a.label);
    await page.press('[data-role="host-label"]', 'Escape');
    await expect(page.locator('[data-role="host-editor"]')).toHaveCount(0);

    // Save persists — read back through the API, and the row shows it.
    await page.locator(`[data-role="host-edit"][data-id="${a.id}"]`).click();
    const savedLabel = `${a.label}Saved`;
    await page.fill('[data-role="host-label"]', savedLabel);
    await page.fill('[data-role="host-port"]', '8095');
    await page.click('[data-role="host-submit"]');
    await waitForToast(page, 'Updated Resolume connection.');
    await expect(page.locator('[data-role="host-editor"]')).toHaveCount(0);
    await expect(rowA.locator('.settings__host-label')).toHaveText(savedLabel);
    await expect(rowA.locator('.settings__host-addr')).toHaveText(/arena-a\.invalid\s*:8095/);
    const saved = await resolumeHost(page, a.id);
    expect(saved.label).toBe(savedLabel);
    expect(saved.host).toBe('arena-a.invalid');
    expect(saved.port).toBe(8095);
    expect(saved.isEnabled).toBe(false);
    // Row B was never touched.
    expect((await resolumeHost(page, b.id)).label).toBe(b.label);
  } finally {
    await deleteVia(page, `${RESOLUME_HOSTS}/${a.id}`);
    await deleteVia(page, `${RESOLUME_HOSTS}/${b.id}`);
  }

  expect(errors).toEqual([]);
});

test('#819 Resolume: a status poll updates the row in place and never resets an open editor', async ({
  page,
}) => {
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  if (!mockResolume) throw new Error('Mock Resolume server not started');
  const mock = mockResolume;
  mock.setOnline(true);
  const host = await createVia<ResolumeHost>(page, RESOLUME_HOSTS, {
    label: `PollEdit${Date.now()}`,
    host: '127.0.0.1',
    port: mock.port,
    isEnabled: true,
  });
  const apiState = async () => (await resolumeHost(page, host.id)).status?.state;

  try {
    await expect.poll(apiState, { timeout: 30_000 }).toBe('connected');
    await gotoOperatorSettings(page);
    const row = resolumeRow(page, host.id);
    const badge = row.locator('[data-role="host-status"]');
    await expect(badge).toHaveAttribute('data-state', 'connected');

    // Tag the row's DOM node: a row the poll destroyed and rebuilt loses the tag.
    await row.evaluate((el) => {
      (el as unknown as { inlineEditTag?: string }).inlineEditTag = 'kept';
    });
    const tag = () =>
      row.evaluate((el) => (el as unknown as { inlineEditTag?: string }).inlineEditTag);

    // 1) A status change updates the badge IN PLACE — same row element.
    mock.setOnline(false);
    await expect.poll(apiState, { timeout: 30_000 }).toBe('error');
    await expect(badge).toHaveAttribute('data-state', 'error', { timeout: 20_000 });
    await expect(row.locator('[data-role="host-error-detail"]')).toContainText('Retrying');
    expect(await tag()).toBe('kept');

    // 2) A status change while the row is being edited keeps the editor and the draft.
    await page.locator(`[data-role="host-edit"][data-id="${host.id}"]`).click();
    const label = row.locator('[data-role="host-label"]');
    const typed = `${host.label}Typing`;
    await label.fill(typed);
    mock.setOnline(true);
    await expect.poll(apiState, { timeout: 30_000 }).toBe('connected');
    // The card's own poll delivers the new status, then one more poll: by then the
    // first one has certainly been applied to the page.
    await waitForHostsPoll(page, (hosts) =>
      hosts.some((h) => h.id === host.id && h.status?.state === 'connected'),
    );
    await waitForHostsPoll(page);
    await expect(row.locator('[data-role="host-editor"]')).toBeVisible();
    await expect(label).toHaveValue(typed);
    expect(await tag()).toBe('kept');

    // Cancel: the summary comes back with the live state; nothing was saved.
    await page.click('[data-role="host-cancel"]');
    await expect(badge).toHaveAttribute('data-state', 'connected');
    expect((await resolumeHost(page, host.id)).label).toBe(host.label);
  } finally {
    mock.setOnline(true);
    await deleteVia(page, `${RESOLUME_HOSTS}/${host.id}`);
  }

  expect(errors).toEqual([]);
});

test('#819 "+ Add connection" opens the editor at the top of the list; the draft survives polls', async ({
  page,
}) => {
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  const existing = await createVia<ResolumeHost>(page, RESOLUME_HOSTS, {
    label: `AddExisting${Date.now()}`,
    host: 'arena-existing.invalid',
    port: 8090,
    isEnabled: false,
  });
  const label = `AddNew${Date.now()}`;
  let createdId: string | undefined;

  try {
    await gotoOperatorSettings(page);
    await expect(resolumeRow(page, existing.id)).toBeVisible();
    await page.click('[data-role="host-add"]');
    const newItem = page.locator('[data-role="host-new-item"]');
    await expect(newItem.locator('[data-role="host-editor"]')).toBeVisible();
    // The new item is the FIRST entry of the list, above the existing rows.
    await expect(
      page.locator('[data-role="resolume-host-list"] > li').first(),
    ).toHaveAttribute('data-role', 'host-new-item');
    await expect(page.locator('[data-role="host-add"]')).toBeDisabled();
    // Defaults for a new connection.
    await expect(newItem.locator('[data-role="host-port"]')).toHaveValue('8090');
    await expect(newItem.locator('[data-role="host-enabled"]')).toBeChecked();
    await expect(newItem.locator('[data-role="host-label"]')).toBeFocused();

    // A poll while typing never resets the draft.
    await newItem.locator('[data-role="host-label"]').fill(label);
    await waitForHostsPoll(page);
    await waitForHostsPoll(page);
    await expect(newItem.locator('[data-role="host-label"]')).toHaveValue(label);

    await newItem.locator('[data-role="host-host"]').fill('arena-new.invalid');
    await newItem.locator('[data-role="host-port"]').fill('8097');
    await newItem.locator('[data-role="host-enabled"]').uncheck();
    await newItem.locator('[data-role="host-submit"]').click();
    await waitForToast(page, 'Added Resolume connection.');
    await expect(page.locator('[data-role="host-new-item"]')).toHaveCount(0);
    await expect(page.locator('[data-role="host-add"]')).toBeEnabled();

    const created = (await listVia<ResolumeHost>(page, RESOLUME_HOSTS)).find(
      (h) => h.label === label,
    );
    expect(created, 'the new connection was stored').toBeDefined();
    createdId = created!.id;
    expect(created!.host).toBe('arena-new.invalid');
    expect(created!.port).toBe(8097);
    expect(created!.isEnabled).toBe(false);
    await expect(resolumeRow(page, created!.id).locator('.settings__host-label')).toHaveText(label);
  } finally {
    await deleteVia(page, `${RESOLUME_HOSTS}/${existing.id}`);
    if (createdId) await deleteVia(page, `${RESOLUME_HOSTS}/${createdId}`);
  }

  expect(errors).toEqual([]);
});

test('#819 Android: inline edit in the row (Cancel restores, Save persists) and "+ Add display"', async ({
  page,
}) => {
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  const stamp = Date.now();
  const display = await createVia<AndroidDisplay>(page, ANDROID_DISPLAYS, {
    label: `InlineTv${stamp}`,
    host: 'tv-inline.invalid',
    port: 5555,
    launchComponent: 'com.tcl.browser',
    isEnabled: false,
  });
  const addedLabel = `AddedTv${stamp}`;
  let addedId: string | undefined;

  try {
    await gotoOperatorSettings(page);
    const row = androidRow(page, display.id);
    await expect(row.locator('.settings__host-label')).toHaveText(display.label);

    // Edit opens IN the row with the display's values, including the launch package.
    await page.locator(`[data-role="android-edit"][data-id="${display.id}"]`).click();
    const editor = row.locator('[data-role="android-editor"]');
    await expect(editor).toBeVisible();
    await expect(page.locator('[data-role="android-editor"]')).toHaveCount(1);
    await expect(editor.locator('[data-role="android-label"]')).toHaveValue(display.label);
    await expect(editor.locator('[data-role="android-host"]')).toHaveValue('tv-inline.invalid');
    await expect(editor.locator('[data-role="android-port"]')).toHaveValue('5555');
    await expect(editor.locator('[data-role="android-component"]')).toHaveValue('com.tcl.browser');
    await expect(editor).toBeInViewport();

    // Cancel restores.
    await editor.locator('[data-role="android-label"]').fill(`${display.label}Discarded`);
    await editor.locator('[data-role="android-component"]').fill('com.discarded/.Main');
    await page.click('[data-role="android-cancel"]');
    await expect(page.locator('[data-role="android-editor"]')).toHaveCount(0);
    await expect(row.locator('.settings__host-label')).toHaveText(display.label);
    await expect(row.locator('[data-role="android-component-value"]')).toHaveText('com.tcl.browser');
    expect((await androidDisplay(page, display.id)).label).toBe(display.label);

    // Save persists — API read-back.
    await page.locator(`[data-role="android-edit"][data-id="${display.id}"]`).click();
    const savedLabel = `${display.label}Saved`;
    await page.fill('[data-role="android-label"]', savedLabel);
    await page.fill('[data-role="android-host"]', 'tv-saved.invalid');
    await page.fill('[data-role="android-port"]', '5566');
    await page.fill('[data-role="android-component"]', 'com.example/.Main');
    await page.click('[data-role="android-submit"]');
    await waitForToast(page, 'Saved Android stage display.');
    await expect(page.locator('[data-role="android-editor"]')).toHaveCount(0);
    await expect(row.locator('.settings__host-label')).toHaveText(savedLabel);
    await expect(row.locator('[data-role="android-component-value"]')).toHaveText('com.example/.Main');
    const saved = await androidDisplay(page, display.id);
    expect(saved.label).toBe(savedLabel);
    expect(saved.host).toBe('tv-saved.invalid');
    expect(saved.port).toBe(5566);
    expect(saved.launchComponent).toBe('com.example/.Main');
    expect(saved.isEnabled).toBe(false);

    // "+ Add display": the new item opens at the top; Cancel creates nothing.
    const before = (await listVia<AndroidDisplay>(page, ANDROID_DISPLAYS)).length;
    await page.click('[data-role="android-add"]');
    const newItem = page.locator('[data-role="android-new-item"]');
    await expect(newItem.locator('[data-role="android-editor"]')).toBeVisible();
    await expect(
      page.locator('[data-role="android-display-list"] > li').first(),
    ).toHaveAttribute('data-role', 'android-new-item');
    await expect(newItem.locator('[data-role="android-port"]')).toHaveValue('5555');
    await expect(newItem.locator('[data-role="android-component"]')).toHaveValue('com.tcl.browser');
    await newItem.locator('[data-role="android-label"]').fill(`${addedLabel}Cancelled`);
    await page.click('[data-role="android-cancel"]');
    await expect(page.locator('[data-role="android-new-item"]')).toHaveCount(0);
    expect(await listVia<AndroidDisplay>(page, ANDROID_DISPLAYS)).toHaveLength(before);

    // …and Save creates it.
    await page.click('[data-role="android-add"]');
    await expect(newItem.locator('[data-role="android-label"]')).toHaveValue('');
    await newItem.locator('[data-role="android-label"]').fill(addedLabel);
    await newItem.locator('[data-role="android-host"]').fill('tv-added.invalid');
    await newItem.locator('[data-role="android-enabled"]').uncheck();
    await newItem.locator('[data-role="android-submit"]').click();
    await waitForToast(page, 'Added Android stage display.');
    await expect(page.locator('[data-role="android-new-item"]')).toHaveCount(0);
    const added = (await listVia<AndroidDisplay>(page, ANDROID_DISPLAYS)).find(
      (d) => d.label === addedLabel,
    );
    expect(added, 'the new display was stored').toBeDefined();
    addedId = added!.id;
    expect(added!.host).toBe('tv-added.invalid');
    expect(added!.port).toBe(5555);
    expect(added!.launchComponent).toBe('com.tcl.browser');
    await expect(androidRow(page, added!.id).locator('.settings__host-label')).toHaveText(addedLabel);
  } finally {
    await deleteVia(page, `${ANDROID_DISPLAYS}/${display.id}`);
    if (addedId) await deleteVia(page, `${ANDROID_DISPLAYS}/${addedId}`);
  }

  expect(errors).toEqual([]);
});

test('#819 compact layout: no form row taller than its content, Companion card under 300 px', async ({
  page,
}) => {
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  // A row of our own (disabled → no live error line whose length we don't control).
  const display = await createVia<AndroidDisplay>(page, ANDROID_DISPLAYS, {
    label: `LayoutTv${Date.now()}`,
    host: 'tv-layout.invalid',
    port: 5555,
    launchComponent: 'com.tcl.browser',
    isEnabled: false,
  });
  await page.setViewportSize({ width: 1600, height: 1000 });
  await gotoOperatorSettings(page);
  const panel = page.locator('[data-view-panel="settings"]');

  // Was ~720 px for one checkbox and one port field.
  const companion = panel.locator('.settings__card', {
    has: page.locator('[data-role="feature-companion-form"]'),
  });
  const companionBox = await companion.boundingBox();
  expect(companionBox).not.toBeNull();
  expect(companionBox!.height).toBeLessThan(300);

  // Measure with both inline editors open, so their rows are covered too.
  await page.click('[data-role="host-add"]');
  await expect(page.locator('[data-role="host-editor"]')).toBeVisible();
  await page.click('[data-role="android-add"]');
  await expect(page.locator('[data-role="android-editor"]')).toBeVisible();

  const labels = await panel
    .locator('.settings__form-row label')
    .evaluateAll((els) =>
      els.map((el) => ({
        text: (el.textContent ?? '').trim().slice(0, 40),
        height: Math.round(el.getBoundingClientRect().height),
      })),
    );
  // Companion, Preferences, Ableton and both editors: well over a dozen fields.
  expect(labels.length).toBeGreaterThan(12);
  // A caption + an input is ~60 px; the bug made every one of them 220+ px.
  expect(labels.filter((l) => l.height > 90)).toEqual([]);

  // A list row is compact too: title, host:port + badge, two small muted lines
  // (was title + seven separate meta lines).
  try {
    const rowBox = await androidRow(page, display.id).boundingBox();
    expect(rowBox).not.toBeNull();
    expect(rowBox!.height).toBeLessThan(160);
  } finally {
    await deleteVia(page, `${ANDROID_DISPLAYS}/${display.id}`);
  }

  expect(errors).toEqual([]);
});
