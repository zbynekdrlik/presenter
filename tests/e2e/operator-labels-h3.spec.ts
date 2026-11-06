/**
 * Operator labels + control sizes E2E (#875 H3, mockup v1 table "Preklad nápisov").
 *
 * On Piesne and Biblia, at 1280×720 and 1920×1080:
 *   - no element of the operator is drawn ALL CAPS by CSS (`text-transform: uppercase`);
 *   - the approved Slovak labels are on screen, none of the old English ones remains
 *     (data — library, song and book names, layout names — is not a label);
 *   - every button, select and field is at least 40 px tall, except the header row
 *     (its controls fit the 40 px row) and the controls lying ON the stage preview
 *     picture (H2); the main actions (show on the wall, clear the wall) are 48 px.
 * The Stage Text select (API layouts only) offers Originál / Preklad / Oboje.
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

/** Old English labels of the v1 table and the operator pages (whole words, case-sensitive). */
const ENGLISH = [
  "Libraries",
  "Playlists",
  "Presentations",
  "Stage Output",
  "Stage Text",
  "Translation",
  "Original",
  "Both",
  "Live",
  "Edit",
  "Follow",
  "ON",
  "OFF",
  "Prepared",
  "Settings",
  "Main",
  "Secondary",
  "None",
  "Find book",
  "Start typing",
  "Select a presentation",
  "Select a library",
  "Search libraries",
  "Search Bible",
  "Load passage",
  "Select all",
  "Clear",
  "selected",
];

/** Containers whose text is DATA (names from the library / Bible), not labels. */
const DATA = [
  '[data-role="library-list"]',
  '[data-role="playlist-list"]',
  '[data-role="presentation-list"]',
  '[data-role="book-list"]',
  '[data-role="slides"]',
  '[data-role="context-title"]',
  '[data-role="stage-preview-frame"]',
  "option",
].join(", ");

async function openOperator(page: Page, viewport: { width: number; height: number }) {
  await page.setViewportSize(viewport);
  await page.goto(`${baseURL}/ui/operator`);
  await page.waitForSelector('body[data-wasm-ready="true"]', { state: "attached" });
  await expect(page.locator("body")).toHaveAttribute("data-shell", "desktop");
}

/** Visible operator elements drawn with `text-transform: uppercase`. */
function uppercaseElements(page: Page) {
  return page.evaluate(() =>
    [...document.querySelectorAll<HTMLElement>(".operator__header, .operator__header *, .operator__main *")]
      .filter((el) => el.getClientRects().length > 0)
      .filter((el) => getComputedStyle(el).textTransform === "uppercase")
      .map((el) => `${el.tagName.toLowerCase()}.${el.className} "${el.innerText.trim().slice(0, 30)}"`),
  );
}

/** Every label-like text of the visible operator: element text outside the
 *  data containers, placeholders, titles and aria-labels. */
function labelTexts(page: Page, data: string) {
  return page.evaluate((dataSelector) => {
    const texts: string[] = [];
    const roots = [...document.querySelectorAll<HTMLElement>(".operator__header, .operator__main")];
    for (const root of roots) {
      const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
      for (let node = walker.nextNode(); node; node = walker.nextNode()) {
        const parent = node.parentElement;
        if (!parent || parent.closest(dataSelector) || parent.getClientRects().length === 0) continue;
        const text = (node.textContent ?? "").trim();
        if (text) texts.push(text);
      }
      for (const el of root.querySelectorAll<HTMLElement>("[placeholder], [title], [aria-label]")) {
        if (el.closest(dataSelector) || el.getClientRects().length === 0) continue;
        for (const attr of ["placeholder", "title", "aria-label"]) {
          const value = el.getAttribute(attr);
          if (value) texts.push(value);
        }
      }
    }
    return texts;
  }, data);
}

function englishIn(texts: string[]): string[] {
  return texts.filter((text) =>
    ENGLISH.some((word) => new RegExp(`(^|[^\\p{L}])${word}($|[^\\p{L}])`, "u").test(text)),
  );
}

