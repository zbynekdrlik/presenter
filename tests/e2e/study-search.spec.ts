/**
 * #851 S3 — the one field on `/ui/study`: any way of writing a reference opens
 * the reader with one field and Enter (T4), a book offers its chapters, words
 * search the text. The server search has no scope or phrase yet (D4), so the
 * page offers neither.
 */
import { test, expect, type Page } from "@playwright/test";
import {
  assertMinFontSize,
  assertMinTapTarget,
  assertNoHorizontalOverflow,
  deriveTestConfig,
  PHONE_360,
  PHONE_390,
  PHONE_LANDSCAPE,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";
import {
  expectChapter,
  favourite,
  field,
  finish,
  gateRoute,
  landed,
  openStudy,
  settled,
  verse,
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

const results = (page: Page) => page.locator('[data-role="search-results"]');
const hits = (page: Page) => page.locator('[data-role="search-hit"]');
const suggest = (page: Page) => page.locator('[data-role="study-suggest"]');

test("zid 11 1 + Enter opens the reader at the verse, highlighted", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390, "/ui/study?t=slk-seb&p=GEN.1", "slk-seb:GEN.1");
  const { page } = opened;
  await field(page).tap();
  await field(page).fill("zid 11 1");
  await expect(page.locator('[data-role="study-row"]')).toContainText("Hebrejom 11:1");
  await field(page).press("Enter");
  await expectChapter(page, "slk-seb:HEB.11");
  await expect(page).toHaveURL(/t=slk-seb&p=HEB\.11&v=1$/);
  await expect(verse(page, 1)).toHaveAttribute("data-selected", "true");
  await expect(verse(page, 2)).toHaveAttribute("data-selected", "false");
  await expect(field(page)).toHaveValue("");
  await expect(suggest(page)).toHaveCount(0);
  await finish(opened);
});

test("a verse deep in a chapter is scrolled into view", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390);
  const { page } = opened;
  await field(page).tap();
  await field(page).fill("zid 11 35");
  await field(page).press("Enter");
  await expect(verse(page, 35)).toHaveAttribute("data-selected", "true");
  await expect(verse(page, 35)).toBeInViewport();
  await finish(opened);
});

test("the same reference in the open chapter moves to the verse", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390);
  const { page } = opened;
  await field(page).tap();
  await field(page).fill("zid 11 30-32");
  await field(page).press("Enter");
  for (const number of [30, 31, 32]) {
    await expect(verse(page, number)).toHaveAttribute("data-selected", "true");
  }
  await expect(page).toHaveURL(/v=30-32$/);
  await expect(verse(page, 30)).toBeInViewport();
  await finish(opened);
});

test("tapping the row works like Enter", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390);
  const { page } = opened;
  await field(page).tap();
  await field(page).fill("zid 11 3");
  await page.locator('[data-role="study-row"]').tap();
  await expect(verse(page, 3)).toHaveAttribute("data-selected", "true");
  await finish(opened);
});

test("an ambiguous name opens the default and offers the other book", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390);
  const { page } = opened;
  await field(page).tap();
  await field(page).fill("jan 3 16");
  const chip = page.locator('[data-role="study-chip"][data-kind="alternative"]');
  await expect(chip).toHaveText("1 Ján");
  await assertMinTapTarget(chip, 44);
  await chip.tap();
  await expectChapter(page, "slk-seb:1JN.3");
  await expect(verse(page, 16)).toHaveAttribute("data-selected", "true");
  await finish(opened);
});

test("a reference that does not exist says why and offers the fix", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390);
  const { page } = opened;
  await field(page).tap();
  await field(page).fill("zid 14");
  await expect(page.locator('[data-role="study-hint"]')).toHaveText("Kniha má len 13 kapitol");
  await field(page).press("Enter");
  await expectChapter(page, "slk-seb:HEB.11");
  await page.locator('[data-role="study-chip"][data-kind="fix"]').tap();
  await expect(field(page)).toHaveValue("Hebrejom 13");
  await field(page).press("Enter");
  await expectChapter(page, "slk-seb:HEB.13");
  await finish(opened);
});

test("a book alone offers its chapters", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390);
  const { page } = opened;
  await field(page).tap();
  await field(page).fill("rim");
  const cells = page.locator('[data-role="study-chapter-cell"]');
  await expect(cells).toHaveCount(16);
  await assertMinTapTarget(cells, 44);
  await cells.nth(7).tap();
  await expectChapter(page, "slk-seb:ROM.8");
  await finish(opened);
});

