/**
 * #932 — the preacher reads on a phone (owner, 2026-11-11): "text biblie má
 * zbytočne veľké odsadenie zľava a sprava … keď je názov knihy a kapitola hore
 * v hlavičkovom riadku, nemusí byť ešte na začiatku kapitoly … pridať záložku
 * … mala by byť prvá … miesto aby sa rovno pridala, vyskočí ďalšie okno".
 * Nothing here reaches a wall (T11).
 */
import { test, expect, type BrowserContextOptions, type Page } from "@playwright/test";
import {
  assertMinTapTarget,
  deriveTestConfig,
  PHONE_360,
  PHONE_390,
  refreshDevData,
  startTestServer,
  stopServer,
  TABLET_1180,
  type ServerHandle,
} from "./support";
import { expectChapter, finish, openStudy, verse, verseNumber } from "./study-support";

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

/** The left and right gaps between a verse's text and the screen's edges. */
async function textGaps(page: Page, number: number): Promise<{ left: number; right: number }> {
  return verse(page, number)
    .locator(".study-line")
    .evaluate((line) => {
      const box = line.getBoundingClientRect();
      return { left: box.left, right: document.documentElement.clientWidth - box.right };
    });
}

/**
 * The left edge of each rendered line of a verse's text (the number left out),
 * read from the glyphs with a Range: the `p` box says nothing about a line a
 * float or an indent pushes in.
 */
async function lineStarts(page: Page, number: number): Promise<number[]> {
  return verse(page, number)
    .locator(".study-line")
    .evaluate((line) => {
      const range = document.createRange();
      range.selectNodeContents(line);
      const button = line.querySelector('[data-role="study-verse-number"]');
      if (button) range.setStartAfter(button);
      const starts = new Map<number, number>();
      for (const rect of Array.from(range.getClientRects())) {
        if (rect.width === 0) continue;
        const row = Math.round(rect.top + rect.height / 2);
        const key = Array.from(starts.keys()).find((top) => Math.abs(top - row) < 8) ?? row;
        starts.set(key, Math.min(starts.get(key) ?? Infinity, rect.left));
      }
      return Array.from(starts.entries())
        .sort(([a], [b]) => a - b)
        .map(([, left]) => left);
    });
}

type Bookmark = { id: number; name: string; verse: number; bookCode: string; chapter: number };

async function bookmarks(page: Page): Promise<Bookmark[]> {
  return page.evaluate(async () => (await fetch("/bible/study/bookmarks")).json());
}

for (const [label, profile] of [
  ["360", PHONE_360],
  ["390", PHONE_390],
] as [string, BrowserContextOptions][]) {
  test(`at ${label} px the verses run to a 12 px edge and keep a 44 px number`, async ({ browser }) => {
    const opened = await openStudy(browser, baseURL, profile);
    const { page } = opened;
    for (const number of [1, 4, 10]) {
      const gaps = await textGaps(page, number);
      expect(gaps.left, `verse ${number} left`).toBeLessThanOrEqual(12);
      expect(gaps.right, `verse ${number} right`).toBeLessThanOrEqual(12);
      expect(gaps.left, `verse ${number} left`).toBeGreaterThanOrEqual(6);
      expect(gaps.right, `verse ${number} right`).toBeGreaterThanOrEqual(6);
      // Every line after the first starts at the edge; the first right after the number.
      const starts = await lineStarts(page, number);
      expect(starts.length, `verse ${number} lines`).toBeGreaterThanOrEqual(2);
      expect(starts[0], `verse ${number} first line`).toBeLessThanOrEqual(44);
      for (const [index, left] of starts.slice(1).entries()) {
        expect(left, `verse ${number} line ${index + 2}`).toBeLessThanOrEqual(12);
      }
    }
    // The number is a real 44 x 44 box from the screen's edge, on the verse's first line (T3).
    const number = verseNumber(page, 4);
    await assertMinTapTarget(number, 44);
    const line = verse(page, 4).locator(".study-line");
    const lineBox = await line.boundingBox();
    const box = await number.boundingBox();
    expect(box && box.x).toBeLessThanOrEqual(1);
    const lineHeight = await line.evaluate((el) => parseFloat(getComputedStyle(el).lineHeight));
    const middle = (lineBox?.y ?? 0) + lineHeight / 2;
    expect(box && Math.abs(box.y + box.height / 2 - middle)).toBeLessThanOrEqual(2);
    // The second line starts at the 10 px edge, under the number: the text owns it.
    const under = await page.evaluate(
      ([x, y]) => document.elementFromPoint(x, y)?.closest('[data-role="study-verse-number"]') === null,
      [14, (lineBox?.y ?? 0) + lineHeight * 1.5] as const,
    );
    expect(under).toBe(true);
    // A tap on the number opens the verse sheet; it is never also a text tap (that would close it).
    await number.tap();
    await expect(page.locator('[data-role="verse-actions"]')).toBeVisible();
    await expect(verse(page, 4)).toHaveAttribute("data-selected", "true");
    await finish(opened);
  });
}

