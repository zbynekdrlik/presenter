/**
 * #932 — the preacher reads on a phone (owner, 2026-11-11): "text biblie má
 * zbytočne veľké odsadenie zľava a sprava … keď je názov knihy a kapitola hore
 * v hlavičkovom riadku, nemusí byť ešte na začiatku kapitoly … pridať záložku
 * … mala by byť prvá … miesto aby sa rovno pridala, vyskočí ďalšie okno".
 * Nothing here reaches a wall (T11).
 */
import { test, expect, type BrowserContextOptions, type Page } from "@playwright/test";
import {
  deriveTestConfig,
  PHONE_360,
  PHONE_390,
  refreshDevData,
  startTestServer,
  stopServer,
  TABLET_1180,
  type ServerHandle,
} from "./support";
import { finish, openStudy, verse, verseNumber } from "./study-support";

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
    }
    // The number sits on the verse's first line, its tap target stays 44 x 44 (T3).
    const number = verseNumber(page, 4);
    const line = await verse(page, 4).locator(".study-line").boundingBox();
    const box = await number.boundingBox();
    expect(box && line && Math.abs(box.y - line.y)).toBeLessThanOrEqual(1);
    const target = await number.evaluate((el) => {
      const style = getComputedStyle(el, "::before");
      return { width: parseFloat(style.width), height: parseFloat(style.height) };
    });
    expect(target.width).toBeGreaterThanOrEqual(44);
    expect(target.height).toBeGreaterThanOrEqual(44);
    // A tap on the number opens the verse sheet, never also highlights as a text tap would close it.
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
  await verseNumber(page, 6).tap();
  const actions = page.locator('[data-role="verse-actions"] .study-sheet__actions button');
  await expect(actions.first()).toHaveText("Pridať záložku");
  await expect(actions.first()).toHaveAttribute("data-role", "verse-save-bookmark");

  await actions.first().tap();
  await expect(page.locator('[data-role="study-notice"]')).toContainText("Záložka pridaná: Hebrejom 11:6");
  // No naming sheet, no verse sheet: back to reading.
  await expect(page.locator('[data-role="save-bookmark"]')).toHaveCount(0);
  await expect(page.locator('[data-role="verse-actions"]')).toHaveCount(0);
  await expect(page).not.toHaveURL(/[?&]s=/);

  // Read back from the server: exactly one new bookmark at Hebrews 11:6.
  const added = (await bookmarks(page)).filter((item) => !before.includes(item.id));
  expect(added).toHaveLength(1);
  expect(added[0]).toMatchObject({ name: "Hebrejom 11:6", bookCode: "HEB", chapter: 11, verse: 6 });

  // It is in Záložky, renamable there.
  await page.locator('[data-role="study-bookmarks"]').tap();
  const sheet = page.locator('[data-role="bookmarks"]');
  await expect(sheet).toHaveAttribute("data-state", "ready");
  const row = sheet.locator(`[data-bookmark-id="${added[0].id}"]`);
  await expect(row.locator(".study-row__label")).toHaveText("Hebrejom 11:6");
  await expect(row.locator('[data-role="bookmark-rename"]')).toBeVisible();
  await row.locator('[data-role="bookmark-delete"]').tap();
  await expect(row).toHaveCount(0);
  await finish(opened);
});
