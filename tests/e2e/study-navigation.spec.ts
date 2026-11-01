/**
 * #851 S2 — moving around `/ui/study`: swipe, verse selection and Kopírovať,
 * ◀ ▶ with the long-press list, the last position, text size and favourites
 * per device.
 */
import { test, expect, type Page } from "@playwright/test";
import {
  assertMinTapTarget,
  assertNoHorizontalOverflow,
  deriveTestConfig,
  PHONE_360,
  PHONE_390,
  PHONE_LANDSCAPE,
  refreshDevData,
  startTestServer,
  stopServer,
  touchDrag,
  type ServerHandle,
} from "./support";
import {
  chapter,
  expectChapter,
  favourite,
  field,
  finish,
  openStudy,
  scrollToVerse,
  settled,
  verse,
  verseNumber,
} from "./study-support";

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

const back = (page: Page) => page.locator('[data-role="history-back"]');
const forward = (page: Page) => page.locator('[data-role="history-forward"]');
const actions = (page: Page) => page.locator('[data-role="verse-actions"]');
const list = (page: Page) => page.locator('[data-role="history-list"]');

/** A finger on the text, drawn by `dx`, `dy` from the middle of the reader. */
async function drag(page: Page, dx: number, dy: number): Promise<void> {
  const from = { x: 195, y: 420 };
  await touchDrag(page, from, { x: from.x + dx, y: from.y + dy }, 8);
}

/** Press and hold through CDP touch events until `shown` is on screen. */
async function longPress(page: Page, x: number, y: number, shown: () => Promise<void>) {
  await touchDrag(page, { x, y }, { x, y }, 1, shown);
}

async function centre(page: Page, selector: string): Promise<{ x: number; y: number }> {
  const box = await page.locator(selector).boundingBox();
  expect(box).not.toBeNull();
  return { x: (box?.x ?? 0) + (box?.width ?? 0) / 2, y: (box?.y ?? 0) + (box?.height ?? 0) / 2 };
}

test("one swipe turns the chapter, either way", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390);
  const { page } = opened;
  await drag(page, -120, 0);
  await expectChapter(page, "slk-seb:HEB.12");
  await drag(page, 120, 10);
  await expectChapter(page, "slk-seb:HEB.11");
  await finish(opened);
});

test("a short or steep drag turns nothing", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390);
  const { page } = opened;
  const scroll = page.locator('[data-role="study-scroll"]');
  await drag(page, -40, 0);
  await drag(page, -100, -200);
  await expect(chapter(page)).toHaveAttribute("data-loaded-for", "slk-seb:HEB.11");
  await expect(page).toHaveURL(/p=HEB\.11/);
  // The steep one was a scroll, and it scrolled.
  await expect.poll(() => scroll.evaluate((el) => el.scrollTop)).toBeGreaterThan(0);
  await finish(opened);
});

test("verse numbers select, extend, and Back clears", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390);
  const { page } = opened;
  await verseNumber(page, 1).tap();
  await expect(verse(page, 1)).toHaveAttribute("data-selected", "true");
  await expect(actions(page)).toContainText("Hebrejom 11:1 (SEB)");
  await expect(page).toHaveURL(/v=1&s=akcie/);

  await verseNumber(page, 3).tap();
  await expect(actions(page)).toContainText("Hebrejom 11:1-3 (SEB)");
  for (const number of [1, 2, 3]) {
    await expect(verse(page, number)).toHaveAttribute("data-selected", "true");
  }
  await expect(verse(page, 4)).toHaveAttribute("data-selected", "false");

  await page.goBack();
  await expect(actions(page)).toHaveCount(0);
  await expect(verse(page, 1)).toHaveAttribute("data-selected", "false");
  await finish(opened);
});

