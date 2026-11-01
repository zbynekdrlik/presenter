/**
 * #851 S1 — the reader on `/ui/study`: one tap to another translation at the
 * same place, chapters across books, its own URL for every step, and a page
 * that fits a phone (T3). Nothing here reaches a wall (T11).
 */
import { test, expect } from "@playwright/test";
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
  chapter,
  expectChapter,
  favourite,
  field,
  finish,
  gateRoute,
  landed,
  openStudy,
  scrollToVerse,
  settled,
  verse,
  verseNumber,
  verseOffset,
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

const nextChapter = '[data-role="study-next-chapter"]';
const previousChapter = '[data-role="study-prev-chapter"]';

test("a first visit opens John 1 in the first favourite and fixes the URL", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390, "/ui/study", "slk-seb:JHN.1");
  const { page } = opened;
  await expect(page).toHaveURL(/\/ui\/study\?t=slk-seb&p=JHN\.1$/);
  await expect(favourite(page, "slk-seb")).toHaveAttribute("aria-pressed", "true");
  await expect(verse(page, 1)).toContainText("Slovo");
  await finish(opened);
});

test("one tap switches the translation at the same place, and Back returns", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390);
  const { page } = opened;
  await expect(page.locator('[data-role="study-title"]')).toContainText("Hebrejom 11");
  await expect(verse(page, 1)).toContainText("Viera");

  await favourite(page, "eng-kjv").tap();
  await expectChapter(page, "eng-kjv:HEB.11");
  await expect(favourite(page, "eng-kjv")).toHaveAttribute("aria-pressed", "true");
  await expect(favourite(page, "slk-seb")).toHaveAttribute("aria-pressed", "false");
  await expect(page.locator('[data-role="study-title"]')).toContainText("Hebrews 11");
  await expect(verse(page, 1)).toContainText("faith");
  await expect(page).toHaveURL(/t=eng-kjv&p=HEB\.11/);

  await page.goBack();
  await expectChapter(page, "slk-seb:HEB.11");
  await expect(favourite(page, "slk-seb")).toHaveAttribute("aria-pressed", "true");
  await finish(opened);
});

test("a switch keeps the verse being read at the top", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390);
  const { page } = opened;
  await scrollToVerse(page, 20);
  await favourite(page, "eng-kjv").tap();
  await expectChapter(page, "eng-kjv:HEB.11");
  await expect.poll(() => verseOffset(page, 20)).toBeGreaterThanOrEqual(0);
  await expect.poll(() => verseOffset(page, 20)).toBeLessThanOrEqual(16);
  await finish(opened);
});

test("the bar turns chapters over book boundaries, with a URL for each", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390, "/ui/study?t=slk-seb&p=MAT.27", "slk-seb:MAT.27");
  const { page } = opened;
  const bar = page.locator(nextChapter);
  await expect(bar).toContainText("kap. 28");
  await bar.tap();
  await expectChapter(page, "slk-seb:MAT.28");
  await expect(page).toHaveURL(/p=MAT\.28/);
  await expect(chapter(page)).toHaveAttribute("data-book-code", "MAT");
  await expect(chapter(page)).toHaveAttribute("data-chapter", "28");

  await expect(bar).toContainText("Marek 1");
  await bar.tap();
  await expectChapter(page, "slk-seb:MRK.1");

  await page.locator(previousChapter).tap();
  await expectChapter(page, "slk-seb:MAT.28");

  await page.goBack();
  await expectChapter(page, "slk-seb:MRK.1");
  await page.goBack();
  await expectChapter(page, "slk-seb:MAT.28");
  await finish(opened);
});

test("the Bible has two ends", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390, "/ui/study?t=slk-seb&p=GEN.1", "slk-seb:GEN.1");
  const { page } = opened;
  await expect(page.locator(previousChapter)).toBeDisabled();
  await expect(page.locator(nextChapter)).toBeEnabled();
  await field(page).fill("zj 22");
  await field(page).press("Enter");
  await expectChapter(page, "slk-seb:REV.22");
  await expect(page.locator(nextChapter)).toBeDisabled();
  await expect(page.locator(previousChapter)).toBeEnabled();
  await finish(opened);
});

test("a chapter on its way leaves the text in place and only the newest lands", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390);
  const { page } = opened;
  let release: () => void = () => undefined;
  const held = new Promise<void>((resolve) => (release = resolve));
  const asksFor = (body: unknown, chapterNumber: number) =>
    (body as { chapter: number }).chapter === chapterNumber;
  await page.route("**/bible/resolve", async (route) => {
    if (asksFor(route.request().postDataJSON(), 12)) await held;
    await route.fallback();
  });
  const late = page.waitForResponse(
    (response) =>
      response.url().endsWith("/bible/resolve") &&
      asksFor(response.request().postDataJSON(), 12),
  );
  await page.locator(nextChapter).tap();
  await expect(chapter(page)).toHaveAttribute("data-state", "loading");
  await expect(verse(page, 1)).toContainText("Viera");

  await page.locator(nextChapter).tap();
  await expectChapter(page, "slk-seb:HEB.13");
  release();
  await landed(late);
  await settled(page);
  await expectChapter(page, "slk-seb:HEB.13");
  await finish(opened);
});

