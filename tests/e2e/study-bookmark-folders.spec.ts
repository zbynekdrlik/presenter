/**
 * Bookmark folders on `/ui/study` (#952, owner 2026-11-20: folders, not tags).
 * The Záložky sheet opens on the default folder "Kde som skončil"; a row of chips
 * picks another folder (in the URL: `s=zalozky<id>`); "＋ Nový priečinok" makes
 * one; an own folder can be renamed and deleted (asks, takes its bookmarks).
 * "Pridať záložku" still saves into the default folder in one tap; ▾ beside it
 * saves into a chosen folder. On a 390 px phone (touch) and on a desktop
 * (mouse). Nothing here reaches a wall (T11).
 */
import { test, expect, type BrowserContextOptions, type Locator, type Page } from "@playwright/test";
import {
  assertMinTapTarget,
  assertNoHorizontalOverflow,
  deriveTestConfig,
  PHONE_390,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";
import { expectChapter, finish, openStudy, verseNumber, type OpenedStudy } from "./study-support";

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

const DESKTOP: BrowserContextOptions = { viewport: { width: 1280, height: 900 } };

const PROFILES: [string, BrowserContextOptions, boolean][] = [
  ["phone 390", PHONE_390, true],
  ["desktop", DESKTOP, false],
];

type Folder = { id: number; name: string; count: number };
type Stored = { id: number; verse: number };

const notice = (page: Page): Locator => page.locator('[data-role="study-notice"]');
const sheet = (page: Page): Locator => page.locator('[data-role="bookmarks"]');
const chips = (page: Page): Locator => sheet(page).locator('[data-role="bookmark-folder"]');
const chip = (page: Page, id: number | null): Locator =>
  sheet(page).locator(`[data-role="bookmark-folder"][data-folder="${id ?? ""}"]`);
const rows = (page: Page): Locator => sheet(page).locator('[data-role="bookmark-row"]');

async function press(target: Locator, touch: boolean): Promise<void> {
  if (touch) await target.tap();
  else await target.click();
}

async function api<T>(page: Page, path: string, method = "GET", body?: unknown): Promise<{ status: number; json: T }> {
  return page.evaluate(
    async ({ path, method, body }) => {
      const response = await fetch(path, {
        method,
        headers: body === undefined ? undefined : { "content-type": "application/json" },
        body: body === undefined ? undefined : JSON.stringify(body),
      });
      const text = await response.text();
      return { status: response.status, json: text ? JSON.parse(text) : null };
    },
    { path, method, body },
  );
}

const folders = async (page: Page): Promise<Folder[]> => (await api<Folder[]>(page, "/bible/study/bookmark-folders")).json;

async function verses(page: Page, folder: number | null): Promise<number[]> {
  const path = folder === null ? "/bible/study/bookmarks" : `/bible/study/bookmarks?folder=${folder}`;
  const answer = await api<Stored[]>(page, path);
  expect(answer.status).toBe(200);
  return answer.json.map((item) => item.verse);
}

/** Every test starts with no folder and an empty default folder (the server lives through the file). */
async function clean(page: Page): Promise<void> {
  for (const folder of await folders(page)) {
    expect((await api(page, "/bible/study/bookmark-folders/delete", "POST", { id: folder.id })).status).toBe(200);
  }
  const ids = (await api<Stored[]>(page, "/bible/study/bookmarks")).json.map((item) => item.id);
  if (ids.length > 0) {
    expect((await api(page, "/bible/study/bookmarks/delete", "POST", { ids })).status).toBe(200);
  }
}

async function open(browser: import("@playwright/test").Browser, profile: BrowserContextOptions): Promise<OpenedStudy> {
  const opened = await openStudy(browser, baseURL, profile);
  await clean(opened.page);
  return opened;
}

/** The sheet settled on `folder`: its chips and this visit's list answered. */
async function settled(page: Page, folder: number | null): Promise<void> {
  await expect(page).toHaveURL(folder === null ? /s=zalozky(&|$)/ : new RegExp(`s=zalozky${folder}(&|$)`));
  await expect(sheet(page).locator('[data-role="bookmark-folders"]')).toHaveAttribute("data-loaded", "true");
  await expect(sheet(page)).toHaveAttribute("data-state", "ready");
  await expect(chip(page, folder)).toHaveAttribute("aria-pressed", "true");
}

async function shownVerses(page: Page): Promise<string[]> {
  return rows(page).locator(".study-bookmark__label").allTextContents();
}

for (const [label, profile, touch] of PROFILES) {
  test(`${label}: a new folder, ▾ saves a verse into it, one tap still saves into "Kde som skončil"`, async ({
    browser,
  }) => {
    const opened = await open(browser, profile);
    const { page } = opened;

    // The sheet opens on the default folder; no own folders yet.
    await press(page.locator('[data-role="study-bookmarks"]'), touch);
    await settled(page, null);
    await expect(chips(page)).toHaveText(["Kde som skončil"]);
    await expect(sheet(page).locator('[data-role="bookmark-folder-menu"]')).toHaveCount(0);

    // "＋ Nový priečinok" asks for a name and shows the new, empty folder.
    page.once("dialog", (dialog) => {
      expect(dialog.type()).toBe("prompt");
      void dialog.accept("Viera");
    });
    await press(sheet(page).locator('[data-role="bookmark-folder-new"]'), touch);
    await expect(notice(page)).toContainText("Priečinok „Viera“ je vytvorený");
    const [viera] = await folders(page);
    expect(viera.name).toBe("Viera");
    await settled(page, viera.id);
    await expect(chips(page)).toHaveText(["Kde som skončil", "Viera"]);
    await expect(sheet(page).locator('[data-role="bookmarks-empty"]')).toContainText("Priečinok je prázdny");
    if (touch) {
      await assertMinTapTarget(chip(page, viera.id), 44);
      await assertMinTapTarget(sheet(page).locator('[data-role="bookmark-folder-new"]'), 44);
      await assertMinTapTarget(sheet(page).locator('[data-role="bookmark-folder-delete"]'), 44);
      await assertNoHorizontalOverflow(page);
    }
    // Back closes the sheet in one step: a folder switch is no history entry.
    await page.goBack();
    await expect(page).not.toHaveURL(/s=zalozky/);
    await expect(sheet(page)).toHaveCount(0);

    // ▾ beside "Pridať záložku": the folders, one tap saves into "Viera".
    await press(verseNumber(page, 3), touch);
    const more = page.locator('[data-role="verse-save-bookmark-folder"]');
    if (touch) await assertMinTapTarget(more, 44);
    await press(more, touch);
    const picker = page.locator('[data-role="verse-bookmark-folders"]');
    await expect(picker).toHaveAttribute("data-loaded", "true");
    await expect(picker.locator('[data-role="verse-bookmark-folder"]')).toHaveText(["Kde som skončil", "Viera"]);
    if (touch) await assertNoHorizontalOverflow(page);
    await press(picker.locator(`[data-folder="${viera.id}"]`), touch);
    await expect(notice(page)).toContainText("Záložka pridaná do „Viera“: Hebrejom 11:3");
    await expect(page.locator('[data-role="verse-actions"]')).toHaveCount(0);

    // The one tap is unchanged: verse 4 goes into the default folder.
    await press(verseNumber(page, 4), touch);
    await press(page.locator('[data-role="verse-save-bookmark"]'), touch);
    await expect(notice(page)).toContainText("Záložka pridaná: Hebrejom 11:4");
    expect(await verses(page, null)).toEqual([4]);
    expect(await verses(page, viera.id)).toEqual([3]);

    // The sheet shows each folder's own list; a chip switches.
    await press(page.locator('[data-role="study-bookmarks"]'), touch);
    await settled(page, null);
    await expect.poll(() => shownVerses(page)).toEqual(["Hebrejom 11:4"]);
    await press(chip(page, viera.id), touch);
    await settled(page, viera.id);
    await expect.poll(() => shownVerses(page)).toEqual(["Hebrejom 11:3"]);
    // A row of the folder opens its verse like any bookmark.
    await press(rows(page).first().locator('[data-role="bookmark-open"]'), touch);
    await expectChapter(page, "slk-seb:HEB.11");
    await expect(page).toHaveURL(/p=HEB\.11&v=3$/);
    await finish(opened);
  });

  test(`${label}: a folder by its URL, renamed, then deleted with its bookmarks after asking`, async ({ browser }) => {
    const opened = await open(browser, profile);
    const { page } = opened;
    const made = await api<Folder>(page, "/bible/study/bookmark-folders", "POST", { name: "Modlitba" });
    expect(made.status).toBe(200);
    const id = made.json.id;
    for (const [verse, folder] of [
      [1, id],
      [2, id],
      [5, null],
    ] as const) {
      const body = { name: `Hebrejom 11:${verse}`, translation: "slk-seb", bookCode: "HEB", chapter: 11, verse, folder };
      expect((await api(page, "/bible/study/bookmarks", "POST", body)).status).toBe(200);
    }

    // A link names the folder: the sheet opens on it, and a reload keeps it.
    await page.goto(`/ui/study?t=slk-seb&p=HEB.11&s=zalozky${id}`);
    await page.waitForSelector('body[data-wasm-ready="true"]', { state: "attached", timeout: 30_000 });
    await settled(page, id);
    await expect.poll(() => shownVerses(page)).toEqual(["Hebrejom 11:2", "Hebrejom 11:1"]);
    await page.reload();
    await page.waitForSelector('body[data-wasm-ready="true"]', { state: "attached", timeout: 30_000 });
    await settled(page, id);

    // Rename: the prompt offers the current name.
    page.once("dialog", (dialog) => {
      expect(dialog.defaultValue()).toBe("Modlitba");
      void dialog.accept("Modlitba a pôst");
    });
    await press(sheet(page).locator('[data-role="bookmark-folder-rename"]'), touch);
    await expect(notice(page)).toContainText("Priečinok sa volá „Modlitba a pôst“");
    await expect(chip(page, id)).toHaveText("Modlitba a pôst");
    expect((await folders(page)).map((folder) => folder.name)).toEqual(["Modlitba a pôst"]);

    // Delete asks, naming its bookmarks; "Zrušiť" keeps everything.
    page.once("dialog", (dialog) => {
      expect(dialog.type()).toBe("confirm");
      expect(dialog.message()).toBe("Zmazať priečinok „Modlitba a pôst“ aj s 2 záložkami? Záložky v ňom zmiznú.");
      void dialog.dismiss();
    });
    await press(sheet(page).locator('[data-role="bookmark-folder-delete"]'), touch);
    await expect(chip(page, id)).toBeVisible();
    expect(await folders(page)).toHaveLength(1);

    page.once("dialog", (dialog) => void dialog.accept());
    await press(sheet(page).locator('[data-role="bookmark-folder-delete"]'), touch);
    await expect(notice(page)).toContainText("Priečinok „Modlitba a pôst“ je zmazaný");
    await settled(page, null);
    await expect(chips(page)).toHaveText(["Kde som skončil"]);
    await expect.poll(() => shownVerses(page)).toEqual(["Hebrejom 11:5"]);
    expect(await folders(page)).toEqual([]);
    expect(await verses(page, null)).toEqual([5]);
    await finish(opened);
  });
}

test("a link to a folder that is gone falls back to the default folder", async ({ browser }) => {
  const opened = await open(browser, PHONE_390);
  const { page } = opened;
  const made = await api<Folder>(page, "/bible/study/bookmark-folders", "POST", { name: "Dočasný" });
  const id = made.json.id;
  expect((await api(page, "/bible/study/bookmark-folders/delete", "POST", { id })).status).toBe(200);
  await page.goto(`/ui/study?t=slk-seb&p=HEB.11&s=zalozky${id}`);
  await page.waitForSelector('body[data-wasm-ready="true"]', { state: "attached", timeout: 30_000 });
  await settled(page, null);
  await expect(chips(page)).toHaveText(["Kde som skončil"]);
  // The gone folder's list answered 404 before the folders did: that line only.
  const deliberate = /Failed to load resource: the server responded with a status of 404\b/;
  expect(opened.errors.filter((text) => deliberate.test(text)).length).toBeLessThanOrEqual(1);
  opened.errors.splice(0, opened.errors.length, ...opened.errors.filter((text) => !deliberate.test(text)));
  await finish(opened);
});
