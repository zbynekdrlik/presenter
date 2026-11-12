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
  // SNV's limit splits 1Sa 4:1-11 into one slide per verse.
  const prefs = await fetch(`${baseURL}/bible/preferences`, {
    method: "PUT",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ mainTranslation: "slk-seb", characterLimit: 150 }),
  });
  expect(prefs.ok).toBeTruthy();
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

/** The slide column's top, as the owner would see it. */
const slidesOnScreen = (page: Page) =>
  role(page, "slides-column").evaluate((column) => {
    const top = Math.round(column.getBoundingClientRect().top);
    return top >= 0 && top < window.innerHeight / 3 ? "on screen" : `top at ${top} px`;
  });

async function openSermon(browser: Browser, id: string) {
  const opened = await openPhoneOperator(browser);
  await opened.page.locator('[data-role="bible-tab"][data-tab="prepared"]').tap();
  await opened.page.locator(`[data-role="presentation-card"][data-presentation-id="${id}"]`).tap();
  await expect(role(opened.page, "sermon-slide-card")).toHaveCount(11);
  return opened;
}

test("opening a sermon on a phone brings its slides on screen", async ({ browser }) => {
  const id = await snvLikeData();
  const { context, page, errors } = await openSermon(browser, id);
  await expect.poll(() => slidesOnScreen(page)).toBe("on screen");
  await expect(role(page, "sermon-badge")).toBeInViewport();
  await expect(role(page, "sermon-slide-card").first()).toBeInViewport();
  expect(errors).toEqual([]);
  await context.close();
  expect((await fetch(`${baseURL}/bible/sermons/${id}`, { method: "DELETE" })).status).toBe(204);
});

test("a finger on a sermon's slides scrolls the page both ways and the page still answers", async ({
  browser,
}) => {
  const id = await snvLikeData();
  const { context, page, errors } = await openSermon(browser, id);
  // The cause: the grid kept its own scroller inside the phone's one scrolling
  // column. Chrome on SNV's data (taller cards with the NLT line) latched the
  // finger onto that zero-room scroller and `contain` never handed the scroll
  // on (scrollTop stuck at 880 of 5870); this suite's Chromium chains it anyway
  // on the test data, so the cause itself is pinned here as well.
  const grid = await role(page, "slides")
    .filter({ has: role(page, "sermon-slide-card") })
    .evaluate((element) => {
      const style = getComputedStyle(element);
      return `${style.overflowY} ${style.overscrollBehaviorY}`;
    });
  expect(grid, "the phone's slide grid must not be a scroller of its own").toBe("visible auto");
  // Wherever the open left the page, put the first slide card under the finger.
  await page.locator(".operator__panel--bible").evaluate((panel) => {
    const card = panel.querySelector('[data-role="sermon-slide-card"]');
    if (card) panel.scrollTop += card.getBoundingClientRect().top - 100;
  });
  const card = await role(page, "sermon-slide-card").first().boundingBox();
  expect(card).not.toBeNull();
  const x = Math.round((card?.x ?? 0) + (card?.width ?? 0) / 2);
  const onCard = (y: number) =>
    page.evaluate(
      ([px, py]) => document.elementFromPoint(px, py)?.closest('[data-role="sermon-slide-card"]') !== null,
      [x, y],
    );
  expect(await onCard(650)).toBe(true);

  // No long task blocks the page while the finger works (the owner saw a
  // "frozen" page; on SNV it was the scroll trap, never a busy main thread).
  await page.evaluate(() => {
    const seen: number[] = [];
    (window as unknown as { __longTasks: number[] }).__longTasks = seen;
    new PerformanceObserver((list) => {
      for (const entry of list.getEntries()) seen.push(Math.round(entry.duration));
    }).observe({ type: "longtask" });
  });

  // Down through the slides…
  const cdp = await context.newCDPSession(page);
  const before = await panelScroll(page);
  for (let round = 0; round < 3; round += 1) await drag(cdp, x, 650, 250);
  await expect.poll(() => panelScroll(page)).toBeGreaterThan(before + 600);

  // …and back up, the finger again on a slide card.
  expect(await onCard(250)).toBe(true);
  const down = await panelScroll(page);
  for (let round = 0; round < 3; round += 1) await drag(cdp, x, 250, 650);
  await expect.poll(() => panelScroll(page)).toBeLessThan(down - 600);

  // The page still answers a tap.
  await page.locator('[data-role="bible-tab"][data-tab="live"]').tap();
  await expect(page.locator('[data-role="bible-tab"][data-tab="live"]')).toHaveAttribute("data-active", "true");
  const longTasks = await page.evaluate(() => (window as unknown as { __longTasks: number[] }).__longTasks);
  expect(longTasks.filter((ms) => ms > 500), "a task over 500 ms blocked the page").toEqual([]);

  expect(errors).toEqual([]);
  await context.close();
  expect((await fetch(`${baseURL}/bible/sermons/${id}`, { method: "DELETE" })).status).toBe(204);
});