test("the sheet's ✕ closes it and only its own verse deselects", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390);
  const { page } = opened;
  await verseNumber(page, 2).tap();
  await actions(page).locator('[data-role="study-sheet-close"]').tap();
  await expect(actions(page)).toHaveCount(0);
  await verseNumber(page, 2).tap();
  await expect(actions(page)).toBeVisible();
  await verseNumber(page, 2).tap();
  await expect(actions(page)).toHaveCount(0);
  await finish(opened);
});

test("the sheet offers only what works today", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390);
  const { page } = opened;
  await verseNumber(page, 1).tap();
  await expect(actions(page).locator("button:not([data-role=study-sheet-close])")).toHaveText([
    "Kopírovať",
  ]);
  await finish(opened);
});

test("Kopírovať copies the text with its reference over plain http", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390);
  const { page } = opened;
  // The LAN page has no navigator.clipboard: the copy is execCommand's.
  await page.evaluate(() => {
    const original = document.execCommand.bind(document);
    (window as unknown as { copied: string[] }).copied = [];
    document.execCommand = (command: string) => {
      const active = document.activeElement as HTMLTextAreaElement;
      (window as unknown as { copied: string[] }).copied.push(active.value);
      return command === "copy" ? true : original(command);
    };
  });
  await verseNumber(page, 1).tap();
  await verseNumber(page, 2).tap();
  await page.locator('[data-role="verse-copy"]').tap();
  const copied = await page.evaluate(() => (window as unknown as { copied: string[] }).copied);
  expect(copied).toHaveLength(1);
  expect(copied[0]).toMatch(/^1 Viera .*\n2 .*\nHebrejom 11:1-2 \(SEB\)$/s);
  await expect(page.locator('[data-role="study-notice"]')).toHaveAttribute("data-visible", "true");
  await expect(page.locator('[data-role="study-notice"]')).toHaveText("Skopírované");
  await expect(actions(page)).toHaveCount(0);
  await finish(opened);
});

test("◀ ▶ step through the history; ▶ ends at the newest place", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390);
  const { page } = opened;
  await expect(back(page)).toHaveAttribute("aria-disabled", "true");
  await expect(forward(page)).toBeDisabled();

  await page.locator('[data-role="study-next-chapter"]').tap();
  await expectChapter(page, "slk-seb:HEB.12");
  await expect(back(page)).toHaveAttribute("aria-disabled", "false");

  await back(page).tap();
  await expectChapter(page, "slk-seb:HEB.11");
  await expect(forward(page)).toBeEnabled();

  await forward(page).tap();
  await expectChapter(page, "slk-seb:HEB.12");
  await expect(forward(page)).toBeDisabled();
  await finish(opened);
});

test("Back returns to the place the reader had scrolled to", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390);
  const { page } = opened;
  const scroll = page.locator('[data-role="study-scroll"]');
  await scroll.evaluate((el) => (el.scrollTop = 900));
  await page.locator('[data-role="study-next-chapter"]').tap();
  await expectChapter(page, "slk-seb:HEB.12");
  await expect.poll(() => scroll.evaluate((el) => el.scrollTop)).toBe(0);
  await back(page).tap();
  await expectChapter(page, "slk-seb:HEB.11");
  await expect.poll(() => scroll.evaluate((el) => el.scrollTop)).toBe(900);
  await finish(opened);
});

test("a closed sheet lights no ▶ and a chapter step leaves no sheet behind", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390);
  const { page } = opened;
  await verseNumber(page, 1).tap();
  await page.goBack();
  await expect(actions(page)).toHaveCount(0);
  await expect(forward(page)).toBeDisabled();

  // The sheet covers the bar on a phone; a swipe on the text still turns.
  await verseNumber(page, 1).tap();
  await drag(page, -120, 0);
  await expectChapter(page, "slk-seb:HEB.12");
  await expect(actions(page)).toHaveCount(0);
  const listed = await page.evaluate(() => {
    const stored = sessionStorage.getItem("presenter:study.history") ?? '""';
    const history = JSON.parse(JSON.parse(stored)) as { entries: { label: string; listed: boolean }[] };
    return history.entries.filter((entry) => entry.listed).map((entry) => entry.label);
  });
  expect(listed).toEqual(["Hebrejom 11 · SEB", "Hebrejom 12 · SEB"]);
  // The system gesture: Chrome swallows the click of a tap right after a swipe.
  await page.goBack();
  await expectChapter(page, "slk-seb:HEB.11");
  await expect(actions(page)).toHaveCount(0);
  await finish(opened);
});

