/**
 * Nastavenia + Stream editor on the shared controls E2E (#875 H4, mockup v1
 * "Jeden vzhľad prvkov", "Drobné tlačidlá", the "Preklad nápisov" table).
 *
 * At 1280×720 and 1920×1080:
 *   - Nastavenia (the operator panel AND the standalone /ui/settings) and the
 *     Stream editor (/ui/stream, every tab, an open scene with an element form,
 *     the output name form) draw nothing ALL CAPS by CSS;
 *   - the Slovak labels are on screen and no English word of the old copy is
 *     left outside the data containers (host labels and addresses, NDI names,
 *     scene names, server error text, the clip-name tokens, the version);
 *   - every button, select and field is at least 40 px tall, except the Stream
 *     editor's top row (the operator header's look: 32 px controls in a 40 px
 *     row) — the panel a top-row button opens is full size;
 *   - the buttons are the shared `.ui-button` / `.ui-toggle` (no page restyle);
 *     a state toggle carries `data-state` + `aria-pressed`;
 *   - the Stream editor has ONE top row: Stream, the output, the output bar,
 *     the tabs and the way back fit inside it.
 *
 * Also asserts zero browser console errors/warnings per
 * ci/browser-console-zero-errors.md.
 */

import { test, expect, type Page } from "@playwright/test";
import {
  attachConsoleErrorCollector,
  deriveTestConfig,
  openView,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

let serverHandle: ServerHandle | undefined;
let baseURL: string;

test.describe.configure({ timeout: 180_000 });

test.beforeAll(async ({}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  baseURL = config.baseURL;
  await refreshDevData(config.dbUrl);
  serverHandle = await startTestServer(config.port, config.dbUrl);
});

test.afterAll(async () => {
  await stopServer(serverHandle);
});

const VIEWPORTS = [
  { width: 1280, height: 720 },
  { width: 1920, height: 1080 },
];

/** Old English copy of Nastavenia and the Stream editor (whole words, case-sensitive). */
const ENGLISH = [
  "Settings",
  "Back to hub",
  "Preferences",
  "Connections",
  "Hosts",
  "Displays",
  "Video Sources",
  "Supported",
  "Save",
  "Cancel",
  "Edit",
  "Delete",
  "Remove",
  "Test",
  "Scan",
  "Activate",
  "ACTIVE",
  "Add",
  "Enable",
  "Enabled",
  "Label",
  "Hostname",
  "Refresh mapping",
  "Connected",
  "Connecting",
  "Disabled",
  "Running",
  "Retrying",
  "Updated",
  "Created",
  "Last",
  "Checking",
  "Ready",
  "Live",
  "Not found",
  "Current song",
  "Line limit",
  "Graphics",
  "Base",
  "Overlay",
  "Operator",
  "URL",
  "Contain",
  "Cover",
  "Stretch",
  "Asset",
];

/** Containers whose text is DATA (stored names, addresses, server text), not labels. */
const DATA = [
  ".settings__host-label",
  ".settings__host-addr",
  ".settings__ndi-parts",
  ".settings__source-detail",
  '[data-role="android-error"]',
  '[data-role="ableset-status-error"]',
  '[data-role="osc-status-error"]',
  '[data-role="ableset-status-song"]',
  '[data-role="resolume-clip-legend"] dt',
  '[data-role="ndi-discovered"] button',
  '[data-role="stream-scene-name"]',
  '[data-testid="version"]',
  ".stream-editor__font-preview",
  "option",
].join(", ");

const SKIP_TYPES = ["checkbox", "radio", "range", "color", "file", "hidden"];

/** Visible elements under `root` drawn with `text-transform: uppercase`. */
function uppercaseElements(page: Page, root: string) {
  return page.evaluate(
    (rootSelector) =>
      [...document.querySelectorAll<HTMLElement>(`${rootSelector}, ${rootSelector} *`)]
        .filter((el) => el.getClientRects().length > 0)
        .filter((el) => getComputedStyle(el).textTransform === "uppercase")
        .map((el) => `${el.tagName.toLowerCase()}.${el.className} "${el.innerText.trim().slice(0, 30)}"`),
    root,
  );
}

/** Every label-like text under `root`: element text outside the data containers,
 *  placeholders, titles and aria-labels. */
function labelTexts(page: Page, root: string) {
  return page.evaluate(
    ({ rootSelector, dataSelector }) => {
      const texts: string[] = [];
      for (const scope of document.querySelectorAll<HTMLElement>(rootSelector)) {
        const walker = document.createTreeWalker(scope, NodeFilter.SHOW_TEXT);
        for (let node = walker.nextNode(); node; node = walker.nextNode()) {
          const parent = node.parentElement;
          if (!parent || parent.closest(dataSelector) || parent.getClientRects().length === 0) continue;
          const text = (node.textContent ?? "").trim();
          if (text) texts.push(text);
        }
        for (const el of scope.querySelectorAll<HTMLElement>("[placeholder], [title], [aria-label]")) {
          if (el.closest(dataSelector) || el.getClientRects().length === 0) continue;
          for (const attr of ["placeholder", "title", "aria-label"]) {
            const value = el.getAttribute(attr);
            if (value) texts.push(value);
          }
        }
      }
      return texts;
    },
    { rootSelector: root, dataSelector: DATA },
  );
}

function englishIn(texts: string[]): string[] {
  return texts.filter((text) =>
    ENGLISH.some((word) => new RegExp(`(^|[^\\p{L}])${word}($|[^\\p{L}])`, "u").test(text)),
  );
}

/** Visible buttons / selects / fields under `root` below 40 px; `.ui-topbar`'s own
 *  controls are left out (the operator header's 32 px look), its panel is not. */
function smallControls(page: Page, root: string) {
  return page.evaluate(
    ({ rootSelector, skip }) =>
      [...document.querySelectorAll<HTMLElement>(
        `${rootSelector} button, ${rootSelector} select, ${rootSelector} input`,
      )]
        .filter((el) => el.getClientRects().length > 0)
        .filter((el) => !(el instanceof HTMLInputElement && skip.includes(el.type)))
        .filter((el) => !el.closest(".ui-topbar") || !!el.closest(".ui-topbar__panel"))
        .filter((el) => el.getBoundingClientRect().height < 39.5)
        .map(
          (el) =>
            `${el.tagName.toLowerCase()}[${el.dataset.role ?? ""}] "${(el.innerText || el.getAttribute("aria-label") || "").trim().slice(0, 30)}" ${el.getBoundingClientRect().height.toFixed(1)} px`,
        ),
    { rootSelector: root, skip: SKIP_TYPES },
  );
}

/** Visible buttons under `root` that do not wear the shared look. */
function unsharedButtons(page: Page, root: string, allowed: string[]) {
  return page.evaluate(
    ({ rootSelector, allowedClasses }) =>
      [...document.querySelectorAll<HTMLElement>(`${rootSelector} button`)]
        .filter((el) => el.getClientRects().length > 0)
        .filter((el) => !["ui-button", "ui-toggle", ...allowedClasses].some((c) => el.classList.contains(c)))
        .map((el) => `button.${el.className} "${el.innerText.trim().slice(0, 30)}"`),
    { rootSelector: root, allowedClasses: allowed },
  );
}

/** The Stream editor's top row: its height and every control that sticks out of it. */
function topRow(page: Page) {
  return page.evaluate(() => {
    const header = document.querySelector<HTMLElement>('[data-role="stream-editor-header"]')!;
    const box = header.getBoundingClientRect();
    const outside = [...header.querySelectorAll<HTMLElement>("button, select, input, a")]
      .filter((el) => el.getClientRects().length > 0 && !el.closest(".ui-topbar__panel"))
      .filter((el) => {
        const r = el.getBoundingClientRect();
        return r.top < box.top - 1 || r.bottom > box.bottom + 1 || r.right > box.right + 1;
      })
      .map((el) => (el.innerText || el.getAttribute("aria-label") || el.tagName).trim());
    return { height: box.height, outside };
  });
}

async function seedSettingsRows(page: Page, stamp: string) {
  const post = async (path: string, data: Record<string, unknown>) => {
    const response = await page.request.post(new URL(path, baseURL).toString(), { data });
    expect(response.ok(), `POST ${path} → ${response.status()}`).toBeTruthy();
    return (await response.json()) as { id: string | number };
  };
  const host = await post("/integrations/resolume/hosts", {
    label: `Arena H4 ${stamp}`,
    host: "arena-h4.invalid",
    port: 8090,
    isEnabled: false,
  });
  const display = await post("/integrations/android-stage/displays", {
    label: `TV H4 ${stamp}`,
    host: "tv-h4.invalid",
    port: 5555,
    isEnabled: false,
  });
  const source = await post("/integrations/video-sources", { label: "H4", ndiName: `PC-H4 (cam ${stamp})` });
  const cleanup = async () => {
    await page.request.delete(new URL(`/integrations/resolume/hosts/${host.id}`, baseURL).toString());
    await page.request.delete(new URL(`/integrations/android-stage/displays/${display.id}`, baseURL).toString());
    await page.request.delete(new URL(`/integrations/video-sources/${source.id}`, baseURL).toString());
  };
  return { ids: { host: String(host.id), display: String(display.id), source: String(source.id) }, cleanup };
}

type Seeded = { host: string; display: string; source: string };

/** The checks every Nastavenia context shares. */
async function expectSettingsOnSharedControls(page: Page, root: string, ids: Seeded) {
  const layout = page.locator(root);
  await expect(layout.locator(".settings__card-header h2")).toContainText([
    "Companion",
    "Predvoľby",
    "Pripojenia Resolume Arena",
    "Pódiové obrazovky Android",
    "Počúvanie",
    "Ovládanie z Abletonu",
    "Zdroje videa",
    "Zmazané piesne",
  ]);
  const resolumeRow = layout.locator(`[data-role="resolume-host-list"] li[data-id="${ids.host}"]`);
  await expect(resolumeRow).toBeVisible();
  await expect(resolumeRow.locator('[data-role="host-status"]')).toHaveText("Vypnuté");
  await expect(resolumeRow.locator('[data-role="host-test"]')).toHaveText("Otestovať");
  await expect(resolumeRow.locator('[data-role="host-refresh-mapping"]')).toHaveText("Načítať klipy");
  await expect(layout.locator('[data-role="resolume-mapping-hint"]')).toContainText("Načítať klipy");
  await expect(resolumeRow.getByRole("button", { name: "Upraviť", exact: true })).toBeVisible();
  await expect(resolumeRow.getByRole("button", { name: "Odstrániť", exact: true })).toBeVisible();
  await expect(
    layout.locator(`[data-role="android-display-list"] li[data-id="${ids.display}"] [data-role="android-status"]`),
  ).toHaveText("Vypnuté");
  const sourceToggle = layout.locator(
    `[data-role="video-source-row"][data-source-id="${ids.source}"] [data-role="video-source-active"]`,
  );
  await expect(sourceToggle).toHaveText("Aktívny");
  await expect(sourceToggle).toHaveAttribute("data-state", "off");
  await expect(sourceToggle).toHaveAttribute("aria-pressed", "false");
  await expect(sourceToggle).toHaveClass(/\bui-toggle\b/);

  // The inline editor too: open a row's Upraviť, check, Zrušiť.
  await resolumeRow.getByRole("button", { name: "Upraviť", exact: true }).click();
  const editor = layout.locator('[data-role="host-editor"]');
  await expect(editor).toBeVisible();
  await expect(editor.locator("label > span").first()).toHaveText("Názov");
  await expect(editor.getByRole("button", { name: "Uložiť", exact: true })).toBeVisible();

  expect(await uppercaseElements(page, root)).toEqual([]);
  expect(englishIn(await labelTexts(page, root))).toEqual([]);
  expect(await smallControls(page, root)).toEqual([]);
  // The one chip that is not a button look: a discovered NDI name (data).
  expect(await unsharedButtons(page, root, ["settings__discovered-name"])).toEqual([]);

  await editor.getByRole("button", { name: "Zrušiť", exact: true }).click();
  await expect(editor).toHaveCount(0);
}

for (const viewport of VIEWPORTS) {
  const size = `${viewport.width}×${viewport.height}`;

  test(`Nastavenia in the operator at ${size}: Slovak labels, shared 40 px controls`, async ({ page }) => {
    const errors: string[] = [];
    attachConsoleErrorCollector(page, errors);
    const { ids, cleanup } = await seedSettingsRows(page, `${viewport.width}o`);
    try {
      await page.setViewportSize(viewport);
      await page.goto(`${baseURL}/ui/operator`);
      await page.waitForSelector('body[data-wasm-ready="true"]', { state: "attached" });
      await expect(page.locator("body")).toHaveAttribute("data-shell", "desktop");
      await openView(page, "settings");
      await expect(page.locator('[data-view-panel="settings"] .settings-layout')).toBeVisible();
      await expectSettingsOnSharedControls(page, '[data-view-panel="settings"] .settings-layout', ids);
    } finally {
      await cleanup();
    }
    expect(errors).toEqual([]);
  });

  test(`standalone /ui/settings at ${size}: Slovak labels, shared 40 px controls`, async ({ page }) => {
    const errors: string[] = [];
    attachConsoleErrorCollector(page, errors);
    const { ids, cleanup } = await seedSettingsRows(page, `${viewport.width}s`);
    try {
      await page.setViewportSize(viewport);
      await page.goto(`${baseURL}/ui/settings`);
      await page.waitForSelector('body[data-wasm-ready="true"]', { state: "attached" });
      await expect(page.locator(".settings__header h1")).toHaveText("Nastavenia");
      await expect(page.locator(".settings__header-nav .settings__link")).toHaveText("← Späť na rozcestník");
      await expectSettingsOnSharedControls(page, ".settings-layout", ids);
    } finally {
      await cleanup();
    }
    expect(errors).toEqual([]);
  });

  test(`Stream editor at ${size}: one top row, Slovak labels, shared 40 px controls`, async ({ page }) => {
    const errors: string[] = [];
    attachConsoleErrorCollector(page, errors);
    await page.setViewportSize(viewport);
    await page.goto(`${baseURL}/ui/stream?tab=scenes&output=stream`);
    await page.waitForSelector('body[data-wasm-ready="true"]', { state: "attached" });
    await page.waitForSelector('[data-role="stream-editor"]');
    const header = page.locator('[data-role="stream-editor-header"]');
    await expect(header.locator("h1")).toHaveText("Stream");
    await expect(page.locator('[data-role="stream-editor-tab"]')).toHaveText(["Scény", "Menovky", "Písma"]);
    await expect(page.locator('[data-role="stream-output-new"]')).toHaveText("Nový výstup");
    await expect(page.locator('[data-role="stream-output-rename"]')).toHaveText("Premenovať");
    await expect(page.locator('[data-role="stream-output-delete"]')).toHaveText("Zmazať výstup");
    await expect(page.locator('[data-role="stream-editor-back"]')).toHaveText("← Operátor");
    await expect(page.locator('[data-role="stream-overlay-row"] h2')).toHaveText("Prekryvné scény");
    await expect(page.locator('[data-role="stream-base-section"] h2')).toHaveText("Základné scény");
    // Everything of the page header lives in that one row.
    for (const old of [".stream-editor__header-title", '[data-role="stream-output-manage"] + [data-role="stream-editor-tabs"]']) {
      expect(await page.locator(`.stream-editor > ${old}`).count(), old).toBe(0);
    }
    let row = await topRow(page);
    expect(row.height).toBeLessThanOrEqual(44);
    expect(row.outside).toEqual([]);

    // A scene with a text element: the cards, the element list and its form.
    const name = `H4 scéna ${viewport.width}`;
    await page.locator('[data-role="stream-add-name"]').fill(name);
    await page.locator('[data-role="stream-add-submit"]').click();
    const card = page.locator('[data-role="stream-scene"]').filter({ hasText: name });
    await expect(card).toHaveCount(1);
    const live = card.locator('[data-role="stream-activate"]');
    await expect(live).toHaveText("Naživo");
    await expect(live).toHaveAttribute("data-state", "off");
    await live.click();
    await expect(live).toHaveAttribute("data-state", "on");
    await expect(live).toHaveAttribute("aria-pressed", "true");
    await expect(card).toHaveAttribute("data-active", "true");
    await card.locator('[data-role="stream-scene-edit"]').click();
    await expect(page.locator('[data-role="stream-element-panel"]')).toBeVisible();
    await page.locator('[data-role="stream-add-element-countdown"]').click();
    await expect(page.locator('[data-role="stream-element"]')).toHaveCount(1);
    await expect(page.locator('[data-role="stream-prop-form"]')).toBeVisible();
    const left = page.locator('[data-role="stream-ts-align-left"]').first();
    await expect(left).toHaveAttribute("aria-label", "Zarovnať vľavo");
    await expect(left).toHaveAttribute("data-state", /^(on|off)$/);
    const liveToggle = page.locator('[data-role="stream-preview-live-toggle"]');
    const previewTitle = page.locator('[data-role="stream-preview-title"]');
    await expect(liveToggle).toHaveText("Naživo");
    // The title says what the picture shows, in both states.
    await expect(liveToggle).toHaveAttribute("aria-pressed", "false");
    await expect(previewTitle).toHaveText("Náhľad — upravovaná scéna");
    await liveToggle.click();
    await expect(liveToggle).toHaveAttribute("aria-pressed", "true");
    await expect(previewTitle).toHaveText("Náhľad — naživo");
    await liveToggle.click();
    await expect(previewTitle).toHaveText("Náhľad — upravovaná scéna");

    for (const tab of ["scenes", "nameplates", "fonts"]) {
      await page.locator(`[data-role="stream-editor-tab"][data-tab="${tab}"]`).click();
      await expect(page.locator(`[data-role="stream-tab-panel"][data-tab="${tab}"]`)).toBeVisible();
      expect(await uppercaseElements(page, ".stream-editor"), tab).toEqual([]);
      expect(englishIn(await labelTexts(page, ".stream-editor")), tab).toEqual([]);
      expect(await smallControls(page, ".stream-editor"), tab).toEqual([]);
      expect(
        await unsharedButtons(page, ".stream-editor", ["ui-topbar__tab", "ui-topbar__button", "stream-editor__element-select"]),
        tab,
      ).toEqual([]);
      row = await topRow(page);
      expect(row.height, tab).toBeLessThanOrEqual(44);
      expect(row.outside, tab).toEqual([]);
    }
    await page.locator('[data-role="stream-editor-tab"][data-tab="scenes"]').click();

    // The output name form opens as a panel under the row: the row keeps its
    // height and the panel's controls are full size.
    await page.locator('[data-role="stream-output-new"]').click();
    const form = page.locator('[data-role="stream-output-form"]');
    await expect(form).toBeVisible();
    await expect(form).toHaveClass(/\bui-topbar__panel\b/);
    row = await topRow(page);
    expect(row.height).toBeLessThanOrEqual(44);
    expect(englishIn(await labelTexts(page, ".stream-editor"))).toEqual([]);
    expect(await smallControls(page, ".stream-editor")).toEqual([]);
    await page.locator('[data-role="stream-output-cancel"]').click();
    await expect(form).toHaveCount(0);

    // Leave the scene off the output for the next test.
    await page.locator('[data-role="stream-clear-base"]').click();
    await expect(card).toHaveAttribute("data-active", "false");
    expect(errors).toEqual([]);
  });
}
