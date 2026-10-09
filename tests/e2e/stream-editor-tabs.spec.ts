/**
 * #829: the stream editor's areas live in tabs — Scény | Menovky | Písma.
 *
 * Before, the nameplates (Menovky) and fonts sat at the very bottom of
 * `/ui/stream`, under every scene column, so during a service they were
 * effectively hidden. Now:
 *  - Menovky opens with one click on the tab bar, without scrolling;
 *  - switching tabs never loses work (the panels stay mounted and are only
 *    hidden): a half-typed nameplate and an unsaved element draft survive,
 *    and no "discard changes?" question appears;
 *  - the active tab is remembered (`?tab=` + localStorage) and merges with the
 *    `?output=` param instead of dropping it;
 *  - at 320 px the tabs wrap and the page never scrolls horizontally.
 *
 * Clean console is asserted last.
 */
import { test, expect, type Page } from "@playwright/test";
import {
  attachEditorConsoleCollector,
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

let serverHandle: ServerHandle | undefined;
let baseURL: string;

type ElementDef = { id: number };
type SceneDef = { id: number; elements: ElementDef[] };
type OutputDef = { scenes: SceneDef[] };

const TAB_STORAGE_KEY = "stream-editor-tab";
const TABS = ["scenes", "nameplates", "fonts"] as const;
type Tab = (typeof TABS)[number];

const tab = (page: Page, id: Tab) =>
  page.locator(`[data-role="stream-editor-tab"][data-tab="${id}"]`);
const panel = (page: Page, id: Tab) =>
  page.locator(`[data-role="stream-tab-panel"][data-tab="${id}"]`);
const frameX = '[data-role="stream-frame-x"]';
const saveBtn = '[data-role="stream-prop-save"]';

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

/** Open the editor and wait for its tab bar (whichever tab is active). */
async function openEditor(page: Page, query = "") {
  await page.goto(`${baseURL}/ui/stream${query}`);
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  await expect(page.locator('[data-role="stream-editor-tabs"]')).toBeVisible({ timeout: 30_000 });
}

/** The given tab is the active one: its button + panel are active and shown,
 *  every other panel is hidden. */
async function expectActiveTab(page: Page, active: Tab) {
  for (const id of TABS) {
    const on = id === active ? "true" : "false";
    await expect(tab(page, id)).toHaveAttribute("data-active", on);
    await expect(panel(page, id)).toHaveAttribute("data-active", on);
    if (id === active) {
      await expect(panel(page, id)).toBeVisible();
    } else {
      await expect(panel(page, id)).toBeHidden();
    }
  }
}

async function getDef(page: Page): Promise<OutputDef> {
  const res = await page.request.get(`${baseURL}/stream/api/outputs/stream/def`);
  expect(res.ok()).toBeTruthy();
  return (await res.json()) as OutputDef;
}

/** A scene with one freshly-added colour element, selected in the form. */
async function sceneWithSelectedElement(page: Page, name: string): Promise<string> {
  await page.locator('[data-role="stream-add-name"]').fill(name);
  await page.locator('[data-role="stream-add-kind"]').selectOption("base");
  await page.locator('[data-role="stream-add-submit"]').click();
  const card = page
    .locator('[data-role="stream-scene"]')
    .filter({ has: page.getByText(name, { exact: true }) });
  await expect(card).toHaveCount(1, { timeout: 15_000 });
  const sceneId = (await card.getAttribute("data-scene-id")) as string;
  await card.locator('[data-role="stream-scene-edit"]').click();
  await page.waitForSelector('[data-role="stream-element-panel"]', { timeout: 15_000 });
  await page.locator('[data-role="stream-add-element-color"]').click();
  let elementId = "";
  await expect
    .poll(async () => {
      const scene = (await getDef(page)).scenes.find((s) => String(s.id) === sceneId);
      elementId = scene?.elements[0] ? String(scene.elements[0].id) : "";
      return elementId !== "";
    })
    .toBe(true);
  await expect(
    page.locator(`[data-role="stream-overlay-element"][data-element-id="${elementId}"]`),
  ).toHaveAttribute("data-selected", "true", { timeout: 15_000 });
  return elementId;
}

test("#829 Menovky in its own tab: no scrolling, nothing lost on a switch, the tab is remembered", async ({
  page,
}) => {
  const errors: string[] = [];
  attachEditorConsoleCollector(page, errors);
  const dialogs: string[] = [];
  page.on("dialog", (d) => {
    dialogs.push(d.message());
    void d.dismiss();
  });

  await openEditor(page);
  await expectActiveTab(page, "scenes");
  await expect(tab(page, "scenes")).toHaveText("Scény");
  await expect(tab(page, "nameplates")).toHaveText("Menovky");
  await expect(tab(page, "fonts")).toHaveText("Písma");

  // An unsaved element edit on the Scény tab.
  await sceneWithSelectedElement(page, "SC_829_Tabs");
  await page.locator(frameX).fill("33");
  await expect(page.locator(saveBtn)).toHaveAttribute("data-dirty", "true");

  // Menovky opens from the top of the page, with nothing to scroll past.
  await page.evaluate(() => window.scrollTo(0, 0));
  await tab(page, "nameplates").click();
  await expectActiveTab(page, "nameplates");
  const nameplates = page.locator('[data-role="stream-nameplates"]');
  const box = await nameplates.boundingBox();
  const viewportHeight = page.viewportSize()?.height ?? 0;
  expect(box, "Menovky panel has a box").toBeTruthy();
  expect(box!.y, "Menovky panel starts inside the first screen").toBeLessThan(viewportHeight);
  expect(await page.evaluate(() => window.scrollY), "no scrolling needed").toBe(0);

  // Create a plate, then leave a second one half-typed.
  await page.locator('[data-role="stream-nameplate-new-name"]').fill("Tab Ján");
  await page.locator('[data-role="stream-nameplate-new-role"]').fill("kazateľ");
  await page.locator('[data-role="stream-nameplate-add-submit"]').click();
  const plateRow = page
    .locator('[data-role="stream-nameplate"]')
    .filter({ has: page.locator('[data-role="stream-nameplate-name"]') });
  await expect
    .poll(async () =>
      plateRow.evaluateAll((rows) =>
        rows.map((r) => (r.querySelector('[data-role="stream-nameplate-name"]') as HTMLInputElement).value),
      ),
    )
    .toContain("Tab Ján");
  await page.locator('[data-role="stream-nameplate-new-name"]').fill("Napoly");

  // Scény ↔ Menovky: the draft and the half-typed plate are both still there.
  await tab(page, "scenes").click();
  await expectActiveTab(page, "scenes");
  await expect(page.locator(frameX)).toHaveValue("33");
  await expect(page.locator(saveBtn)).toHaveAttribute("data-dirty", "true");
  await tab(page, "nameplates").click();
  await expectActiveTab(page, "nameplates");
  await expect(page.locator('[data-role="stream-nameplate-new-name"]')).toHaveValue("Napoly");

  // Písma is its own tab too.
  await tab(page, "fonts").click();
  await expectActiveTab(page, "fonts");
  await expect(page.locator('[data-role="stream-font-panel"]')).toBeVisible();
  expect(new URL(page.url()).searchParams.get("tab")).toBe("fonts");

  // No tab switch ever asked about the unsaved edit.
  expect(dialogs, "no discard question on a tab switch").toEqual([]);

  // The tab is remembered: a reload (URL) and a fresh open (localStorage).
  await tab(page, "nameplates").click();
  await expectActiveTab(page, "nameplates");
  expect(await page.evaluate((key) => window.localStorage.getItem(key), TAB_STORAGE_KEY)).toBe(
    "nameplates",
  );
  await page.reload();
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  await expectActiveTab(page, "nameplates");
  expect(new URL(page.url()).searchParams.get("tab")).toBe("nameplates");
  await openEditor(page);
  await expectActiveTab(page, "nameplates");

  // Switching the output keeps ?tab= (and the other way round).
  const outputs = (await (await page.request.get(`${baseURL}/stream/api/outputs`)).json()) as Array<{
    slug: string;
  }>;
  const other = outputs.find((o) => o.slug !== "stream")?.slug as string;
  expect(other, "a second output exists").toBeTruthy();
  await page.locator('[data-role="stream-output-select"]').selectOption(other);
  await expect.poll(() => new URL(page.url()).searchParams.get("output")).toBe(other);
  expect(new URL(page.url()).searchParams.get("tab")).toBe("nameplates");
  await tab(page, "scenes").click();
  await expect.poll(() => new URL(page.url()).searchParams.get("tab")).toBe("scenes");
  expect(new URL(page.url()).searchParams.get("output")).toBe(other);

  // After the reload nothing was dirty, so still no question was ever asked.
  expect(dialogs, "no discard question at all").toEqual([]);
  expect(errors, `editor console: ${errors.join(" | ")}`).toEqual([]);
});

test("#829 the tabs wrap at 320 px and the page never scrolls sideways", async ({ page }) => {
  const errors: string[] = [];
  attachEditorConsoleCollector(page, errors);
  await page.setViewportSize({ width: 320, height: 640 });
  await openEditor(page);

  for (const id of TABS) {
    await tab(page, id).click();
    await expectActiveTab(page, id);
    const overflow = await page.evaluate(() => ({
      page: document.documentElement.scrollWidth - document.documentElement.clientWidth,
      bar: (() => {
        const bar = document.querySelector('[data-role="stream-editor-tabs"]') as HTMLElement;
        return bar.scrollWidth - bar.clientWidth;
      })(),
    }));
    expect(overflow.page, `no horizontal page scroll on the ${id} tab`).toBeLessThanOrEqual(0);
    expect(overflow.bar, "the tab bar itself does not overflow").toBeLessThanOrEqual(0);
    for (const t of TABS) {
      const b = await tab(page, t).boundingBox();
      expect(b, `tab ${t} has a box`).toBeTruthy();
      expect(b!.x + b!.width, `tab ${t} fits the 320 px width`).toBeLessThanOrEqual(320);
    }
  }

  expect(errors, `editor console: ${errors.join(" | ")}`).toEqual([]);
});