test("Back with no stored offset goes to the place's verse", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390);
  const { page } = opened;
  await field(page).tap();
  await field(page).fill("zid 12 20");
  await field(page).press("Enter");
  await expectChapter(page, "slk-seb:HEB.12");
  await field(page).tap();
  await field(page).fill("zid 13");
  await field(page).press("Enter");
  await expectChapter(page, "slk-seb:HEB.13");
  // A reload forgets the offsets in memory.
  await page.reload();
  await expectChapter(page, "slk-seb:HEB.13");
  await page.goBack();
  await expectChapter(page, "slk-seb:HEB.12");
  await expect(verse(page, 20)).toBeInViewport();
  await page.goBack();
  await expectChapter(page, "slk-seb:HEB.11");
  await expect.poll(() =>
    page.locator('[data-role="study-scroll"]').evaluate((el) => el.scrollTop),
  ).toBe(0);
  await finish(opened);
});

test("a reload keeps the place in the chapter", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390);
  const { page } = opened;
  await scrollToVerse(page, 25);
  await page.waitForFunction(() =>
    (localStorage.getItem("presenter:study.lastVerse") ?? "").includes("|25"),
  );
  await page.reload();
  await expectChapter(page, "slk-seb:HEB.11");
  await expect(verse(page, 25)).toBeInViewport();
  await finish(opened);
});

test("a reopen keeps the place even when the last URL had a verse", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390, "/ui/study?t=slk-seb&p=HEB.11&v=1");
  const { page } = opened;
  await scrollToVerse(page, 25);
  await page.waitForFunction(() =>
    (localStorage.getItem("presenter:study.lastVerse") ?? "").includes("|25"),
  );
  await page.goto("/ui/study");
  await expectChapter(page, "slk-seb:HEB.11");
  await expect(verse(page, 25)).toBeInViewport();
  await finish(opened);
});

test("a link's verse wins over the verse saved for its chapter", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390);
  const { page } = opened;
  await scrollToVerse(page, 25);
  await page.waitForFunction(() =>
    (localStorage.getItem("presenter:study.lastVerse") ?? "").includes("|25"),
  );
  await page.goto("/ui/study?t=slk-seb&p=HEB.11&v=3");
  await expectChapter(page, "slk-seb:HEB.11");
  await expect(verse(page, 3)).toBeInViewport();
  await finish(opened);
});

test("coming back through a link starts a new list: ◀ and ✕ never leave the page", async ({
  browser,
}) => {
  const opened = await openStudy(browser, baseURL, PHONE_390);
  const { page } = opened;
  await page.locator('[data-role="study-next-chapter"]').tap();
  await expectChapter(page, "slk-seb:HEB.12");

  await page.goto("/ui/operator");
  await page.waitForSelector('body[data-wasm-ready="true"]', { state: "attached" });
  await page.goto("/ui/study");
  await expectChapter(page, "slk-seb:HEB.12");

  await expect(back(page)).toHaveAttribute("aria-disabled", "true");
  await back(page).dispatchEvent("click");
  await settled(page);
  await expect(page).toHaveURL(/\/ui\/study/);
  await page.locator('[data-role="study-text-size"]').tap();
  await page.locator('[data-role="study-text-sheet"] [data-role="study-sheet-close"]').tap();
  await expect(page.locator('[data-role="study-text-sheet"]')).toHaveCount(0);
  await expect(page).toHaveURL(/\/ui\/study/);

  // The browser's own Back to the old list leaves ◀ on the new one.
  await page.goBack();
  await page.waitForSelector('[data-role="surface-nav"]', { state: "attached" });
  await page.goBack();
  await expectChapter(page, "slk-seb:HEB.12");
  await expect(back(page)).toHaveAttribute("aria-disabled", "true");
  // The operator polls the stage and warns when left mid-save: its noise, not this page's.
  opened.wallRequests.length = 0;
  const left = opened.errors.filter((text) => !/auto-save preferences failed/.test(text));
  opened.errors.splice(0, opened.errors.length, ...left);
  await finish(opened);
});