test("a quick second translation wins over a slow first one", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390);
  const { page } = opened;
  let release: () => void = () => undefined;
  const held = new Promise<void>((resolve) => (release = resolve));
  const isKjv = (body: unknown) => (body as { mainTranslation: string }).mainTranslation === "eng-kjv";
  await page.route("**/bible/resolve", async (route) => {
    if (isKjv(route.request().postDataJSON())) await held;
    await route.fallback();
  });
  const late = page.waitForResponse(
    (response) =>
      response.url().endsWith("/bible/resolve") && isKjv(response.request().postDataJSON()),
  );
  await favourite(page, "eng-kjv").tap();
  await expect(chapter(page)).toHaveAttribute("data-state", "loading");
  await favourite(page, "slk-roh").tap();
  await expectChapter(page, "slk-roh:HEB.11");
  release();
  await landed(late);
  await settled(page);
  await expectChapter(page, "slk-roh:HEB.11");
  await expect(favourite(page, "slk-roh")).toHaveAttribute("aria-pressed", "true");
  await finish(opened);
});

test("a verse number does nothing while another chapter is on its way", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390);
  const { page } = opened;
  const answer = page.waitForResponse("**/bible/resolve");
  const release = await gateRoute(page, "**/bible/resolve");
  await page.locator(nextChapter).tap();
  await expect(chapter(page)).toHaveAttribute("data-state", "loading");
  await expect(page).toHaveURL(/p=HEB\.12/);

  // Verse 1 on screen is Hebrews 11; a selection now would name Hebrews 12.
  await expect(verseNumber(page, 1)).toBeDisabled();
  await verseNumber(page, 1).dispatchEvent("click");
  await expect(page.locator('[data-role="verse-actions"]')).toHaveCount(0);
  await expect(page).not.toHaveURL(/v=1/);

  release();
  await landed(answer);
  await expectChapter(page, "slk-seb:HEB.12");
  await expect(verseNumber(page, 1)).toBeEnabled();
  await expect(verse(page, 1)).toHaveAttribute("data-selected", "false");
  await finish(opened);
});

test("failed books say so, the bar stays neutral and a reference never becomes a search", async ({
  browser,
}) => {
  let failing = true;
  const searches: string[] = [];
  const opened = await openStudy(browser, baseURL, PHONE_390, undefined, undefined, async (page) => {
    await page.route("**/bible/books**", async (route) => {
      if (!failing) return route.fallback();
      await route.fulfill({ status: 200, contentType: "application/json", body: "not-json" });
    });
    page.on("request", (request) => {
      if (request.url().includes("/bible/search")) searches.push(request.url());
    });
  });
  const { page } = opened;
  const card = page.locator('[data-role="study-structure-error"]');
  await expect(card).toBeVisible();
  await expect(page.locator(nextChapter)).toBeDisabled();
  await expect(page.locator(nextChapter)).not.toContainText("Koniec");
  await expect(page.locator(previousChapter)).not.toContainText("Začiatok");

  await field(page).tap();
  await field(page).fill("zid 11 1");
  await expect(page.locator('[data-role="study-hint"]')).toBeVisible();
  await expect(page.locator('[data-role="study-text-search"]')).toHaveCount(0);
  await field(page).press("Enter");
  await expect(page).not.toHaveURL(/view=search/);
  expect(searches).toEqual([]);
  await field(page).blur();

  failing = false;
  await card.locator('[data-role="study-retry"]').tap();
  await expect(card).toHaveCount(0);
  await expect(page.locator(nextChapter)).toContainText("kap. 12");
  await field(page).tap();
  await field(page).fill("zid 11 3");
  await field(page).press("Enter");
  await expect(verse(page, 3)).toHaveAttribute("data-selected", "true");
  await finish(opened);
});

test("a failed chapter shows why and retries inline", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390);
  const { page } = opened;
  let failed = false;
  await page.route("**/bible/resolve", async (route) => {
    if (failed) return route.fallback();
    failed = true;
    await route.fulfill({ status: 200, contentType: "application/json", body: "not-json" });
  });
  await page.locator(nextChapter).tap();
  const card = page.locator('[data-role="study-chapter-error"]');
  await expect(card).toContainText("Nepodarilo sa načítať");
  await expect(chapter(page)).toHaveAttribute("data-state", "error");
  await card.locator('[data-role="study-retry"]').tap();
  await expectChapter(page, "slk-seb:HEB.12");
  await expect(card).toHaveCount(0);
  await finish(opened);
});