/** Visible buttons / selects / fields below `min` px, outside the header row and the preview picture. */
function smallControls(page: Page, min: number) {
  return page.evaluate((minHeight) => {
    const skip = ["checkbox", "radio", "range", "color", "file", "hidden"];
    return [...document.querySelectorAll<HTMLElement>(".operator__main button, .operator__main select, .operator__main input")]
      .filter((el) => el.getClientRects().length > 0)
      .filter((el) => !(el instanceof HTMLInputElement && skip.includes(el.type)))
      .filter((el) => !el.closest(".operator__stage-preview"))
      .filter((el) => el.getBoundingClientRect().height < minHeight - 0.5)
      .map(
        (el) =>
          `${el.tagName.toLowerCase()}[${el.dataset.role ?? ""}] "${(el.innerText || el.getAttribute("aria-label") || "").trim().slice(0, 30)}" ${el.getBoundingClientRect().height.toFixed(1)} px`,
      );
  }, min);
}

/** Header row controls must lie inside the header (≤ 44 px, H1). */
async function headerRowFits(page: Page) {
  return page.evaluate(() => {
    const header = document.querySelector(".operator__header")!.getBoundingClientRect();
    const outside = [...document.querySelectorAll<HTMLElement>(".operator__header button, .operator__header select, .operator__header input")]
      .filter((el) => el.getClientRects().length > 0)
      .filter((el) => {
        const r = el.getBoundingClientRect();
        return r.top < header.top - 1 || r.bottom > header.bottom + 1;
      })
      .map((el) => (el.innerText || el.getAttribute("aria-label") || el.tagName).trim());
    return { height: header.height, outside };
  });
}

async function heightOf(page: Page, selector: string) {
  const box = await page.locator(selector).first().boundingBox();
  if (!box) throw new Error(`${selector} has no box`);
  return box.height;
}

