/**
 * #851 S4 — compare on `/ui/study`: one verse stacked on a phone, the chapter
 * in columns on wider screens, a failed NLT costing only its own rows. Nothing
 * here reaches a wall (T11).
 */
import { test, expect, type Locator, type Page } from "@playwright/test";
import {
  assertMinTapTarget,
  assertNoHorizontalOverflow,
  deriveTestConfig,
  PHONE_390,
  refreshDevData,
  startTestServer,
  stopServer,
  TABLET_1180,
  type ServerHandle,
} from "./support";
import { dropDeliberateNlt503, expectChapter, finish, openStudy, verseNumber } from "./study-support";

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

const compare = (page: Page): Locator => page.locator('[data-role="study-compare"]');
const row = (page: Page, code: string, verse: number): Locator =>
  page.locator(`[data-role="compare-row"][data-translation="${code}"][data-verse="${verse}"]`);

/** The verse's row sits at one height across the columns. */
async function expectAligned(page: Page, codes: string[], verse: number): Promise<void> {
  const tops = await Promise.all(
    codes.map(async (code) => {
      const box = await row(page, code, verse).boundingBox();
      expect(box, `${code} ${verse} is on the page`).not.toBeNull();
      return box?.y;
    }),
  );
  expect(new Set(tops).size, "a verse's row lines up across the columns").toBe(1);
}

test("the bar opens a phone's verse across the favourites, each row labelled", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390);
  const { page } = opened;
  await verseNumber(page, 3).tap();
  await page.locator('[data-role="verse-compare"]').tap();

  await expect(page).toHaveURL(/view=compare/);
  await expect(page).toHaveURL(/p=HEB\.11\.3/);
  await expect(compare(page)).toHaveAttribute("data-layout", "stacked");
  await expect(compare(page)).toHaveAttribute("data-loaded-for", "HEB.11.3");
  await expect(row(page, "slk-seb", 3)).toContainText("SEB");
  await expect(row(page, "slk-seb", 3).locator('[data-kind="text"]')).not.toBeEmpty();
  await expect(row(page, "eng-kjv", 3)).toContainText("Through faith we understand");
  await assertNoHorizontalOverflow(page);
  await assertMinTapTarget(page.locator('[data-role="compare-next"]'), 44);

  await page.goBack();
  await expectChapter(page, "slk-seb:HEB.11");
  dropDeliberateNlt503(opened);
  await finish(opened);
});

test("an unreachable NLT costs only its own row and says so", async ({ browser }) => {
  const opened = await openStudy(
    browser,
    baseURL,
    PHONE_390,
    "/ui/study?view=compare&t=slk-seb&p=HEB.11.1&tt=slk-seb,eng-nlt",
    null,
  );
  const { page } = opened;
  await expect(compare(page)).toHaveAttribute("data-loaded-for", "HEB.11.1");
  await expect(row(page, "eng-nlt", 1).locator('[data-role="compare-failed"]')).toContainText(
    "NLT nedostupné",
  );
  await expect(row(page, "slk-seb", 1).locator('[data-kind="text"]')).toContainText("Viera");
  dropDeliberateNlt503(opened);
  await finish(opened);
});

test("◀ ▶ move by verse, across chapters, and Back leaves compare at once", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390);
  const { page } = opened;
  await page.locator('[data-role="study-compare-open"]').tap();
  await expect(compare(page)).toHaveAttribute("data-loaded-for", "HEB.11.1");

  await page.locator('[data-role="compare-next"]').tap();
  await expect(compare(page)).toHaveAttribute("data-loaded-for", "HEB.11.2");
  await page.locator('[data-role="compare-prev"]').tap();
  await page.locator('[data-role="compare-prev"]').tap();
  await expect(compare(page)).toHaveAttribute("data-loaded-for", "HEB.10.39");

  await page.goBack();
  await expectChapter(page, "slk-seb:HEB.11");
  dropDeliberateNlt503(opened);
  await finish(opened);
});

test("a verse one translation lacks says so and the rest stays aligned", async ({ browser }) => {
  // The SEB has three chapters of Malachi, the KJV four.
  const opened = await openStudy(
    browser,
    baseURL,
    PHONE_390,
    "/ui/study?view=compare&t=eng-kjv&p=MAL.4.5&tt=eng-kjv,slk-seb",
    null,
  );
  const { page } = opened;
  await expect(compare(page)).toHaveAttribute("data-loaded-for", "MAL.4.5");
  await expect(row(page, "slk-seb", 5).locator('[data-kind="missing"]')).toContainText(
    "v tomto preklade tento verš nie je",
  );
  await expect(row(page, "eng-kjv", 5)).toContainText("Elijah");
  await finish(opened);
});

test("the current row is equally tall across the columns and the bar names the book", async ({
  browser,
}) => {
  const opened = await openStudy(
    browser,
    baseURL,
    TABLET_1180,
    "/ui/study?view=compare&t=slk-seb&p=HEB.11.5&tt=slk-seb,slk-roh,eng-kjv",
    null,
  );
  const { page } = opened;
  await expect(compare(page)).toHaveAttribute("data-loaded-for", "HEB.11.5");
  const heights = await Promise.all(
    ["slk-seb", "slk-roh", "eng-kjv"].map(
      async (code) => (await row(page, code, 5).boundingBox())?.height,
    ),
  );
  expect(new Set(heights).size, `row heights ${heights}`).toBe(1);
  await expect(page.locator('[data-role="compare-place"]')).toHaveText("Hebrejom 11:5");
  await finish(opened);
});