/** A plain presentation of eight long slides. */
async function longPresentation(name: string): Promise<string> {
  const { id } = await json<{ id: string }>("/bible/presentations", "POST", { name });
  await json(`/bible/presentations/${id}/append`, "POST", {
    slides: Array.from({ length: 8 }, (_, index) => ({
      bibleMain: `${index + 1}. ${"Slovo na stene ".repeat(14)}`,
      bibleTranslation: "",
      bibleMainReference: `Test ${index + 1} (SEB)`,
    })),
  });
  return id;
}

test("an ordinary prepared presentation: slides on screen, no inner scroller, a finger scrolls", async ({
  browser,
}) => {
  const id = await longPresentation("Dlhá prezentácia");
  const { context, page, errors } = await openPhoneOperator(browser);
  await page.locator('[data-role="bible-tab"][data-tab="prepared"]').tap();
  await page.locator(`[data-role="presentation-card"][data-presentation-id="${id}"]`).tap();
  await expect(role(page, "slide-card")).toHaveCount(8);
  await expect.poll(() => slidesOnScreen(page)).toBe("on screen");
  const grid = await role(page, "slides")
    .filter({ has: role(page, "slide-card") })
    .evaluate((element) => {
      const style = getComputedStyle(element);
      return `${style.overflowY} ${style.overscrollBehaviorY}`;
    });
  expect(grid, "the phone's slide grid must not be a scroller of its own").toBe("visible auto");

  const cdp = await context.newCDPSession(page);
  const card = await role(page, "slide-card").first().boundingBox();
  const x = Math.round((card?.x ?? 0) + (card?.width ?? 0) / 2);
  const before = await panelScroll(page);
  for (let round = 0; round < 3; round += 1) await drag(cdp, x, 650, 250);
  await expect.poll(() => panelScroll(page)).toBeGreaterThan(before + 600);
  expect(errors).toEqual([]);
  await context.close();
});

/** Two animation frames: a scroll planned for the next frame has landed. */
const twoFrames = (page: Page) =>
  page.evaluate(() => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))));

test("the open never moves the page once the operator left Pripravené, nor on a desktop", async ({
  browser,
}) => {
  const id = await longPresentation("Pomalá prezentácia");
  // A phone: Naživo tapped while the presentation is still loading.
  const { context, page, errors } = await openPhoneOperator(browser);
  let release: () => void = () => undefined;
  const held = new Promise<void>((resolve) => {
    release = resolve;
  });
  await page.route(`**/bible/presentations/${id}`, async (route) => {
    if (route.request().method() === "GET") await held;
    await route.continue();
  });
  await page.locator('[data-role="bible-tab"][data-tab="prepared"]').tap();
  const answered = page.waitForResponse((response) => response.url().endsWith(`/bible/presentations/${id}`));
  await page.locator(`[data-role="presentation-card"][data-presentation-id="${id}"]`).tap();
  await page.locator('[data-role="bible-tab"][data-tab="live"]').tap();
  await expect(page.locator('[data-role="bible-tab"][data-tab="live"]')).toHaveAttribute("data-active", "true");
  const before = await panelScroll(page);
  release();
  await answered;
  await twoFrames(page);
  expect(await panelScroll(page)).toBe(before);
  expect(errors).toEqual([]);
  await context.close();

  // A desktop: the panel never scrolls, the slide column keeps its own place.
  const desktop = await browser.newContext({ viewport: { width: 1280, height: 800 }, baseURL });
  const wide = await desktop.newPage();
  const wideErrors: string[] = [];
  attachConsoleErrorCollector(wide, wideErrors);
  await wide.goto("/ui/operator/bible");
  await wide.waitForSelector('body[data-wasm-ready="true"]', { state: "attached", timeout: 30_000 });
  await wide.locator('[data-role="bible-tab"][data-tab="prepared"]').click();
  await wide.locator(`[data-role="presentation-card"][data-presentation-id="${id}"]`).click();
  await expect(role(wide, "slide-card")).toHaveCount(8);
  await twoFrames(wide);
  expect(await panelScroll(wide)).toBe(0);
  expect(wideErrors).toEqual([]);
  await desktop.close();
});
