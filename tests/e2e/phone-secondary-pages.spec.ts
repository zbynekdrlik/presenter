import { test, expect, type Page } from "@playwright/test";
import {
  attachConsoleErrorCollector,
  deriveTestConfig,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

// #850 spec §14: the Bible Tablet and the Stream editor on a phone. The sidebar
// took 2/3 of the width, the header ran off the screen, Clear was 29 px.
test.describe.configure({ timeout: 120_000 });
test.use({ viewport: { width: 390, height: 844 }, hasTouch: true });

let serverHandle: ServerHandle | undefined;
let baseURL: string;

test.beforeAll(async ({}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  baseURL = config.baseURL;
  serverHandle = await startTestServer(config.port, config.dbUrl, config.oscPort);
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
});

/** The right edge of every visible control under `selector`, past the viewport. */
function offScreen(page: Page, selector: string) {
  return page.evaluate((sel) => {
    return [...document.querySelectorAll<HTMLElement>(`${sel} button, ${sel} input, ${sel} select`)]
      .filter((el) => el.getClientRects().length > 0)
      .filter((el) => el.getBoundingClientRect().right > window.innerWidth + 0.5)
      .map((el) => el.dataset.role ?? el.tagName);
  }, selector);
}

/** Visible buttons, selects and text fields under 44 px high. Checkboxes have their own rule. */
function smallTargets(page: Page, selector: string) {
  return page.evaluate((sel) => {
    const own = ["checkbox", "radio", "range", "color", "file", "hidden"];
    return [...document.querySelectorAll<HTMLElement>(`${sel} button, ${sel} select, ${sel} input, ${sel} a`)]
      .filter((el) => el.getClientRects().length > 0)
      .filter((el) => !(el instanceof HTMLInputElement && own.includes(el.type)))
      .filter((el) => el.getBoundingClientRect().height < 43.5)
      .map((el) => `${el.dataset.role ?? el.tagName} ${el.getBoundingClientRect().height.toFixed(0)}`);
  }, selector);
}

test("the Bible Tablet on a phone: the list takes the screen, a pick opens the slides, Back returns", async ({
  page,
  request,
}) => {
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  const name = `Telefón ${Date.now()}`;
  const created = await request.post(new URL("/bible/presentations", baseURL).toString(), {
    data: { name },
  });
  expect(created.ok()).toBeTruthy();

  await page.goto(new URL("/ui/tablet", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]');
  await expect(page.locator('[data-role="presentation-list"]')).toContainText(name);

  const sidebar = page.locator(".tablet-sidebar");
  expect((await sidebar.boundingBox())!.width).toBe(390);
  expect(await offScreen(page, ".tablet-header")).toEqual([]);
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(390);
  expect(await smallTargets(page, "body.tablet")).toEqual([]);
  // The close button sits above the list, not on its first row.
  const close = (await page.locator('[data-role="sidebar-close"]').boundingBox())!;
  const firstRow = (await page.locator('[data-role="presentation-button"]').first().boundingBox())!;
  expect(close.y + close.height).toBeLessThanOrEqual(firstRow.y);

  await page.locator('[data-role="presentation-button"]', { hasText: name }).click();
  await expect
    .poll(async () => (await page.locator(".tablet-main").boundingBox())!.width)
    .toBe(390);
  const back = page.locator('[data-role="sidebar-toggle"]');
  await expect(back).toBeVisible();
  expect(await smallTargets(page, "body.tablet")).toEqual([]);

  await back.click();
  await expect.poll(async () => (await sidebar.boundingBox())!.width).toBe(390);
  expect(errors).toEqual([]);
});

test("a tablet held wide keeps its sidebar next to the slides", async ({ page }) => {
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  await page.setViewportSize({ width: 1024, height: 768 });
  await page.goto(new URL("/ui/tablet", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]');
  expect((await page.locator(".tablet-sidebar").boundingBox())!.width).toBe(260);
  expect((await page.locator(".tablet-main").boundingBox())!.width).toBe(764);
  expect(errors).toEqual([]);
});

test("the Stream editor on a phone: 44 px targets and checkboxes a finger can hit", async ({ page }) => {
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  // A fresh database has no scenes: without them the cards are not under test.
  const stamp = Date.now();
  for (const kind of ["base", "overlay"]) {
    const scene = await page.request.post(new URL("/stream/api/outputs/stream/scenes", baseURL).toString(), {
      data: { name: `Telefón ${kind} ${stamp}`, kind },
    });
    expect(scene.ok()).toBeTruthy();
  }
  await page.goto(new URL("/ui/stream", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]');
  await expect(page.locator('[data-role="stream-output-select"]')).toBeVisible();
  await expect(page.locator('[data-role="stream-scene"]')).toHaveCount(2);

  // A card takes the width of its list: no empty third of the screen beside it.
  const narrowCards = await page.evaluate(() =>
    [...document.querySelectorAll<HTMLElement>('[data-role="stream-scene"]')]
      .map((el) => ({
        card: el.getBoundingClientRect().width,
        list: el.parentElement!.getBoundingClientRect().width,
      }))
      .filter((w) => w.card < w.list - 0.5),
  );
  expect(narrowCards).toEqual([]);
  expect(await smallTargets(page, "body")).toEqual([]);
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(390);

  const boxes = await page.evaluate(() =>
    [...document.querySelectorAll<HTMLInputElement>('input[type="checkbox"]')]
      .filter((el) => el.getClientRects().length > 0)
      .map((el) => {
        const box = el.getBoundingClientRect();
        const label = el.closest("label")?.getBoundingClientRect();
        return { w: box.width, labelH: label?.height ?? 0 };
      }),
  );
  expect(boxes.length).toBeGreaterThan(0);
  expect(boxes.filter((b) => b.w < 20 || b.labelH < 43.5)).toEqual([]);
  expect(errors).toEqual([]);
});
