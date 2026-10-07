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
//
// Round 2 (the editor state machine's guards): a poll answered after a save never
// reverts the row (`list_sync::ResponseOrder`), a row deleted elsewhere closes its
// editor with a toast and focus on "+ Add", a save in flight locks every trigger and
// ignores Escape / Cancel, and unsaved changes lock the other rows' Edit and "+ Add".

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

    // Cancel restores: the typed change is dropped, row and server keep the old label,
    // and focus goes back to the row's Edit button.
    const editA = page.locator(`[data-role="host-edit"][data-id="${a.id}"]`);
    await page.fill('[data-role="host-label"]', `${a.label}Discarded`);
    await page.click('[data-role="host-cancel"]');
    await expect(page.locator('[data-role="host-editor"]')).toHaveCount(0);
    await expect(rowA.locator('.settings__host-label')).toHaveText(a.label);
    await expect(editA).toBeFocused();
    expect((await resolumeHost(page, a.id)).label).toBe(a.label);
    // Re-opening shows the stored value, not the discarded draft; Escape cancels too.
    await editA.click();
    await expect(page.locator('[data-role="host-label"]')).toHaveValue(a.label);
    await page.press('[data-role="host-label"]', 'Escape');
    await expect(page.locator('[data-role="host-editor"]')).toHaveCount(0);
    await expect(editA).toBeFocused();

    // Save persists — read back through the API, and the row shows it.
    await page.locator(`[data-role="host-edit"][data-id="${a.id}"]`).click();
    const savedLabel = `${a.label}Saved`;
    await page.fill('[data-role="host-label"]', savedLabel);
    await page.fill('[data-role="host-port"]', '8095');
    await page.click('[data-role="host-submit"]');
    await waitForToast(page, 'Updated Resolume connection.');
    await expect(page.locator('[data-role="host-editor"]')).toHaveCount(0);
    await expect(editA).toBeFocused();
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

    // The launch package is part of the draft (round 2, item 7): changing only it is an
    // unsaved change, so "+ Add display" is locked until Save or Cancel.
    const androidAdd = page.locator('[data-role="android-add"]');
    await expect(androidAdd).toBeEnabled();
    await editor.locator('[data-role="android-component"]').fill('com.discarded/.Main');
    await expect(androidAdd).toBeDisabled();
    await expect(androidAdd).toHaveAttribute('data-lock', 'unsaved');

    // Cancel restores.
    await editor.locator('[data-role="android-label"]').fill(`${display.label}Discarded`);
    await page.click('[data-role="android-cancel"]');
    await expect(androidAdd).toBeEnabled();
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
    // Focus goes back to "+ Add display".
    await expect(page.locator('[data-role="android-add"]')).toBeFocused();
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
  try {
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

    // A list row is compact too: title, host:port + badge, two small muted lines
    // (was title + seven separate meta lines).
    const rowBox = await androidRow(page, display.id).boundingBox();
    expect(rowBox).not.toBeNull();
    expect(rowBox!.height).toBeLessThan(160);

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
  } finally {
    await deleteVia(page, `${ANDROID_DISPLAYS}/${display.id}`);
  }

  expect(errors).toEqual([]);
});

// ── #819 round 2: the editor state machine's guards ─────────────────────────────────