for (const viewport of VIEWPORTS) {
  const size = `${viewport.width}×${viewport.height}`;

  test(`Piesne at ${size}: Slovak sentence-case labels, 40 px controls`, async ({ page }) => {
    const errors: string[] = [];
    attachConsoleErrorCollector(page, errors);
    await openOperator(page, viewport);
    await openView(page, "worship");
    const worship = page.locator('[data-view-panel="worship"]');
    await expect(worship).toBeVisible();

    await expect(worship.locator(".operator__group h2").first()).toHaveText("Knižnice");
    await expect(worship.locator("h2", { hasText: "Playlisty" })).toBeVisible();
    await expect(page.locator('[data-role="context-title"]')).toHaveText("Piesne v knižnici");
    await expect(worship.locator('[data-role="slides"] .empty')).toHaveText("Vyber pieseň vľavo.");
    await expect(page.locator('[data-role="global-search-query"]')).toHaveAttribute(
      "placeholder",
      "Hľadať knižnice, piesne, slajdy",
    );
    await expect(page.locator('[data-role="mode-toggle"][data-mode="live"]')).toHaveText("Naživo");
    await expect(page.locator('[data-role="mode-toggle"][data-mode="edit"]')).toHaveText("Úprava");
    await expect(page.locator('[data-role="stage-layout-select"]')).toHaveAttribute(
      "title",
      "Rozloženie na pódiových obrazovkách",
    );
    await expect(page.locator('[data-role="ableset-enable"]')).toHaveText("Ableton");
    await expect(page.locator('[data-role="ableset-follow"]')).toHaveText("Sledovať pódium");

    // Pick a library and a song so the lists and the slide grid are full.
    await page.locator('[data-role="library-item"]').first().click();
    await page.locator('[data-role="presentation-item"]').first().click();
    await expect(page.locator('[data-role="slides"] [data-slide-id]').first()).toBeVisible();

    expect(await uppercaseElements(page)).toEqual([]);
    expect(englishIn(await labelTexts(page, DATA))).toEqual([]);
    expect(await smallControls(page, 40)).toEqual([]);
    const row = await headerRowFits(page);
    expect(row.height).toBeLessThanOrEqual(44);
    expect(row.outside).toEqual([]);

    // Edit mode: the slide card controls follow the same rule.
    await page.locator('[data-role="mode-toggle"][data-mode="edit"]').click();
    await expect(page.locator('[data-action="duplicate"]').first()).toHaveText("Duplikovať");
    await expect(page.locator('[data-action="delete"]').first()).toHaveText("Zmazať");
    expect(await uppercaseElements(page)).toEqual([]);
    expect(englishIn(await labelTexts(page, DATA))).toEqual([]);
    expect(await smallControls(page, 40)).toEqual([]);
    await page.locator('[data-role="mode-toggle"][data-mode="live"]').click();

    expect(errors).toEqual([]);
  });

  test(`Biblia at ${size}: Slovak sentence-case labels, 40 px controls, 48 px main actions`, async ({
    page,
  }) => {
    const errors: string[] = [];
    attachConsoleErrorCollector(page, errors);
    await openOperator(page, viewport);
    await openView(page, "bible");
    await expect(page.locator('[data-view-panel="bible"]')).toBeVisible();

    const tab = (id: string) => page.locator(`[data-role="bible-tab"][data-tab="${id}"]`);
    await expect(tab("live")).toHaveText("Naživo");
    await expect(tab("prepared")).toHaveText("Pripravené");
    await expect(tab("settings")).toHaveText("Nastavenie");
    await expect(page.locator(".bible__translation-row span", { hasText: "Hlavný preklad" })).toBeVisible();
    await expect(page.locator(".bible__translation-row span", { hasText: "Druhý preklad" })).toBeVisible();
    await expect(page.locator('[data-role="secondary-translation"] option[value=""]')).toHaveText("žiadny");
    await expect(page.locator('[data-role="book-filter"]')).toHaveAttribute("placeholder", "Začni písať názov knihy");
    await expect(page.locator("label.operator__field > span", { hasText: /^Kniha$/ })).toBeVisible();
    await expect(page.locator('[data-role="global-search-query"]')).toHaveAttribute(
      "placeholder",
      "Hľadať verše v Biblii",
    );

    expect(await uppercaseElements(page)).toEqual([]);
    expect(englishIn(await labelTexts(page, DATA))).toEqual([]);
    expect(await smallControls(page, 40)).toEqual([]);
    // Show a verse: the main action and the one that clears it are 48 px.
    await page.locator('[data-role="console-field"]').fill("John 3:16");
    await expect(page.locator('[data-role="bible-console"]')).toHaveAttribute("data-preview-ready", "true");
    expect(await heightOf(page, '[data-role="console-show"]')).toBeGreaterThanOrEqual(47.5);
    await page.locator('[data-role="console-show"]').click();
    const clearLive = page.locator('[data-role="console-clear-live"]');
    await expect(clearLive).toBeVisible();
    expect(await heightOf(page, '[data-role="console-clear-live"]')).toBeGreaterThanOrEqual(47.5);
    expect(await smallControls(page, 40)).toEqual([]);
    await clearLive.click();
    await expect(page.locator('[data-role="console-on-air"]')).toHaveText("Na stene: nič");
    const row = await headerRowFits(page);
    expect(row.height).toBeLessThanOrEqual(44);
    expect(row.outside).toEqual([]);

    // The other two tabs.
    for (const id of ["prepared", "settings"]) {
      await tab(id).click();
      await expect(tab(id)).toHaveAttribute("data-active", "true");
      expect(await uppercaseElements(page), id).toEqual([]);
      expect(englishIn(await labelTexts(page, DATA)), id).toEqual([]);
      expect(await smallControls(page, 40), id).toEqual([]);
    }
    await tab("live").click();

    expect(errors).toEqual([]);
  });
}

test("Stage Text offers Originál / Preklad / Oboje on an API layout", async ({ page }) => {
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  await openOperator(page, VIEWPORTS[0]);
  const layout = page.locator('[data-role="stage-layout-select"]');
  const before = await layout.inputValue();
  await layout.selectOption("api");
  const mode = page.locator('[data-role="stage-text-mode-select"]');
  await expect(mode).toBeVisible();
  await expect(mode).toHaveAttribute("title", "Text na pódiu");
  await expect(mode.locator("option")).toHaveText(["Originál", "Preklad", "Oboje"]);
  expect(await uppercaseElements(page)).toEqual([]);
  const row = await headerRowFits(page);
  expect(row.outside).toEqual([]);
  await layout.selectOption(before);
  await expect(mode).toHaveCount(0);
  expect(errors).toEqual([]);
});