test("a sheet that came with the URL closes in place, without its verse", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390, "/ui/study?t=slk-seb&p=HEB.11&v=2&s=akcie");
  const { page } = opened;
  await expect(actions(page)).toBeVisible();
  await actions(page).locator('[data-role="study-sheet-close"]').tap();
  await expect(actions(page)).toHaveCount(0);
  await expect(page).toHaveURL(/\/ui\/study\?t=slk-seb&p=HEB\.11$/);
  await expect(verse(page, 2)).toHaveAttribute("data-selected", "false");
  await expect(page.locator('[data-selected="true"]')).toHaveCount(0);

  await page.goto("/ui/study?t=slk-seb&p=HEB.11&s=akcie");
  await expectChapter(page, "slk-seb:HEB.11");
  await expect(actions(page)).toHaveCount(0);
  await finish(opened);
});

test("a long press on ◀ lists the places and a row goes there", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390);
  const { page } = opened;
  await page.locator('[data-role="study-next-chapter"]').tap();
  await expectChapter(page, "slk-seb:HEB.12");
  await page.locator('[data-role="study-next-chapter"]').tap();
  await expectChapter(page, "slk-seb:HEB.13");

  const { x, y } = await centre(page, '[data-role="history-back"]');
  await longPress(page, x, y, async () => {
    await expect(list(page)).toBeVisible();
  });
  await expect(list(page).locator('[data-role="history-entry"]')).toHaveText([
    "Hebrejom 12 · SEB",
    "Hebrejom 11 · SEB",
  ]);
  await expectChapter(page, "slk-seb:HEB.13");
  await assertMinTapTarget(list(page).locator('[data-role="history-entry"]'), 44);

  await list(page).locator('[data-role="history-entry"]').nth(1).tap();
  await expectChapter(page, "slk-seb:HEB.11");
  await expect(list(page)).toHaveCount(0);
  await finish(opened);
});

test("a tap on ◀ never opens the list", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390);
  const { page } = opened;
  await page.locator('[data-role="study-next-chapter"]').tap();
  await expectChapter(page, "slk-seb:HEB.12");
  await back(page).tap();
  await expectChapter(page, "slk-seb:HEB.11");
  await expect(list(page)).toHaveCount(0);
  await finish(opened);
});

test("the backdrop closes a list", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390);
  const { page } = opened;
  await page.locator('[data-role="study-text-size"]').tap();
  await expect(page.locator('[data-role="study-text-sheet"]')).toBeVisible();
  await page.locator('[data-role="study-backdrop"]').tap({ position: { x: 100, y: 100 } });
  await expect(page.locator('[data-role="study-text-sheet"]')).toHaveCount(0);
  await finish(opened);
});