/** A promise plus its resolver, to hold a routed request until the test releases it. */
function deferred(): { promise: Promise<void>; resolve: () => void } {
  let resolve!: () => void;
  const promise = new Promise<void>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

async function twoHosts(page: Page, prefix: string): Promise<[ResolumeHost, ResolumeHost]> {
  const stamp = Date.now();
  const a = await createVia<ResolumeHost>(page, RESOLUME_HOSTS, {
    label: `${prefix}A${stamp}`,
    host: 'arena-a.invalid',
    port: 8090,
    isEnabled: false,
  });
  const b = await createVia<ResolumeHost>(page, RESOLUME_HOSTS, {
    label: `${prefix}B${stamp}`,
    host: 'arena-b.invalid',
    port: 8091,
    isEnabled: false,
  });
  return [a, b];
}

test('#819 a status poll sent before a save but answered after it never reverts the saved row', async ({
  page,
}) => {
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  const host = await createVia<ResolumeHost>(page, RESOLUME_HOSTS, {
    label: `StalePoll${Date.now()}`,
    host: 'arena-stale.invalid',
    port: 8090,
    isEnabled: false,
  });
  const savedLabel = `${host.label}Saved`;

  try {
    await gotoOperatorSettings(page);
    const row = resolumeRow(page, host.id);
    await expect(row.locator('.settings__host-label')).toHaveText(host.label);
    await page.locator(`[data-role="host-edit"][data-id="${host.id}"]`).click();
    await row.locator('[data-role="host-label"]').fill(savedLabel);
    await row.locator('[data-role="host-port"]').fill('8096');

    // Hold the card's next status poll: take its answer from the server NOW (the
    // pre-save list) and deliver it only after the save and the save's own reload
    // are done. That is the "slow poll lands last" order `list_sync::ResponseOrder`
    // must drop.
    const pollHeld = deferred();
    const release = deferred();
    const staleDelivered = deferred();
    let holding = false;
    await page.route(
      (u) => u.pathname === RESOLUME_HOSTS,
      async (route) => {
        if (holding || route.request().method() !== 'GET') {
          await route.fallback();
          return;
        }
        holding = true;
        const stale = await route.fetch();
        pollHeld.resolve();
        await release.promise;
        await route.fulfill({ response: stale });
        staleDelivered.resolve();
      },
    );
    await pollHeld.promise;

    await row.locator('[data-role="host-submit"]').click();
    // The success toast is shown after the save's reload was applied.
    await waitForToast(page, 'Updated Resolume connection.');
    await expect(page.locator('[data-role="host-editor"]')).toHaveCount(0);
    await expect(row.locator('.settings__host-label')).toHaveText(savedLabel);

    // Record every DOM change of the list: an applied stale answer would put the old
    // label back, even if only until the next poll.
    await page.evaluate(
      ({ id, staleLabel }) => {
        const flags = window as unknown as { staleLabelSeen?: boolean };
        flags.staleLabelSeen = false;
        const list = document.querySelector('[data-role="resolume-host-list"]');
        if (!list) throw new Error('Resolume host list not found');
        new MutationObserver(() => {
          const label = list.querySelector(`li[data-id="${id}"] .settings__host-label`);
          if (label?.textContent === staleLabel) flags.staleLabelSeen = true;
        }).observe(list, { childList: true, subtree: true, characterData: true });
      },
      { id: host.id, staleLabel: host.label },
    );
    release.resolve();
    await staleDelivered.promise;
    // Two more polls: by the second one the stale answer has certainly been handled.
    await waitForHostsPoll(page);
    await waitForHostsPoll(page);

    expect(
      await page.evaluate(() => (window as unknown as { staleLabelSeen?: boolean }).staleLabelSeen),
    ).toBe(false);
    await expect(row.locator('.settings__host-label')).toHaveText(savedLabel);
    await expect(row.locator('.settings__host-addr')).toHaveText(/arena-stale\.invalid\s*:8096/);
    const saved = await resolumeHost(page, host.id);
    expect(saved.label).toBe(savedLabel);
    expect(saved.port).toBe(8096);
  } finally {
    await deleteVia(page, `${RESOLUME_HOSTS}/${host.id}`);
  }

  expect(errors).toEqual([]);
});

test('#819 a row deleted elsewhere while edited: the editor closes, a toast says why, focus goes to "+ Add"', async ({
  page,
}) => {
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  const host = await createVia<ResolumeHost>(page, RESOLUME_HOSTS, {
    label: `GoneElsewhere${Date.now()}`,
    host: 'arena-gone.invalid',
    port: 8090,
    isEnabled: false,
  });
  let removed = false;

  try {
    await gotoOperatorSettings(page);
    const row = resolumeRow(page, host.id);
    await page.locator(`[data-role="host-edit"][data-id="${host.id}"]`).click();
    const label = row.locator('[data-role="host-label"]');
    await label.fill(`${host.label}Unsaved`);
    await expect(label).toBeFocused();

    // Another operator (another tab, the API) removes the row being edited.
    await deleteVia(page, `${RESOLUME_HOSTS}/${host.id}`);
    removed = true;

    // The card's next poll no longer lists it: the editor closes and says why,
    // instead of silently throwing the typed change away.
    await waitForToast(page, 'This connection was removed elsewhere.');
    await expect(page.locator('[data-role="host-editor"]')).toHaveCount(0);
    await expect(row).toHaveCount(0);
    // Focus does not fall back to <body>: it lands on "+ Add connection", unlocked.
    const add = page.locator('[data-role="host-add"]');
    await expect(add).toBeEnabled();
    await expect(add).toBeFocused();
  } finally {
    if (!removed) await deleteVia(page, `${RESOLUME_HOSTS}/${host.id}`);
  }

  expect(errors).toEqual([]);
});

test('#819 while a save is in flight every Edit and "+ Add" is locked and Escape / Cancel are ignored', async ({
  page,
}) => {
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  const [a, b] = await twoHosts(page, 'Busy');
  const savedLabel = `${a.label}Saved`;

  try {
    await gotoOperatorSettings(page);
    const rowA = resolumeRow(page, a.id);
    await page.locator(`[data-role="host-edit"][data-id="${a.id}"]`).click();
    const label = rowA.locator('[data-role="host-label"]');
    await label.fill(savedLabel);

    // Hold the save's PUT until the locked state has been checked.
    const putHeld = deferred();
    const release = deferred();
    await page.route(
      (u) => u.pathname === `${RESOLUME_HOSTS}/${a.id}`,
      async (route) => {
        if (route.request().method() !== 'PUT') {
          await route.fallback();
          return;
        }
        putHeld.resolve();
        await release.promise;
        await route.continue();
      },
    );
    await rowA.locator('[data-role="host-submit"]').click();
    await putHeld.promise;

    const editB = page.locator(`[data-role="host-edit"][data-id="${b.id}"]`);
    const add = page.locator('[data-role="host-add"]');
    await expect(rowA.locator('[data-role="host-submit"]')).toBeDisabled();
    await expect(rowA.locator('[data-role="host-cancel"]')).toBeDisabled();
    for (const trigger of [editB, add]) {
      await expect(trigger).toBeDisabled();
      await expect(trigger).toHaveAttribute('data-lock', 'saving');
    }

    // Escape is ignored while saving: the editor stays open with what was typed.
    await label.press('Escape');
    // Let the page finish handling the key (a frame) before looking.
    await page.evaluate(() => new Promise((done) => requestAnimationFrame(() => done(null))));
    await expect(page.locator('[data-role="host-editor"]')).toHaveCount(1);
    await expect(label).toHaveValue(savedLabel);

    release.resolve();
    await waitForToast(page, 'Updated Resolume connection.');
    await expect(page.locator('[data-role="host-editor"]')).toHaveCount(0);
    await expect(rowA.locator('.settings__host-label')).toHaveText(savedLabel);
    for (const trigger of [editB, add]) {
      await expect(trigger).toBeEnabled();
      await expect(trigger).not.toHaveAttribute('data-lock');
    }
    expect((await resolumeHost(page, a.id)).label).toBe(savedLabel);
  } finally {
    await deleteVia(page, `${RESOLUME_HOSTS}/${a.id}`);
    await deleteVia(page, `${RESOLUME_HOSTS}/${b.id}`);
  }

  expect(errors).toEqual([]);
});

test('#819 unsaved changes lock the other rows\' Edit and "+ Add" until they are saved or cancelled', async ({
  page,
}) => {
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  const [a, b] = await twoHosts(page, 'Dirty');

  try {
    await gotoOperatorSettings(page);
    const rowA = resolumeRow(page, a.id);
    const editB = page.locator(`[data-role="host-edit"][data-id="${b.id}"]`);
    const add = page.locator('[data-role="host-add"]');
    await page.locator(`[data-role="host-edit"][data-id="${a.id}"]`).click();
    const label = rowA.locator('[data-role="host-label"]');

    // An editor with nothing changed locks nothing (one click switches rows).
    await expect(editB).toBeEnabled();
    await expect(add).toBeEnabled();

    // A change locks the other triggers, with a tooltip saying what to do.
    await label.fill(`${a.label}Changed`);
    for (const trigger of [editB, add]) {
      await expect(trigger).toBeDisabled();
      await expect(trigger).toHaveAttribute('data-lock', 'unsaved');
      await expect(trigger).toHaveAttribute('title', 'Save or cancel the open editor first');
    }

    // Typing the stored value back is no change any more.
    await label.fill(a.label);
    for (const trigger of [editB, add]) {
      await expect(trigger).toBeEnabled();
      await expect(trigger).not.toHaveAttribute('title');
    }

    // Every field counts, not only the label.
    await rowA.locator('[data-role="host-port"]').fill('8099');
    await expect(editB).toHaveAttribute('data-lock', 'unsaved');

    // Cancel drops the change and unlocks everything; nothing was saved.
    await rowA.locator('[data-role="host-cancel"]').click();
    await expect(page.locator('[data-role="host-editor"]')).toHaveCount(0);
    for (const trigger of [editB, add]) {
      await expect(trigger).toBeEnabled();
      await expect(trigger).not.toHaveAttribute('data-lock');
    }
    expect((await resolumeHost(page, a.id)).port).toBe(a.port);
  } finally {
    await deleteVia(page, `${RESOLUME_HOSTS}/${a.id}`);
    await deleteVia(page, `${RESOLUME_HOSTS}/${b.id}`);
  }

  expect(errors).toEqual([]);
});

test('#819 a row deleted elsewhere during its own save: focus reaches "+ Add" once the save settles', async ({
  page,
}) => {
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  const host = await createVia<ResolumeHost>(page, RESOLUME_HOSTS, {
    label: `GoneMidSave${Date.now()}`,
    host: 'arena-gone-mid-save.invalid',
    port: 8090,
    isEnabled: false,
  });
  let removed = false;

  try {
    await gotoOperatorSettings(page);
    const row = resolumeRow(page, host.id);
    await page.locator(`[data-role="host-edit"][data-id="${host.id}"]`).click();
    await row.locator('[data-role="host-label"]').fill(`${host.label}Saved`);

    // Hold the save's PUT, and remove the row elsewhere while it is in flight.
    const putHeld = deferred();
    const release = deferred();
    await page.route(
      (u) => u.pathname === `${RESOLUME_HOSTS}/${host.id}`,
      async (route) => {
        if (route.request().method() !== 'PUT') {
          await route.fallback();
          return;
        }
        putHeld.resolve();
        await release.promise;
        await route.continue();
      },
    );
    await row.locator('[data-role="host-submit"]').click();
    await putHeld.promise;
    await deleteVia(page, `${RESOLUME_HOSTS}/${host.id}`);
    removed = true;

    // The next poll closes the editor even though its save is still in flight…
    const toast = page.locator('[data-role="settings-toast"]');
    await expect(page.locator('[data-role="host-editor"]')).toHaveCount(0, { timeout: 20_000 });
    await expect(toast).toHaveText('This connection was removed elsewhere.');
    // …while "+ Add" stays locked by that save, so it cannot take focus yet.
    const add = page.locator('[data-role="host-add"]');
    await expect(add).toHaveAttribute('data-lock', 'saving');
    await expect(add).toBeDisabled();

    // The PUT now reaches a server without the row (a real 404): the save fails,
    // the lock lifts, and the focus request still lands on "+ Add" — it was not used
    // up on the disabled button.
    release.resolve();
    await expect(toast).toHaveText(/^Unable to save connection\./);
    await expect(add).toBeEnabled();
    await expect(add).toBeFocused();
  } finally {
    if (!removed) await deleteVia(page, `${RESOLUME_HOSTS}/${host.id}`);
  }

  // Exactly one deliberate non-2xx — that PUT's 404, which Chrome logs (ui skill
  // #598 / #718) — and nothing else.
  const notFound = errors.filter((e) =>
    /Failed to load resource: the server responded with a status of 404\b/.test(e),
  );
  expect(notFound).toHaveLength(1);
  expect(errors.filter((e) => !notFound.includes(e))).toEqual([]);
});
