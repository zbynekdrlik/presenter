import { test, expect, type Page } from "@playwright/test";
import {
  attachConsoleErrorCollector,
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

// #738 regression: the /ui/stream editor property panel could not be scrolled
// at a normal desktop viewport. tablet.css ships a bare global
// `html { height:100%; overflow:hidden }` (its iOS-bounce guard) that trunk
// bundles into every page; the editor's element property form legitimately
// exceeds the viewport, so the page root clipped everything past the fold and
// the user could not reach — let alone edit — the bottom controls. Owner:
// "teraz neviem ani skrolnut dole a upravit parametre". This test drives a
// REAL wheel scroll (not Playwright's programmatic scrollIntoView, which works
// even on an overflow:hidden root) so it genuinely fails on the bug.

let serverHandle: ServerHandle | undefined;
let baseURL: string;

type ElementDef = { id: number; props: Record<string, any> & { kind: string } };
type SceneDef = { id: number; name: string; kind: string; elements: ElementDef[] };
type OutputDef = { scenes: SceneDef[] };

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

async function openEditor(page: Page) {
  await page.goto(`${baseURL}/ui/stream`);
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  await page.waitForSelector('[data-role="stream-editor"]', { timeout: 30_000 });
}

async function getDef(page: Page): Promise<OutputDef> {
  const res = await page.request.get(
    new URL("/stream/api/outputs/stream/def", baseURL).toString(),
    { timeout: 30_000 },
  );
  expect(res.ok()).toBeTruthy();
  return (await res.json()) as OutputDef;
}

async function getScene(page: Page, sceneId: string): Promise<SceneDef> {
  const scene = (await getDef(page)).scenes.find((s) => String(s.id) === sceneId);
  expect(scene, `scene ${sceneId} present`).toBeTruthy();
  return scene as SceneDef;
}

async function addScene(page: Page, name: string): Promise<string> {
  await page.locator('[data-role="stream-add-name"]').fill(name);
  await page.locator('[data-role="stream-add-kind"]').selectOption("base");
  await page.locator('[data-role="stream-add-submit"]').click();
  const card = page
    .locator('[data-role="stream-scene"]')
    .filter({ has: page.getByText(name, { exact: true }) });
  await expect(card).toHaveCount(1, { timeout: 15_000 });
  const id = await card.getAttribute("data-scene-id");
  expect(id, "new scene has a data-scene-id").toBeTruthy();
  return id as string;
}

async function openPanel(page: Page, sceneId: string) {
  await page
    .locator(`[data-role="stream-scene"][data-scene-id="${sceneId}"] [data-role="stream-scene-edit"]`)
    .click();
  await page.waitForSelector('[data-role="stream-element-panel"]', { timeout: 15_000 });
}

async function addVerseElement(page: Page, sceneId: string): Promise<string> {
  const before = (await getScene(page, sceneId)).elements.map((e) => e.id);
  await page.locator('[data-role="stream-add-element-verse"]').click();
  let newId = "";
  await expect
    .poll(async () => {
      const added = (await getScene(page, sceneId)).elements.find((e) => !before.includes(e.id));
      if (added) {
        newId = String(added.id);
        return true;
      }
      return false;
    })
    .toBe(true);
  return newId;
}

test("property panel is scrollable: the last control is reachable + editable at a desktop viewport (#738)", async ({
  page,
}) => {
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);

  await page.setViewportSize({ width: 1280, height: 720 });
  await openEditor(page);

  const scene = await addScene(page, "SE_ScrollBug");
  await openPanel(page, scene);
  // A verse element renders the TALLEST property form (three TextStyle groups
  // + frame + transition) — guarantees the form overflows a 720px viewport.
  const verse = await addVerseElement(page, scene);
  await page
    .locator(`[data-role="stream-element"][data-element-id="${verse}"] [data-role="stream-element-select"]`)
    .click();
  await page.waitForSelector('[data-role="stream-prop-form"]', { timeout: 10_000 });

  // The form MUST exceed the viewport, or this test proves nothing.
  const overflows = await page.evaluate(
    () => document.documentElement.scrollHeight > window.innerHeight + 100,
  );
  expect(overflows, "verse property form must exceed a 720px viewport").toBe(true);

  // BUG #738: the page root must NOT clip its overflow — the user has to be able
  // to scroll to the bottom controls. A bare global `html{overflow:hidden}` from
  // tablet.css leaking onto /ui/stream is exactly what broke this.
  const rootOverflowY = await page.evaluate(
    () => getComputedStyle(document.documentElement).overflowY,
  );
  expect(rootOverflowY, "page root must allow scrolling, not clip with overflow:hidden").not.toBe(
    "hidden",
  );

  // Functional proof: a NORMAL wheel scroll (not programmatic scrollIntoView,
  // which works even on an overflow:hidden root) brings the bottom-most control
  // (the Save button) into the viewport.
  await page.evaluate(() => window.scrollTo(0, 0));
  await page.mouse.move(640, 360);
  await page.mouse.wheel(0, 4000);
  await page.waitForTimeout(250);
  const saveReachable = await page
    .locator('[data-role="stream-prop-save"]')
    .evaluate((el) => el.getBoundingClientRect().bottom <= window.innerHeight + 1);
  expect(saveReachable, "after a wheel scroll the Save button is within the viewport").toBe(true);

  // …and the last parameter is actually editable end-to-end: set the crossfade
  // duration, save, and confirm the def persisted it.
  await page.locator('[data-role="stream-transition-fade"]').check();
  await page.locator('[data-role="stream-transition-ms"]').fill("1234");
  await page.locator('[data-role="stream-prop-save"]').click();
  await expect
    .poll(async () => {
      const el = (await getScene(page, scene)).elements.find((e) => String(e.id) === verse);
      return el?.props?.content_transition?.duration_ms;
    })
    .toBe(1234);

  expect(errors, "browser console must be clean").toEqual([]);
});