test("a reload and a reopen come back to the last position", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390);
  const { page } = opened;
  await favourite(page, "eng-kjv").tap();
  await expectChapter(page, "eng-kjv:HEB.11");
  await page.locator('[data-role="study-next-chapter"]').tap();
  await expectChapter(page, "eng-kjv:HEB.12");
  await page.reload();
  await expectChapter(page, "eng-kjv:HEB.12");
  await expect(favourite(page, "eng-kjv")).toHaveAttribute("aria-pressed", "true");
  await expect(back(page)).toHaveAttribute("aria-disabled", "false");

  const scroll = page.locator('[data-role="study-scroll"]');
  await scroll.evaluate((el) => {
    const target = el.querySelector('[data-verse="20"]') as HTMLElement;
    el.scrollTop += target.getBoundingClientRect().top - el.getBoundingClientRect().top;
  });
  await page.waitForFunction(() =>
    (localStorage.getItem("presenter:study.lastVerse") ?? "").includes("|20"),
  );
  await page.goto("/ui/study");
  await expectChapter(page, "eng-kjv:HEB.12");
  await expect(verse(page, 20)).toBeInViewport();
  await finish(opened);
});

test("the text size has 3 steps and stays on this device", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390);
  const { page } = opened;
  const text = page.locator(".study-verse__text").first();
  const size = () => text.evaluate((el) => parseFloat(getComputedStyle(el).fontSize));
  const medium = await size();
  await page.locator('[data-role="study-text-size"]').tap();
  const sheet = page.locator('[data-role="study-text-sheet"]');
  await assertMinTapTarget(sheet.locator('[data-role="study-text-option"]'), 44);
  await sheet.locator('[data-size="large"]').tap();
  await expect.poll(size).toBeGreaterThan(medium);
  await expect(sheet.locator('[data-size="large"]')).toHaveAttribute("aria-pressed", "true");
  await sheet.locator('[data-size="small"]').tap();
  await expect.poll(size).toBeLessThan(medium);
  await sheet.locator('[data-size="large"]').tap();
  const large = await size();

  await page.reload();
  await expectChapter(page, "slk-seb:HEB.11");
  await expect.poll(size).toBe(large);
  await finish(opened);
});

test("a long press on a chip changes that slot, a tap does not", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390);
  const { page } = opened;
  const sheet = page.locator('[data-role="study-fav-sheet"]');
  const { x, y } = await centre(page, '[data-role="study-fav"][data-translation="slk-sevp"]');
  await longPress(page, x, y, async () => {
    await expect(sheet).toBeVisible();
  });
  await assertMinTapTarget(sheet.locator('[data-role="study-fav-option"]'), 44);
  await expect(sheet.locator('[data-translation="slk-mil"]')).toBeVisible();
  // The press did not switch the translation.
  await expectChapter(page, "slk-seb:HEB.11");

  await sheet.locator('[data-translation="slk-mil"]').tap();
  await expect(sheet).toHaveCount(0);
  await expect(favourite(page, "slk-mil")).toBeVisible();
  await expect(favourite(page, "slk-sevp")).toHaveCount(0);
  await expect(page.locator('[data-role="study-fav"]')).toHaveCount(5);

  await page.reload();
  await expectChapter(page, "slk-seb:HEB.11");
  await expect(page.locator('[data-role="study-fav"]').nth(2)).toHaveAttribute(
    "data-translation",
    "slk-mil",
  );
  await finish(opened);
});

for (const [name, profile] of [
  ["360", PHONE_360],
  ["390", PHONE_390],
  ["landscape", PHONE_LANDSCAPE],
] as const) {
  test(`every sheet fits a phone: ${name}`, async ({ browser }) => {
    const opened = await openStudy(browser, baseURL, profile);
    const { page } = opened;
    await verseNumber(page, 1).tap();
    await expect(actions(page)).toBeVisible();
    await assertNoHorizontalOverflow(page);
    await assertMinTapTarget(page.locator('[data-role="verse-copy"]'), 44);
    await assertMinTapTarget(page.locator('[data-role="study-sheet-close"]'), 44);
    await page.goBack();
    await expect(actions(page)).toHaveCount(0);

    await page.locator('[data-role="study-text-size"]').tap();
    await assertNoHorizontalOverflow(page);
    await expect(field(page)).toBeVisible();
    await finish(opened);
  });
}
