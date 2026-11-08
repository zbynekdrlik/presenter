/**
 * #915 — named bookmarks on `/ui/study` (owner, 2026-11-08: "studium nemam
 * moznost si ulozit kde citam"; ROZHODNUTÉ option 2, a list of several named
 * bookmarks). The list lives on the server, so it survives a reload and is the
 * same on every device. Nothing here reaches a wall (T11).
 */
import { test, expect, type BrowserContextOptions, type Locator, type Page } from "@playwright/test";
import {
  assertMinTapTarget,
  assertNoHorizontalOverflow,
  deriveTestConfig,
  PHONE_360,
  PHONE_390,
  refreshDevData,
  startTestServer,
  stopServer,
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

const notice = (page: Page): Locator => page.locator('[data-role="study-notice"]');
const sheet = (page: Page): Locator => page.locator('[data-role="bookmarks"]');
/** The row of the bookmark called `name` (each test names its own). */
const bookmark = (page: Page, name: string | RegExp): Locator =>
  sheet(page)
    .locator('[data-role="bookmark-row"]')
    .filter({ has: page.locator('[data-role="bookmark-open"] .study-row__label', { hasText: name }) });

async function openList(page: Page): Promise<void> {
  await page.locator('[data-role="study-bookmarks"]').tap();
  await expect(page).toHaveURL(/s=zalozky/);
  // The settle marker: this visit's list answered (a list of an earlier visit never counts).
  await expect(sheet(page)).toHaveAttribute("data-state", "ready");
}

for (const [label, profile] of [
  ["360", PHONE_360],
  ["390", PHONE_390],
] as [string, BrowserContextOptions][]) {
  test(`at ${label} px a bookmark is saved, kept over a reload, opened, renamed, moved and deleted`, async ({
    browser,
  }) => {
    const name = `Kázeň ${label}`;
    const renamed = `Nedeľa ${label}`;
    const opened = await openStudy(browser, baseURL, profile);
    const { page } = opened;

    // Save: the verse sheet names the verse, the name is editable.
    await verseNumber(page, 4).tap();
    await page.locator('[data-role="verse-save-bookmark"]').tap();
    await expect(page).toHaveURL(/v=4&s=zalozka/);
    const nameField = page.locator('[data-role="bookmark-new-name"]');
    await expect(nameField).toHaveValue("Hebrejom 11:4");
    await assertNoHorizontalOverflow(page);
    await nameField.fill(name);
    await page.locator('[data-role="bookmark-save"]').tap();
    await expect(notice(page)).toContainText(`Záložka uložená: ${name}`);
    await expect(page.locator('[data-role="save-bookmark"]')).toHaveCount(0);

    // A new visit somewhere else: the list comes from the server.
    await page.goto("/ui/study?t=slk-seb&p=ROM.8");
    await page.waitForSelector('body[data-wasm-ready="true"]', { state: "attached", timeout: 30_000 });
    await expectChapter(page, "slk-seb:ROM.8");
    await assertMinTapTarget(page.locator('[data-role="study-bookmarks"]'), 44);
    await openList(page);
    const row = bookmark(page, name);
    await expect(row.locator(".study-row__detail")).toHaveText("Hebrejom 11:4 (SEB)");
    await assertMinTapTarget(row.locator("button"), 44);
    await assertNoHorizontalOverflow(page);

    // Open: the reader lands on the verse, highlighted.
    await row.locator('[data-role="bookmark-open"]').tap();
    await expectChapter(page, "slk-seb:HEB.11");
    await expect(page).toHaveURL(/p=HEB\.11&v=4$/);
    await expect(verse(page, 4)).toHaveAttribute("data-selected", "true");
    await expect(verse(page, 4)).toBeInViewport();

    // Rename.
    await openList(page);
    await bookmark(page, name).locator('[data-role="bookmark-rename"]').tap();
    const renameField = sheet(page).locator('[data-role="bookmark-name"]');
    await expect(renameField).toHaveValue(name);
    await renameField.fill(renamed);
    await sheet(page).locator('[data-role="bookmark-rename-save"]').tap();
    await expect(bookmark(page, renamed).locator(".study-row__detail")).toHaveText("Hebrejom 11:4 (SEB)");

    // Move here: from another chapter, to the verse selected there.
    await page.goto("/ui/study?t=slk-seb&p=ROM.8&v=28");
    await page.waitForSelector('body[data-wasm-ready="true"]', { state: "attached", timeout: 30_000 });
    await expectChapter(page, "slk-seb:ROM.8");
    await openList(page);
    await bookmark(page, renamed).locator('[data-role="bookmark-move"]').tap();
    await expect(bookmark(page, renamed).locator(".study-row__detail")).toHaveText(/ 8:28 \(SEB\)$/);
    await expect(notice(page)).toContainText("Presunuté:");

    // Delete, Späť brings it back, delete again: no test data is left.
    await bookmark(page, renamed).locator('[data-role="bookmark-delete"]').tap();
    await expect(bookmark(page, renamed)).toHaveCount(0);
    await expect(notice(page)).toContainText(`Zmazané: ${renamed}`);
    await page.locator('[data-role="study-notice-undo"]').tap();
    await expect(bookmark(page, renamed).locator(".study-row__detail")).toHaveText(/ 8:28 \(SEB\)$/);
    await bookmark(page, renamed).locator('[data-role="bookmark-delete"]').tap();
    await expect(bookmark(page, renamed)).toHaveCount(0);
    const left = await page.evaluate(async () => (await fetch("/bible/study/bookmarks")).json());
    expect(left.map((item: { name: string }) => item.name)).not.toContain(renamed);
    await finish(opened);
  });
}

test("a name of spaces cannot be saved; the server refuses it in Slovak too", async ({
  browser,
}) => {
  const opened = await openStudy(browser, baseURL, PHONE_390);
  const { page } = opened;
  await verseNumber(page, 2).tap();
  await page.locator('[data-role="verse-save-bookmark"]').tap();
  const nameField = page.locator('[data-role="bookmark-new-name"]');
  const save = page.locator('[data-role="bookmark-save"]');
  await expect(nameField).toHaveValue("Hebrejom 11:2");
  await expect(save).toBeEnabled();
  await nameField.fill("   ");
  await expect(save).toBeDisabled();
  await nameField.press("Enter");
  await expect(page.locator('[data-role="save-bookmark"]')).toBeVisible();
  const saved = await page.evaluate(async () => (await fetch("/bible/study/bookmarks")).json());
  expect(saved.map((item: { name: string }) => item.name)).not.toContain("   ");
  // The server refuses it too, in Slovak (the API itself; the button never sends it).
  const refused = await page.evaluate(async () => {
    const response = await fetch("/bible/study/bookmarks", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ name: " ", translation: "slk-seb", bookCode: "HEB", chapter: 11, verse: 2 }),
    });
    return { status: response.status, body: await response.json() };
  });
  expect(refused).toEqual({ status: 422, body: { message: "Záložka potrebuje názov" } });
  const deliberate = /Failed to load resource: the server responded with a status of 422\b/;
  expect(opened.errors.filter((text) => deliberate.test(text))).toHaveLength(1);
  opened.errors.splice(0, opened.errors.length, ...opened.errors.filter((text) => !deliberate.test(text)));
  await finish(opened);
});
