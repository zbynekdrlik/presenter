/**
 * #941 — a prepared sermon on a phone. The owner opened a sermon from the
 * operator's Pripravené tab: nothing changed on screen (the slides sat ~1500 px
 * below the list), and once he scrolled to them the page stuck — every touch
 * scroll that started on a slide card moved nothing, up or down, until a reload.
 * Cause: the slide grid kept the desktop's own scroller (`overflow-y: auto` +
 * `overscroll-behavior: contain`) inside the phone's one scrolling column, so it
 * was a scroll container with nothing to scroll that never handed the gesture on.
 */
import { test, expect, type Browser, type CDPSession, type Page } from "@playwright/test";
import {
  attachConsoleErrorCollector,
  deriveTestConfig,
  PHONE_390,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

test.describe.configure({ timeout: 180_000 });

let serverHandle: ServerHandle | undefined;
let baseURL = "";

test.beforeAll(async ({}, testInfo) => {
  const cfg = deriveTestConfig(testInfo);
  baseURL = cfg.baseURL;
  await refreshDevData(cfg.dbUrl);
  serverHandle = await startTestServer(cfg.port, cfg.dbUrl, cfg.oscPort);
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
});

const role = (page: Page, name: string) => page.locator(`[data-role="${name}"]`);

const json = async <T>(path: string, method = "GET", body?: unknown): Promise<T> => {
  const response = await fetch(`${baseURL}${path}`, {
    method,
    headers: body === undefined ? {} : { "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  expect(response.ok, `${method} ${path}: ${response.status}`).toBeTruthy();
  return (await response.json()) as T;
};

/** SNV's shape: a sermon holding 1Sa 4:1-11 (SEB) = 11 slides, under a list of other presentations. */
async function snvLikeData(): Promise<string> {
  for (const name of ["1Sa 4 & Efe 1, 3", "Zd stvrtok", "zbierka", "Zaklady 2", "Zd 25.10", "zaklady 1"]) {
    await json("/bible/presentations", "POST", { name });
  }
  const { id } = await json<{ id: string }>("/bible/sermons", "POST", {
    title: "Kázeň na telefóne",
    serviceDate: "2031-03-09",
    defaultTranslation: "slk-seb",
  });
  const { revision } = await json<{ revision: number }>(`/bible/sermons/${id}`);
  await json(`/bible/sermons/${id}/items`, "POST", {
    expectedRevision: revision,
    translation: "slk-seb",
    bookCode: "1SA",
    chapter: 4,
    verseStart: 1,
    verseEnd: 11,
  });
  const detail = await json<{ slides: unknown[] }>(`/bible/sermons/${id}`);
  expect(detail.slides).toHaveLength(11);
  return id;
}

async function openPhoneOperator(browser: Browser) {
  const context = await browser.newContext({ ...PHONE_390, baseURL });
  const page = await context.newPage();
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  await page.goto("/ui/operator/bible");
  await page.waitForSelector('body[data-wasm-ready="true"]', { state: "attached", timeout: 30_000 });
  await expect(page.locator("body")).toHaveAttribute("data-shell", "phone");
  return { context, page, errors };
}

/** A real finger drag (touch events through the browser's input pipeline). */
async function drag(cdp: CDPSession, x: number, fromY: number, toY: number) {
  await cdp.send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [{ x, y: fromY }] });
  const steps = 20;
  for (let step = 1; step <= steps; step += 1) {
    const y = fromY + ((toY - fromY) * step) / steps;
    await cdp.send("Input.dispatchTouchEvent", { type: "touchMove", touchPoints: [{ x, y }] });
  }
  await cdp.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
}

const panelScroll = (page: Page) =>
  page.locator(".operator__panel--bible").evaluate((panel) => Math.round(panel.scrollTop));

test("opening a sermon on a phone shows its slides, and a finger on them scrolls both ways", async ({
  browser,
}) => {
  const id = await snvLikeData();
  const { context, page, errors } = await openPhoneOperator(browser);
  await page.locator('[data-role="bible-tab"][data-tab="prepared"]').tap();
  await page.locator(`[data-role="presentation-card"][data-presentation-id="${id}"]`).tap();
  await expect(role(page, "sermon-slide-card")).toHaveCount(11);

  // The tap's result is on screen: the slide column starts at the top, under the header.
  await expect
    .poll(async () =>
      role(page, "slides-column").evaluate((column) => {
        const top = Math.round(column.getBoundingClientRect().top);
        return top >= 0 && top < window.innerHeight / 3 ? "on screen" : `top at ${top} px`;
      }),
    )
    .toBe("on screen");
  await expect(role(page, "sermon-badge")).toBeInViewport();

  // A finger on a slide card moves the page down…
  const cdp = await context.newCDPSession(page);
  const card = await role(page, "sermon-slide-card").first().boundingBox();
  expect(card).not.toBeNull();
  const x = Math.round((card?.x ?? 0) + (card?.width ?? 0) / 2);
  const before = await panelScroll(page);
  for (let round = 0; round < 3; round += 1) await drag(cdp, x, 650, 250);
  await expect.poll(() => panelScroll(page)).toBeGreaterThan(before + 600);
  // …and the point under the finger was a slide card the whole way.
  const under = await page.evaluate(
    ([px]) => document.elementFromPoint(px, 650)?.closest('[data-role="sermon-slide-card"]') !== null,
    [x],
  );
  expect(under).toBe(true);

  // …and back up to the list.
  const down = await panelScroll(page);
  for (let round = 0; round < 3; round += 1) await drag(cdp, x, 250, 650);
  await expect.poll(() => panelScroll(page)).toBeLessThan(down - 600);

  // The page still answers a tap.
  await page.locator('[data-role="bible-tab"][data-tab="live"]').tap();
  await expect(page.locator('[data-role="bible-tab"][data-tab="live"]')).toHaveAttribute("data-active", "true");

  expect(errors).toEqual([]);
  await context.close();
  expect((await fetch(`${baseURL}/bible/sermons/${id}`, { method: "DELETE" })).status).toBe(204);
});

test("an ordinary prepared presentation scrolls under a finger on its slides too", async ({ browser }) => {
  const { id } = await json<{ id: string }>("/bible/presentations", "POST", { name: "Dlhá prezentácia" });
  await json(`/bible/presentations/${id}/append`, "POST", {
    slides: Array.from({ length: 8 }, (_, index) => ({
      bibleMain: `${index + 1}. ${"Slovo na stene ".repeat(14)}`,
      bibleTranslation: "",
      bibleMainReference: `Test ${index + 1} (SEB)`,
    })),
  });
  const { context, page, errors } = await openPhoneOperator(browser);
  await page.locator('[data-role="bible-tab"][data-tab="prepared"]').tap();
  await page.locator(`[data-role="presentation-card"][data-presentation-id="${id}"]`).tap();
  await expect(role(page, "slide-card")).toHaveCount(8);
  await expect
    .poll(async () =>
      role(page, "slides-column").evaluate((column) => {
        const top = Math.round(column.getBoundingClientRect().top);
        return top >= 0 && top < window.innerHeight / 3 ? "on screen" : `top at ${top} px`;
      }),
    )
    .toBe("on screen");

  const cdp = await context.newCDPSession(page);
  const card = await role(page, "slide-card").first().boundingBox();
  const x = Math.round((card?.x ?? 0) + (card?.width ?? 0) / 2);
  const before = await panelScroll(page);
  for (let round = 0; round < 3; round += 1) await drag(cdp, x, 650, 250);
  await expect.poll(() => panelScroll(page)).toBeGreaterThan(before + 600);
  expect(errors).toEqual([]);
  await context.close();
});