test("a tablet keeps the 44 px number column", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, TABLET_1180);
  const { page } = opened;
  const row = await verse(page, 4).boundingBox();
  const line = await verse(page, 4).locator(".study-line").boundingBox();
  expect(row && line && Math.round(line.x - row.x)).toBe(44);
  const number = await verseNumber(page, 4).boundingBox();
  expect(row && number && Math.round(number.x - row.x)).toBe(0);
  expect(number?.height).toBeGreaterThanOrEqual(44);
  await finish(opened);
});

test("the header names the chapter, the text repeats no chapter heading", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390);
  const { page } = opened;
  await expect(page.locator('[data-role="study-place"]')).toHaveText("Hebrejom 11");
  const chapter = page.locator('[data-role="study-chapter"]');
  await expect(chapter.locator("h1, h2")).toHaveCount(0);
  // The SEB has no Strong's numbers: no Čísla chip, so the head takes no room.
  await expect(chapter.locator(".study-chapter__head")).toBeHidden();
  await expect(chapter).not.toContainText("Hebrejom 11");
  // The first verse is the first thing under the field.
  const scroller = await page.locator('[data-role="study-scroll"]').boundingBox();
  const first = await verse(page, 1).boundingBox();
  expect(scroller && first && first.y - scroller.y).toBeLessThanOrEqual(24);
  await finish(opened);
});

test("Pridať záložku is the verse sheet's first action and adds in one tap", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390);
  const { page } = opened;
  const before = (await bookmarks(page)).map((item) => item.id);
  // One chapter on: a second history step back would land on Hebrews 11.
  await page.locator('[data-role="study-next-chapter"]').tap();
  await expectChapter(page, "slk-seb:HEB.12");
  await verseNumber(page, 6).tap();
  const actions = page.locator('[data-role="verse-actions"] .study-sheet__actions button');
  await expect(actions.first()).toHaveText("Pridať záložku");
  await expect(actions.first()).toHaveAttribute("data-role", "verse-save-bookmark");

  // A quick double tap adds it once: the sheet closes through a history step.
  await actions.first().evaluate((button: HTMLElement) => {
    button.click();
    button.click();
  });
  await expect(page.locator('[data-role="study-notice"]')).toContainText("Záložka pridaná: Hebrejom 12:6");
  // No naming sheet, no verse sheet: back to reading.
  await expect(page.locator('[data-role="save-bookmark"]')).toHaveCount(0);
  await expect(page.locator('[data-role="verse-actions"]')).toHaveCount(0);
  await expect(page).not.toHaveURL(/[?&]s=/);
  await expectChapter(page, "slk-seb:HEB.12");
  await expect(page).toHaveURL(/p=HEB\.12$/);

  // Read back from the server: exactly one new bookmark at Hebrews 12:6.
  const added = (await bookmarks(page)).filter((item) => !before.includes(item.id));
  expect(added).toHaveLength(1);
  expect(added[0]).toMatchObject({ name: "Hebrejom 12:6", bookCode: "HEB", chapter: 12, verse: 6 });

  // It is in Záložky, renamable there.
  await page.locator('[data-role="study-bookmarks"]').tap();
  const sheet = page.locator('[data-role="bookmarks"]');
  await expect(sheet).toHaveAttribute("data-state", "ready");
  const row = sheet.locator(`[data-bookmark-id="${added[0].id}"]`);
  await expect(row.locator(".study-row__label")).toHaveText("Hebrejom 12:6");
  await expect(row.locator('[data-role="bookmark-rename"]')).toBeVisible();
  await row.locator('[data-role="bookmark-delete"]').tap();
  await expect(row).toHaveCount(0);
  await finish(opened);
});