test("an empty field lists the books, and a book leads to its chapters", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390);
  const { page } = opened;
  await field(page).tap();
  const books = page.locator('[data-role="study-book"]');
  await expect(books).toHaveCount(66);
  await page.locator('[data-role="study-book"][data-book-code="RUT"]').tap();
  await expect(page.locator('[data-role="study-chapter-cell"]')).toHaveCount(4);
  await page.locator('[data-role="study-chapter-cell"][data-chapter="2"]').tap();
  await expectChapter(page, "slk-seb:RUT.2");
  await finish(opened);
});

test("G26 stays Genezis 26: there are no Strong's numbers yet", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390);
  const { page } = opened;
  await field(page).tap();
  await field(page).fill("G26");
  await field(page).press("Enter");
  await expectChapter(page, "slk-seb:GEN.26");
  await finish(opened);
});

test("words search the reader's translation and a hit opens the verse", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390, "/ui/study?t=slk-seb&p=GEN.1", "slk-seb:GEN.1");
  const { page } = opened;
  await field(page).tap();
  await field(page).fill("viera zárukou");
  await expect(page.locator('[data-role="study-text-search"]')).toContainText("viera zárukou");
  await field(page).press("Enter");

  await expect(results(page)).toHaveAttribute("data-loaded-for", "slk-seb:viera zárukou");
  await expect(page).toHaveURL(/view=search&t=slk-seb&q=viera/);
  const hit = hits(page).filter({ hasText: "Hebrejom 11:1" }).first();
  await expect(hit).toContainText("· SEB");
  await expect(hit.locator("mark").first()).toBeVisible();
  await expect(field(page)).toHaveValue("viera zárukou");
  await expect(page.locator('[data-role="study-prev-chapter"]')).toHaveCount(0);

  await hit.tap();
  await expectChapter(page, "slk-seb:HEB.11");
  await expect(verse(page, 1)).toHaveAttribute("data-selected", "true");

  await page.goBack();
  await expect(results(page)).toHaveAttribute("data-loaded-for", "slk-seb:viera zárukou");
  await expect(field(page)).toHaveValue("viera zárukou");

  await page.goBack();
  await expectChapter(page, "slk-seb:GEN.1");
  await expect(field(page)).toHaveValue("");
  await finish(opened);
});

test("a search URL opens by itself and a chip searches the other translation", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390, "/ui/study?view=search&t=eng-kjv&q=faith", null);
  const { page } = opened;
  await expect(results(page)).toHaveAttribute("data-loaded-for", "eng-kjv:faith");
  await expect(page.locator('[data-role="search-count"]')).toHaveText(
    "Zobrazených prvých 100 výsledkov",
  );
  await expect(hits(page)).toHaveCount(100);
  await expect(hits(page).first()).toContainText("· KJV");

  await favourite(page, "slk-seb").tap();
  await expect(results(page)).toHaveAttribute("data-loaded-for", "slk-seb:faith");
  await finish(opened);
});

test("nothing found and the NLT each say so", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390, "/ui/study?view=search&t=slk-seb&q=qqxxzz", null);
  const { page } = opened;
  await expect(results(page)).toHaveAttribute("data-loaded-for", "slk-seb:qqxxzz");
  await expect(page.locator('[data-role="search-empty"]')).toContainText("Nič sa nenašlo");

  await favourite(page, "eng-nlt").tap();
  await expect(results(page)).toHaveAttribute("data-loaded-for", "eng-nlt:qqxxzz");
  await expect(page.locator('[data-role="search-empty"]')).toContainText("V NLT sa nedá hľadať");
  await finish(opened);
});