test("an unreachable NLT says so inline, the other chips stay", async ({ browser }) => {
  // support.ts points the NLT at a dead loopback: the server's real 503.
  const opened = await openStudy(browser, baseURL, PHONE_390);
  const { page } = opened;
  await favourite(page, "eng-nlt").tap();
  const card = page.locator('[data-role="study-chapter-error"]');
  await expect(card).toContainText("NLT nedostupné");
  await expect(card.locator('[data-role="study-retry"]')).toBeVisible();
  await card.locator('[data-role="study-retry"]').tap();
  await expect(chapter(page)).toHaveAttribute("data-state", "error");

  await favourite(page, "slk-seb").tap();
  await expectChapter(page, "slk-seb:HEB.11");

  // Chrome logs each deliberate 503 (ui skill #718): exactly these, nothing else.
  const deliberate = /Failed to load resource: the server responded with a status of 503\b/;
  const expected = opened.errors.filter((text) => deliberate.test(text));
  expect(expected.length).toBeGreaterThanOrEqual(1);
  opened.errors.splice(0, opened.errors.length, ...opened.errors.filter((text) => !deliberate.test(text)));
  await finish(opened);
});

test("a gospels-only translation says what it holds", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390, "/ui/study?t=slk-mil&p=HEB.11", null);
  const { page } = opened;
  await expect(page.locator('[data-role="study-missing"]')).toHaveText(
    "Tento preklad obsahuje len evanjeliá",
  );
  await expect(chapter(page)).toHaveAttribute("data-state", "missing");
  await finish(opened);
});

for (const [name, profile] of [
  ["360", PHONE_360],
  ["390", PHONE_390],
  ["landscape", PHONE_LANDSCAPE],
] as const) {
  test(`the reader fits a phone: ${name}`, async ({ browser }) => {
    const opened = await openStudy(browser, baseURL, profile);
    const { page } = opened;
    await assertNoHorizontalOverflow(page);
    await assertMinFontSize(field(page), 16);
    await assertMinTapTarget(page.locator('[data-role="study-fav"]'), 44);
    await assertMinTapTarget(page.locator('[data-role="history-back"]'), 44);
    await assertMinTapTarget(page.locator('[data-role="history-forward"]'), 44);
    await assertMinTapTarget(page.locator('[data-role="study-text-size"]'), 44);
    await assertMinTapTarget(page.locator(nextChapter), 44);
    await assertMinTapTarget(page.locator(previousChapter), 44);
    await assertMinTapTarget(page.locator('[data-role="study-verse-number"]').first(), 44);
    await assertMinFontSize(page.locator(".study-verse__text").first(), 16);

    // The text gets the room, even on a phone on its side.
    const height = await page.locator('[data-role="study-scroll"]').evaluate((el) => el.clientHeight);
    expect(height).toBeGreaterThan((profile.viewport.height * 0.6) | 0);
    await finish(opened);
  });
}

test("a tablet in portrait keeps one column, wider", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, {
    viewport: { width: 820, height: 1180 },
    hasTouch: true,
  });
  const { page } = opened;
  await assertNoHorizontalOverflow(page);
  const width = await chapter(page).evaluate((el) => el.getBoundingClientRect().width);
  expect(width).toBeGreaterThan(600);
  expect(width).toBeLessThanOrEqual(720);
  await finish(opened);
});

test("a wide screen keeps the text column at 720 px and docks a sheet beside it", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, { viewport: { width: 1280, height: 800 } });
  const { page } = opened;
  await assertNoHorizontalOverflow(page);
  await page.locator('[data-role="study-verse-number"]').first().click();
  const sheet = page.locator('[data-role="verse-actions"]');
  await expect(sheet).toBeVisible();
  const box = await sheet.boundingBox();
  const text = await chapter(page).boundingBox();
  expect(box?.height).toBe(800);
  expect(box?.width).toBeGreaterThanOrEqual(380);
  expect(box?.width).toBeLessThanOrEqual(480);
  expect((text?.x ?? 0) + (text?.width ?? 0)).toBeLessThanOrEqual(box?.x ?? 0);
  await finish(opened);
});

test("the page installs as its own app and its worker caches nothing", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390);
  const { page } = opened;
  await expect(page.locator('link[rel="manifest"]')).toHaveAttribute(
    "href",
    "/ui/study/manifest.json",
  );
  const manifest = await page.request.get("/ui/study/manifest.json");
  expect(manifest.ok()).toBeTruthy();
  expect(manifest.headers()["content-type"]).toContain("application/manifest+json");
  const body = await manifest.json();
  expect(body).toMatchObject({
    id: "/ui/study",
    scope: "/ui/study",
    start_url: "/ui/study",
    name: "Presenter Štúdium",
    short_name: "Štúdium",
    orientation: "any",
  });
  const worker = await page.request.get("/ui/study/sw.js");
  expect(worker.ok()).toBeTruthy();
  const code = await worker.text();
  expect(code).not.toMatch(/["'`]fetch["'`]/);
  expect(code).not.toContain("caches.");
  await finish(opened);
});

test("the operator's surface nav opens the page", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, { viewport: { width: 1280, height: 800 } }, "/ui/operator", null);
  const { page } = opened;
  const link = page.locator('[data-role="surface-nav-link"][data-target="Štúdium"]');
  await expect(link).toHaveAttribute("href", "/ui/study");
  await link.evaluate((el) => el.removeAttribute("target"));
  await link.click();
  await expect(chapter(page)).toHaveAttribute("data-loaded-for", /:JHN\.1$/);
  // The operator polls the stage; T11 is about the study page.
  opened.wallRequests.length = 0;
  await settled(page);
  await finish(opened);
});
