/**
 * #851 S5 — the KJV word sheet and the Strong's view on `/ui/study`: a word
 * opens its entry without moving the text, Back walks the sheet and the links,
 * the occurrences list survives a verse and its Back. Nothing here reaches a
 * wall (T11).
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
import {
  expectChapter,
  field,
  finish,
  gateRoute,
  landed,
  openStudy,
  scrollToVerse,
  settled,
  verse,
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

const KJV_HEB = "/ui/study?t=eng-kjv&p=HEB.11";
const sheet = (page: Page): Locator => page.locator('[data-role="word-sheet"]');
const role = (page: Page, name: string): Locator => page.locator(`[data-role="${name}"]`);
const word = (page: Page, number: number, token: number): Locator =>
  page.locator(`[data-role="study-word"][data-verse="${number}"][data-token="${token}"]`);
const results = (page: Page): Locator => role(page, "strongs-results");
const hits = (page: Page): Locator => role(page, "strongs-hit");
const scopeChip = (page: Page, scope: string): Locator =>
  page.locator(`[data-role="search-scope"][data-scope="${scope}"]`);
const scrollTop = (page: Page): Promise<number> =>
  role(page, "study-scroll").evaluate((el) => el.scrollTop);

async function openFaith(page: Page): Promise<void> {
  await word(page, 1, 2).tap();
  await expect(sheet(page)).toHaveAttribute("data-strongs", "G4102");
  await expect(role(page, "word-sheet-lemma")).toHaveText("πίστις");
}

test("a word opens its entry and touches neither the verse nor the scroll", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390, KJV_HEB, "eng-kjv:HEB.11");
  const { page } = opened;
  await expect(word(page, 1, 2)).toHaveText("faith");
  await expect(word(page, 1, 2)).toHaveAttribute("role", "button");
  await expect(word(page, 1, 2)).toHaveAttribute("tabindex", "0");
  await openFaith(page);

  await expect(page).toHaveURL(/w=G4102&wv=1\.2/);
  await expect(sheet(page)).toHaveAttribute("data-state", "peek");
  await expect(sheet(page).locator(".study-sheet__title")).toHaveText("faith · G4102");
  await expect(role(page, "word-sheet-definition")).toHaveText("persuasion, i.e. credence");
  await expect(role(page, "word-sheet-kjv")).toContainText("V KJV: assurance, belief");
  await expect(word(page, 1, 2)).toHaveAttribute("data-active", "true");
  await expect(verse(page, 1)).toHaveAttribute("data-selected", "false");
  await expect(role(page, "verse-actions")).toHaveCount(0);
  await assertNoHorizontalOverflow(page);
  await assertMinTapTarget(role(page, "word-sheet-occurrences"), 44);
  await assertMinTapTarget(role(page, "word-sheet-full"), 44);
  await assertMinTapTarget(role(page, "word-sheet-next"), 44);

  await page.goBack();
  await expect(sheet(page)).toHaveCount(0);
  await expect(page).not.toHaveURL(/[?&]w=/);
  await finish(opened);
});

test("closing the sheet leaves the reading position where it was", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390, KJV_HEB, "eng-kjv:HEB.11");
  const { page } = opened;
  await scrollToVerse(page, 20);
  const before = await scrollTop(page);
  expect(before).toBeGreaterThan(200);

  await verse(page, 20).locator('[data-role="study-word"]').first().click();
  await expect(sheet(page)).toBeVisible();
  await page.goBack();
  await expect(sheet(page)).toHaveCount(0);
  await settled(page);
  expect(await scrollTop(page)).toBe(before);

  await verse(page, 20).locator('[data-role="study-word"]').first().click();
  await expect(sheet(page)).toBeVisible();
  await role(page, "study-backdrop").click({ position: { x: 20, y: 20 } });
  await expect(sheet(page)).toHaveCount(0);
  await settled(page);
  expect(await scrollTop(page)).toBe(before);
  await finish(opened);
});

test("Čísla shows the numbers, survives a reload and a badge opens the same sheet", async ({
  browser,
}) => {
  const opened = await openStudy(browser, baseURL, PHONE_390, KJV_HEB, "eng-kjv:HEB.11");
  const { page } = opened;
  const chip = role(page, "study-numbers");
  await expect(chip).toHaveAttribute("aria-pressed", "false");
  await expect(role(page, "study-strongs-badge")).toHaveCount(0);
  await assertMinTapTarget(chip, 44);

  await chip.tap();
  await expect(chip).toHaveAttribute("aria-pressed", "true");
  const badge = word(page, 1, 2).locator('[data-role="study-strongs-badge"]');
  await expect(badge).toHaveText("G4102");

  await page.reload();
  await expectChapter(page, "eng-kjv:HEB.11");
  await expect(role(page, "study-numbers")).toHaveAttribute("aria-pressed", "true");
  await badge.tap();
  await expect(sheet(page)).toHaveAttribute("data-strongs", "G4102");
  await page.goBack();
  await expect(sheet(page)).toHaveCount(0);

  await role(page, "study-numbers").tap();
  await expect(role(page, "study-strongs-badge")).toHaveCount(0);
  await finish(opened);
});

test("a translation without numbers has no chip and no words, and copies plain text", async ({
  browser,
}) => {
  const opened = await openStudy(browser, baseURL, PHONE_390);
  const { page } = opened;
  await expect(role(page, "study-numbers")).toHaveCount(0);
  await expect(role(page, "study-word")).toHaveCount(0);
  await role(page, "study-fav").and(page.locator('[data-translation="eng-kjv"]')).tap();
  await expectChapter(page, "eng-kjv:HEB.11");
  await expect(role(page, "study-numbers")).toBeVisible();
  await role(page, "study-fav").and(page.locator('[data-translation="slk-seb"]')).tap();
  await expectChapter(page, "slk-seb:HEB.11");
  await expect(role(page, "study-numbers")).toHaveCount(0);
  await finish(opened);
});

test("◀ ▶ walk the chapter's tagged words, stop at its ends and leave Back one step", async ({
  browser,
}) => {
  const opened = await openStudy(browser, baseURL, PHONE_390, KJV_HEB, "eng-kjv:HEB.11");
  const { page } = opened;
  await word(page, 1, 0).tap();
  await expect(sheet(page)).toHaveAttribute("data-strongs", "G1161");
  await expect(role(page, "word-sheet-prev")).toBeDisabled();

  await role(page, "word-sheet-next").tap();
  await expect(sheet(page)).toHaveAttribute("data-strongs", "G4102");
  await expect(page).toHaveURL(/w=G4102&wv=1\.2/);
  await expect(word(page, 1, 2)).toHaveAttribute("data-active", "true");
  await expect(word(page, 1, 0)).not.toHaveAttribute("data-active", "true");
  await role(page, "word-sheet-prev").tap();
  await expect(sheet(page)).toHaveAttribute("data-strongs", "G1161");
  await role(page, "word-sheet-next").tap();

  await page.goBack();
  await expect(sheet(page)).toHaveCount(0);

  // The chapter's last tagged word: "perfect", 11:40.
  await word(page, 40, 24).click();
  await expect(sheet(page)).toHaveAttribute("data-strongs", "G5048");
  await expect(role(page, "word-sheet-next")).toBeDisabled();
  await expect(role(page, "word-sheet-prev")).toBeEnabled();
  await finish(opened);
});

test("a derivation link opens that entry, Back returns and ✕ leaves the sheet", async ({
  browser,
}) => {
  const opened = await openStudy(browser, baseURL, PHONE_390, KJV_HEB, "eng-kjv:HEB.11");
  const { page } = opened;
  await openFaith(page);
  await role(page, "word-sheet-full").tap();
  await expect(sheet(page)).toHaveAttribute("data-state", "full");
  await expect(role(page, "word-sheet-definition")).toContainText("moral conviction");
  const link = page.locator('[data-role="word-sheet-link"][data-strongs="G3982"]');
  await assertMinTapTarget(link, 44);

  await link.tap();
  await expect(sheet(page)).toHaveAttribute("data-strongs", "G3982");
  await expect(role(page, "word-sheet-lemma")).toHaveText("πείθω");
  await expect(page).toHaveURL(/w=G3982&wv=1\.2/);

  await page.goBack();
  await expect(sheet(page)).toHaveAttribute("data-strongs", "G4102");
  await expect(role(page, "word-sheet-lemma")).toHaveText("πίστις");

  await link.tap();
  await expect(sheet(page)).toHaveAttribute("data-strongs", "G3982");
  await role(page, "study-sheet-close").tap();
  await expect(sheet(page)).toHaveCount(0);
  await expect(page).not.toHaveURL(/[?&]w=/);
  await finish(opened);
});

test("Všetky výskyty lists the verses, marks the word, and a scope chip narrows them", async ({
  browser,
}) => {
  const opened = await openStudy(browser, baseURL, PHONE_390, KJV_HEB, "eng-kjv:HEB.11");
  const { page } = opened;
  await openFaith(page);
  await expect(role(page, "word-sheet-occurrences")).toHaveText("Všetky výskyty (228)");
  await role(page, "word-sheet-occurrences").tap();

  await expect(results(page)).toHaveAttribute("data-loaded-for", "G4102:all");
  await expect(page).toHaveURL(/view=strongs&t=eng-kjv&n=G4102&scope=all/);
  await expect(sheet(page)).toHaveCount(0);
  await expect(hits(page)).toHaveCount(100);
  await expect(role(page, "strongs-count")).toHaveText("Veršov: 228");
  await expect(hits(page).first().locator("mark").first()).toHaveText("faith");
  await expect(scopeChip(page, "all")).toHaveAttribute("aria-pressed", "true");
  await expect(scopeChip(page, "all")).toContainText("Celá Biblia (228)");
  await expect(scopeChip(page, "nt")).toContainText("NZ (228)");
  await expect(scopeChip(page, "ot"), "no such verses: no dead chip").toHaveCount(0);
  await expect(scopeChip(page, "HEB")).toContainText("(31)");
  await assertNoHorizontalOverflow(page);
  await assertMinTapTarget(scopeChip(page, "HEB"), 44);

  await scopeChip(page, "HEB").tap();
  await expect(results(page)).toHaveAttribute("data-loaded-for", "G4102:HEB");
  await expect(page).toHaveURL(/scope=HEB/);
  await expect(hits(page)).toHaveCount(31);
  await expect(role(page, "strongs-more")).toHaveCount(0);
  await expect(hits(page).first()).toHaveAttribute("data-book-code", "HEB");
  await finish(opened);
});

test("Ďalšie appends, a verse opens the KJV, and Back finds the list unasked", async ({ browser }) => {
  const opened = await openStudy(
    browser,
    baseURL,
    PHONE_390,
    "/ui/study?view=strongs&t=slk-seb&n=G4102&scope=all",
    null,
  );
  const { page } = opened;
  await expect(results(page)).toHaveAttribute("data-loaded-for", "G4102:all");
  await expect(role(page, "strongs-more")).toHaveText("Ďalšie (128)");
  await role(page, "strongs-more").tap();
  await expect(hits(page)).toHaveCount(200);
  await expect(role(page, "strongs-more")).toHaveText("Ďalšie (28)");
  await role(page, "strongs-more").tap();
  await expect(hits(page)).toHaveCount(228);
  await expect(role(page, "strongs-more")).toHaveCount(0);

  const asked: string[] = [];
  page.on("request", (request) => {
    if (request.url().includes("/bible/study/strongs/")) asked.push(request.url());
  });
  const target = page.locator(
    '[data-role="strongs-hit"][data-book-code="HEB"][data-chapter="11"][data-verse="1"]',
  );
  await target.evaluate((el) => el.scrollIntoView());
  const before = await scrollTop(page);
  expect(before).toBeGreaterThan(1000);

  await target.click();
  await expectChapter(page, "eng-kjv:HEB.11");
  await expect(verse(page, 1)).toHaveAttribute("data-selected", "true");

  await page.goBack();
  await expect(results(page)).toHaveAttribute("data-loaded-for", "G4102:all");
  await expect(hits(page)).toHaveCount(228);
  await expect.poll(() => scrollTop(page)).toBe(before);
  expect(asked, "Back asks nothing").toEqual([]);
  await expect(page).toHaveURL(/t=slk-seb/);
  await finish(opened);
});

test("a late page of the scope left behind does not land", async ({ browser }) => {
  const opened = await openStudy(
    browser,
    baseURL,
    PHONE_390,
    "/ui/study?view=strongs&t=eng-kjv&n=G4102&scope=all",
    null,
  );
  const { page } = opened;
  await expect(results(page)).toHaveAttribute("data-loaded-for", "G4102:all");
  const release = await gateRoute(page, /strongs\/G4102\?scope=all&offset=100/);
  const late = page.waitForResponse(/strongs\/G4102\?scope=all&offset=100/);
  await role(page, "strongs-more").tap();
  await expect(role(page, "strongs-more")).toBeDisabled();

  await scopeChip(page, "ROM").tap();
  await expect(results(page)).toHaveAttribute("data-loaded-for", "G4102:ROM");
  await expect(hits(page)).toHaveCount(35);

  release();
  await landed(late);
  await settled(page);
  await expect(hits(page)).toHaveCount(35);
  await expect(results(page)).toHaveAttribute("data-loaded-for", "G4102:ROM");
  await finish(opened);
});

test("a Hebrew lemma reads right to left and its number links too", async ({ browser }) => {
  const opened = await openStudy(
    browser,
    baseURL,
    PHONE_390,
    "/ui/study?t=eng-kjv&p=GEN.1",
    "eng-kjv:GEN.1",
  );
  const { page } = opened;
  await word(page, 1, 3).tap();
  await expect(sheet(page)).toHaveAttribute("data-strongs", "H430");
  await expect(role(page, "word-sheet-lemma")).toHaveAttribute("dir", "rtl");
  await expect(role(page, "word-sheet-lemma")).toHaveText("אֱלֹהִים");
  await role(page, "word-sheet-full").tap();
  await expect(page.locator('[data-role="word-sheet-link"][data-strongs="H433"]')).toBeVisible();
  await role(page, "word-sheet-next").tap();
  await expect(sheet(page)).toHaveAttribute("data-strongs", "H1254");
  await expect(role(page, "word-sheet-lemma")).toHaveAttribute("dir", "rtl");
  await finish(opened);
});

test("a word opens over compare, stays in the verse on screen and survives a reload", async ({
  browser,
}) => {
  const opened = await openStudy(
    browser,
    baseURL,
    PHONE_390,
    "/ui/study?view=compare&t=slk-seb&p=HEB.11.1&tt=slk-seb,eng-kjv",
    null,
  );
  const { page } = opened;
  const compare = role(page, "study-compare");
  await expect(compare).toHaveAttribute("data-loaded-for", "HEB.11.1");
  await role(page, "study-numbers").waitFor();
  const faith = page
    .locator('[data-role="compare-row"][data-translation="eng-kjv"]')
    .locator('[data-role="study-word"][data-token="2"]');
  await faith.tap();
  await expect(sheet(page)).toHaveAttribute("data-strongs", "G4102");
  await expect(page).toHaveURL(/view=compare.*w=G4102&wv=1\.2/);

  await page.reload();
  await expect(sheet(page)).toHaveAttribute("data-strongs", "G4102");
  await expect(role(page, "word-sheet-lemma")).toHaveText("πίστις");
  await expect(compare).toHaveAttribute("data-loaded-for", "HEB.11.1");

  // Verse 2 is not on a phone's compare page: the steps end with this verse.
  for (let step = 0; step < 40; step += 1) {
    if (await role(page, "word-sheet-next").isDisabled()) break;
    await role(page, "word-sheet-next").tap();
  }
  await expect(role(page, "word-sheet-next")).toBeDisabled();
  await expect(page).toHaveURL(/wv=1\./);

  await role(page, "study-sheet-close").tap();
  await expect(sheet(page)).toHaveCount(0);
  await expect(compare).toHaveAttribute("data-loaded-for", "HEB.11.1");
  await finish(opened);
});

test("a URL naming no word on the page leaves no sheet, a good one is kept by a reload", async ({
  browser,
}) => {
  const opened = await openStudy(
    browser,
    baseURL,
    PHONE_390,
    `${KJV_HEB}&w=G4102&wv=99.1`,
    "eng-kjv:HEB.11",
  );
  const { page } = opened;
  await expect(page).not.toHaveURL(/[?&]w=/);
  await expect(sheet(page)).toHaveCount(0);

  await page.goto("/ui/study?t=slk-seb&p=HEB.11&w=G4102&wv=1.2");
  await expectChapter(page, "slk-seb:HEB.11");
  await expect(page).not.toHaveURL(/[?&]w=/);
  await expect(sheet(page)).toHaveCount(0);

  await page.goto("/ui/study?view=compare&t=slk-seb&p=HEB.11.1&tt=slk-seb,slk-roh&w=G4102&wv=1.2");
  await expect(role(page, "study-compare")).toHaveAttribute("data-loaded-for", "HEB.11.1");
  await expect(page).not.toHaveURL(/[?&]w=/);
  await expect(sheet(page)).toHaveCount(0);

  await page.goto(`${KJV_HEB}&w=G4102&wv=1.2`);
  await expect(sheet(page)).toHaveAttribute("data-strongs", "G4102");
  await page.reload();
  await expect(sheet(page)).toHaveAttribute("data-strongs", "G4102");
  await role(page, "study-sheet-close").tap();
  await expect(sheet(page)).toHaveCount(0);
  await expectChapter(page, "eng-kjv:HEB.11");
  await finish(opened);
});

test("a word with several numbers has a tab each", async ({ browser }) => {
  const opened = await openStudy(
    browser,
    baseURL,
    PHONE_390,
    "/ui/study?t=eng-kjv&p=GEN.1",
    "eng-kjv:GEN.1",
    async (page) => {
      await page.route(/\/bible\/study\/chapter\?.*book_code=GEN/, (route) =>
        route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify({
            book: "Genesis",
            verses: [
              {
                verse: 1,
                text: "In the beginning God.",
                markup: [
                  { t: "In the " },
                  { t: "beginning", s: ["H7225", "H430"] },
                  { t: " God." },
                ],
              },
            ],
          }),
        }),
      );
    },
  );
  const { page } = opened;
  await word(page, 1, 1).tap();
  await expect(sheet(page)).toHaveAttribute("data-strongs", "H7225");
  const tabs = role(page, "word-sheet-tab");
  await expect(tabs).toHaveText(["H7225", "H430"]);
  await expect(tabs.first()).toHaveAttribute("aria-pressed", "true");
  await tabs.nth(1).tap();
  await expect(sheet(page)).toHaveAttribute("data-strongs", "H430");
  await expect(page).toHaveURL(/w=H430&wv=1\.1/);
  await expect(tabs.nth(1)).toHaveAttribute("aria-pressed", "true");
  await page.goBack();
  await expect(sheet(page)).toHaveCount(0);
  await finish(opened);
});

test("no dictionary says so, and a failed read offers a retry", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390, KJV_HEB, "eng-kjv:HEB.11");
  const { page } = opened;
  await page.route(/\/bible\/study\/lexicon\/G1161$/, (route) =>
    route.fulfill({ status: 404, contentType: "application/json", body: "{}" }),
  );
  await word(page, 1, 0).tap();
  await expect(role(page, "word-sheet-missing")).toHaveText("Slovník nie je nainštalovaný");
  await expect(role(page, "word-sheet-occurrences"), "no dead button").toHaveCount(0);

  let broken = true;
  await page.route(/\/bible\/study\/lexicon\/G2076$/, async (route) => {
    if (broken) await route.fulfill({ status: 200, contentType: "application/json", body: "nie" });
    else await route.fallback();
  });
  await role(page, "study-sheet-close").tap();
  await word(page, 1, 4).tap();
  await expect(role(page, "word-sheet-error")).toBeVisible();
  broken = false;
  await role(page, "word-sheet-error").locator('[data-role="study-retry"]').tap();
  await expect(role(page, "word-sheet-lemma")).toBeVisible();

  const deliberate = /Failed to load resource: the server responded with a status of 404\b/;
  expect(opened.errors.filter((text) => deliberate.test(text))).toHaveLength(1);
  opened.errors.splice(0, opened.errors.length, ...opened.errors.filter((t) => !deliberate.test(t)));
  await finish(opened);
});

test("a tablet docks the sheet beside the text", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, TABLET_1180, KJV_HEB, "eng-kjv:HEB.11");
  const { page } = opened;
  await word(page, 1, 2).click();
  const box = await sheet(page).boundingBox();
  expect(box?.height).toBe(TABLET_1180.viewport.height);
  expect((box?.x ?? 0) + (box?.width ?? 0)).toBe(TABLET_1180.viewport.width);
  await assertNoHorizontalOverflow(page);
  await finish(opened);
});

test("the field opens a number first, and keeps the passage for g 26", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390);
  const { page } = opened;
  await expect(field(page)).toHaveAttribute("placeholder", "Odkaz, slová alebo G26…");
  await field(page).tap();
  await field(page).fill("G26");
  await expect(role(page, "study-strongs")).toContainText("G26");
  await expect(role(page, "study-row").first()).toContainText("Genezis 26");
  await field(page).press("Enter");
  await expect(results(page)).toHaveAttribute("data-loaded-for", "G26:all");
  await expect(hits(page).first().locator("mark").first()).toBeVisible();

  await page.goBack();
  await expectChapter(page, "slk-seb:HEB.11");
  await field(page).tap();
  await field(page).fill("G26");
  await role(page, "study-row").first().tap();
  await expectChapter(page, "slk-seb:GEN.26");
  await finish(opened);
});

test("notes, italics and headings render, the words of a note stay out of the line", async ({
  browser,
}) => {
  const opened = await openStudy(
    browser,
    baseURL,
    PHONE_390,
    "/ui/study?t=slk-seb&p=1CH.1",
    "slk-seb:1CH.1",
  );
  const { page } = opened;
  const note = verse(page, 3).locator('[data-role="study-note"]').first();
  await expect(verse(page, 3)).not.toContainText("Hebr. Chanók");
  await assertMinTapTarget(note, 44);
  await note.tap();
  await expect(role(page, "study-note-text")).toContainText("Hebr. Chanók");
  await note.tap();
  await expect(role(page, "study-note-text")).toHaveCount(0);

  await page.goto("/ui/study?t=eng-kjv&p=GEN.2");
  await expectChapter(page, "eng-kjv:GEN.2");
  await expect(verse(page, 14).locator("i").first()).toHaveText("is");

  await page.goto("/ui/study?t=slk-roh&p=GEN.1");
  await expectChapter(page, "slk-roh:GEN.1");
  await expect(verse(page, 1).locator('[data-role="study-heading"]')).toHaveText("Stvorenie sveta");
  await expect(verse(page, 1)).toContainText("Na počiatku");
  await finish(opened);
});

test("a copy of a tagged verse stays the plain text", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390, KJV_HEB, "eng-kjv:HEB.11");
  const { page, context } = opened;
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await role(page, "study-numbers").tap();
  await expect(word(page, 1, 2).locator('[data-role="study-strongs-badge"]')).toBeVisible();
  await verse(page, 1).locator('[data-role="study-verse-number"]').tap();
  await role(page, "verse-copy").tap();
  await expect(role(page, "study-notice")).toHaveText("Skopírované");
  const copied = await page.evaluate(() => navigator.clipboard.readText());
  expect(copied.startsWith("Now faith is the substance")).toBe(true);
  expect(copied).not.toContain("G4102");
  expect(await verseOffset(page, 1)).toBeGreaterThanOrEqual(0);
  await finish(opened);
});