test("Back from a hit finds the list as it was, unasked", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390, "/ui/study?view=search&t=eng-kjv&q=faith", null);
  const { page } = opened;
  await expect(hits(page).first()).toBeVisible();
  const scroll = page.locator(".study-scroll");
  await scroll.evaluate((el) => (el.scrollTop = 1500));
  const searches: string[] = [];
  page.on("request", (request) => {
    if (request.url().includes("/bible/search")) searches.push(request.url());
  });
  const near = await page.evaluate(() => {
    const box = document.querySelector(".study-scroll") as HTMLElement;
    const rows = Array.from(document.querySelectorAll('[data-role="search-hit"]'));
    return rows.findIndex((row) => row.getBoundingClientRect().top > box.getBoundingClientRect().top);
  });
  await hits(page).nth(near).tap();
  await expect(page.locator('[data-role="study-chapter"]')).toHaveAttribute("data-state", "ready");

  await page.goBack();
  await expect(hits(page).first()).toBeAttached();
  await expect.poll(() => scroll.evaluate((el) => el.scrollTop)).toBe(1500);
  expect(searches).toEqual([]);
  await finish(opened);
});

test("a newer search wins over a slow older one", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390);
  const { page } = opened;
  const late = page.waitForResponse(/\/bible\/search\?query=viera/);
  const release = await gateRoute(page, /\/bible\/search\?query=viera/);
  await field(page).tap();
  await field(page).fill("viera");
  await field(page).press("Enter");
  await expect(results(page)).toHaveAttribute("data-state", "loading");
  await field(page).tap();
  await field(page).fill("nadej");
  await field(page).press("Enter");
  await expect(results(page)).toHaveAttribute("data-loaded-for", "slk-seb:nadej");
  release();
  await landed(late);
  await settled(page);
  await expect(results(page)).toHaveAttribute("data-loaded-for", "slk-seb:nadej");
  await finish(opened);
});

test("a book's name can still be searched as a word, under its chapters", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390);
  const { page } = opened;
  await field(page).tap();
  await field(page).fill("Izaiáš");
  await expect(page.locator('[data-role="study-chapter-cell"]').first()).toBeVisible();
  const search = page.locator('[data-role="study-text-search"]');
  await expect(search).toContainText("Izaiáš");
  const grid = await page.locator('[data-role="study-chapter-grid"]').boundingBox();
  const row = await search.boundingBox();
  expect(row?.y ?? 0).toBeGreaterThan(grid?.y ?? 0);
  await search.tap();
  await expect(results(page)).toHaveAttribute("data-loaded-for", "slk-seb:Izaiáš");
  await finish(opened);
});

test("the same reference again scrolls back to the verse", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390);
  const { page } = opened;
  await field(page).tap();
  await field(page).fill("zid 11 35");
  await field(page).press("Enter");
  await expect(verse(page, 35)).toBeInViewport();
  await page.locator('[data-role="study-scroll"]').evaluate((el) => (el.scrollTop = 0));
  await expect(verse(page, 35)).not.toBeInViewport();
  await field(page).tap();
  await field(page).fill("zid 11 35");
  await field(page).press("Enter");
  await expect(verse(page, 35)).toBeInViewport();
  await finish(opened);
});

test("a failed search retries inline", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390);
  const { page } = opened;
  let failed = false;
  await page.route("**/bible/search**", async (route) => {
    if (failed) return route.fallback();
    failed = true;
    await route.fulfill({ status: 200, contentType: "application/json", body: "not-json" });
  });
  await field(page).tap();
  await field(page).fill("viera");
  await field(page).press("Enter");
  const card = page.locator('[data-role="search-error"]');
  await expect(card).toContainText("Nepodarilo sa načítať");
  await card.locator('[data-role="study-retry"]').tap();
  await expect(hits(page).first()).toBeVisible();
  await expect(card).toHaveCount(0);
  await finish(opened);
});

for (const [name, profile] of [
  ["360", PHONE_360],
  ["390", PHONE_390],
  ["landscape", PHONE_LANDSCAPE],
] as const) {
  test(`search fits a phone: ${name}`, async ({ browser }) => {
    const opened = await openStudy(browser, baseURL, profile);
    const { page } = opened;
    await field(page).tap();
    await assertMinFontSize(field(page), 16);
    await expect(page.locator('[data-role="study-book"]')).toHaveCount(66);
    await assertNoHorizontalOverflow(page);
    await assertMinTapTarget(page.locator('[data-role="study-book"]'), 44);

    await field(page).fill("viera");
    await expect(page.locator('[data-role="study-text-search"]')).toBeVisible();
    await assertMinTapTarget(page.locator('[data-role="study-text-search"]'), 44);
    await field(page).press("Enter");
    await expect(hits(page).first()).toBeVisible();
    await assertNoHorizontalOverflow(page);
    await assertMinTapTarget(hits(page), 44);
    await finish(opened);
  });
}