test("a tablet shows the chapter in aligned columns and a chip switches one", async ({ browser }) => {
  const opened = await openStudy(
    browser,
    baseURL,
    TABLET_1180,
    "/ui/study?view=compare&t=slk-seb&p=HEB.11.5&tt=slk-seb,slk-roh,eng-kjv",
    null,
  );
  const { page } = opened;
  await expect(compare(page)).toHaveAttribute("data-loaded-for", "HEB.11.5");
  await expect(compare(page)).toHaveAttribute("data-layout", "columns");
  await expect(compare(page)).toHaveAttribute("data-columns", "3");

  await expectAligned(page, ["slk-seb", "slk-roh", "eng-kjv"], 7);
  await expect(row(page, "slk-seb", 5)).toHaveAttribute("data-current", "true");
  await expect(row(page, "slk-seb", 5)).toBeInViewport();

  await page.locator('[data-role="compare-column"]').nth(1).selectOption("slk-sevp");
  await expect(page).toHaveURL(/tt=slk-seb,slk-sevp,eng-kjv/);
  await expect(row(page, "slk-sevp", 5).locator('[data-kind="text"]')).not.toBeEmpty();
  await expect(row(page, "slk-roh", 5)).toHaveCount(0);

  await row(page, "eng-kjv", 7).locator('[data-role="compare-open"]').click();
  await expectChapter(page, "eng-kjv:HEB.11");
  await expect(page.locator('[data-role="study-verse"][data-selected="true"]')).toHaveAttribute(
    "data-verse",
    "7",
  );
  await page.goBack();
  await expect(compare(page)).toHaveAttribute("data-loaded-for", "HEB.11.5");
  await finish(opened);
});

test("two translations on a wide screen stay two aligned columns", async ({ browser }) => {
  const opened = await openStudy(
    browser,
    baseURL,
    TABLET_1180,
    "/ui/study?view=compare&t=slk-seb&p=HEB.11.5&tt=slk-seb,eng-kjv",
    null,
  );
  const { page } = opened;
  await expect(compare(page)).toHaveAttribute("data-loaded-for", "HEB.11.5");
  await expect(compare(page)).toHaveAttribute("data-columns", "2");
  await expectAligned(page, ["slk-seb", "eng-kjv"], 7);
  await finish(opened);
});

test("a chip on a narrow tablet does not shorten what a wider screen shows", async ({ browser }) => {
  const opened = await openStudy(
    browser,
    baseURL,
    { ...TABLET_1180, viewport: { width: 820, height: 1180 } },
    "/ui/study?view=compare&t=slk-seb&p=HEB.11.5",
    null,
  );
  const { page } = opened;
  await expect(compare(page)).toHaveAttribute("data-columns", "2");
  await expect(compare(page)).toHaveAttribute("data-loaded-for", "HEB.11.5");
  await page.locator('[data-role="compare-column"]').nth(1).selectOption("eng-kjv");
  await expect(row(page, "eng-kjv", 5).locator('[data-kind="text"]')).not.toBeEmpty();

  await page.setViewportSize({ width: 1180, height: 820 });
  await expect(compare(page)).toHaveAttribute("data-columns", "3");
  await expect(page.locator('[data-role="compare-column"]')).toHaveCount(3);
  await expect(compare(page)).toHaveAttribute("data-loaded-for", "HEB.11.5");
  await finish(opened);
});

test("turning a tablet lands the columns on the verse", async ({ browser }) => {
  const opened = await openStudy(
    browser,
    baseURL,
    { ...TABLET_1180, viewport: { width: 700, height: 1000 } },
    "/ui/study?view=compare&t=slk-seb&p=HEB.11.20&tt=slk-seb,slk-roh",
    null,
  );
  const { page } = opened;
  await expect(compare(page)).toHaveAttribute("data-layout", "stacked");
  await expect(compare(page)).toHaveAttribute("data-loaded-for", "HEB.11.20");
  await page.setViewportSize({ width: 1100, height: 800 });
  await expect(compare(page)).toHaveAttribute("data-layout", "columns");
  await expect(row(page, "slk-seb", 20)).toBeInViewport();
  await finish(opened);
});

test("Back into the columns returns to where the reader had scrolled", async ({ browser }) => {
  const opened = await openStudy(
    browser,
    baseURL,
    TABLET_1180,
    "/ui/study?view=compare&t=slk-seb&p=HEB.11.5&tt=slk-seb,eng-kjv",
    null,
  );
  const { page } = opened;
  await expect(compare(page)).toHaveAttribute("data-loaded-for", "HEB.11.5");
  await row(page, "slk-seb", 30).scrollIntoViewIfNeeded();
  await row(page, "slk-seb", 30).locator('[data-role="compare-open"]').click();
  await expectChapter(page, "slk-seb:HEB.11");
  await page.goBack();
  await expect(compare(page)).toHaveAttribute("data-loaded-for", "HEB.11.5");
  await expect(row(page, "slk-seb", 30)).toBeInViewport();
  await expect(row(page, "slk-seb", 5)).not.toBeInViewport();
  await finish(opened);
});

test("a chapter the reader's translation lacks does not trap ◀ ▶", async ({ browser }) => {
  const opened = await openStudy(
    browser,
    baseURL,
    PHONE_390,
    "/ui/study?view=compare&t=slk-seb&p=MAL.4.5&tt=slk-seb,eng-kjv",
    null,
  );
  const { page } = opened;
  await expect(compare(page)).toHaveAttribute("data-loaded-for", "MAL.4.5");
  await page.locator('[data-role="compare-next"]').tap();
  await expect(page).not.toHaveURL(/p=MAL\.4\.5/);
  await finish(opened);
});
