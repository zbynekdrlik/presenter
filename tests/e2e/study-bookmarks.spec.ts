/**
 * Bookmarks on `/ui/study`: #915 (a server-side list per reader), reshaped by
 * #951 (owner, 2026-11-14: "ja len potrebujem vediet kde som skoncil citanie"):
 * newest on top, a compact row = the place + the start of the verse's text, the
 * whole row opens it, a drag on the small handle reorders, "Vybrať" → ONE
 * "Zmazať (N)" with Späť. No rename, no "Presunúť sem". On a 390 px phone (touch)
 * and on a desktop (mouse). Nothing here reaches a wall (T11).
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
  touchDrag,
  type ServerHandle,
} from "./support";
import { expectChapter, finish, openStudy, verse, verseNumber, type OpenedStudy } from "./study-support";

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

/** A desktop wide enough to dock the sheet as the side pane, tall enough for every row. */
const DESKTOP: BrowserContextOptions = { viewport: { width: 1280, height: 900 } };

const PROFILES: [string, BrowserContextOptions, boolean][] = [
  ["phone 390", PHONE_390, true],
  ["desktop", DESKTOP, false],
];

type Stored = { id: number; name: string; verse: number; chapter: number; bookCode: string; text: string | null };

const notice = (page: Page): Locator => page.locator('[data-role="study-notice"]');
const sheet = (page: Page): Locator => page.locator('[data-role="bookmarks"]');
const rows = (page: Page): Locator => sheet(page).locator('[data-role="bookmark-row"]');
const row = (page: Page, id: number): Locator => sheet(page).locator(`[data-bookmark-id="${id}"]`);

async function press(target: Locator, touch: boolean): Promise<void> {
  if (touch) await target.tap();
  else await target.click();
}

async function stored(page: Page): Promise<Stored[]> {
  return page.evaluate(async () => (await fetch("/bible/study/bookmarks")).json());
}

/** Every test starts from an empty list (the spec's server lives through the file). */
async function emptyList(page: Page): Promise<void> {
  const ids = (await stored(page)).map((item) => item.id);
  if (ids.length === 0) return;
  const status = await page.evaluate(
    async (ids) =>
      (
        await fetch("/bible/study/bookmarks/delete", {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({ ids }),
        })
      ).status,
    ids,
  );
  expect(status).toBe(200);
}

/** Hebrews 11:<verse> through the API, oldest first: answers the ids in that order. */
async function seed(page: Page, verses: number[]): Promise<number[]> {
  const ids: number[] = [];
  for (const at of verses) {
    const made = await page.evaluate(async (at) => {
      const response = await fetch("/bible/study/bookmarks", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ name: `Hebrejom 11:${at}`, translation: "slk-seb", bookCode: "HEB", chapter: 11, verse: at }),
      });
      return response.json();
    }, at);
    ids.push(made.id);
  }
  return ids;
}

async function openList(page: Page, touch: boolean): Promise<void> {
  await press(page.locator('[data-role="study-bookmarks"]'), touch);
  await expect(page).toHaveURL(/s=zalozky/);
  // The settle marker: this visit's list answered (a list of an earlier visit never counts).
  await expect(sheet(page)).toHaveAttribute("data-state", "ready");
}

/** The ids as the sheet shows them, top first. */
async function shownIds(page: Page): Promise<number[]> {
  const ids = await rows(page).evaluateAll((list) => list.map((el) => Number(el.getAttribute("data-bookmark-id"))));
  return ids;
}

/** Both the screen and the server hold `ids`, top first. */
async function expectOrder(page: Page, ids: number[]): Promise<void> {
  await expect.poll(() => shownIds(page)).toEqual(ids);
  await expect.poll(async () => (await stored(page)).map((item) => item.id)).toEqual(ids);
}

/** Drag row `id` by its handle to height `y` (a finger on the phone, the mouse on the desktop). */
async function dragTo(page: Page, id: number, y: number, touch: boolean): Promise<void> {
  const handle = row(page, id).locator('[data-role="bookmark-drag"]');
  const box = await handle.boundingBox();
  expect(box, "the drag handle").not.toBeNull();
  if (!box) return;
  const from = { x: box.x + box.width / 2, y: box.y + box.height / 2 };
  if (touch) {
    await touchDrag(page, from, { x: from.x, y }, 10);
  } else {
    await page.mouse.move(from.x, from.y);
    await page.mouse.down();
    await page.mouse.move(from.x, y, { steps: 10 });
    await page.mouse.up();
  }
}

async function rowBox(page: Page, id: number): Promise<{ top: number; bottom: number }> {
  const box = await row(page, id).boundingBox();
  if (!box) throw new Error(`row ${id} has no box`);
  return { top: box.y, bottom: box.y + box.height };
}

async function open(browser: import("@playwright/test").Browser, profile: BrowserContextOptions): Promise<OpenedStudy> {
  const opened = await openStudy(browser, baseURL, profile);
  await emptyList(opened.page);
  return opened;
}

for (const [label, profile, touch] of PROFILES) {
  test(`${label}: a new bookmark goes on top; a compact row shows the place and the verse's text and opens it`, async ({
    browser,
  }) => {
    const opened = await open(browser, profile);
    const { page } = opened;

    // Two bookmarks the way the preacher adds them: verse 2, then verse 5.
    for (const at of [2, 5]) {
      await press(verseNumber(page, at), touch);
      await press(page.locator('[data-role="verse-save-bookmark"]'), touch);
      await expect(notice(page)).toContainText(`Záložka pridaná: Hebrejom 11:${at}`);
      await expect(page.locator('[data-role="verse-actions"]')).toHaveCount(0);
    }
    const [newest, older] = await stored(page);
    expect([newest.verse, older.verse], "the server answers newest first").toEqual([5, 2]);

    // The text the server reads for the row is the verse's own (SEB).
    const chapter = await page.evaluate(async () =>
      (await fetch("/bible/study/chapter?translation=slk-seb&book_code=HEB&chapter=11")).json(),
    );
    const verseText = (at: number): string =>
      chapter.verses.find((item: { verse: number }) => item.verse === at).text as string;

    await openList(page, touch);
    await expect.poll(() => shownIds(page)).toEqual([newest.id, older.id]);
    const top = row(page, newest.id);
    await expect(top.locator(".study-bookmark__label")).toHaveText("Hebrejom 11:5");
    await expect(top.locator(".study-bookmark__tag")).toHaveText("SEB");
    const shownText = (await top.locator('[data-role="bookmark-text"]').textContent()) ?? "";
    expect(verseText(5).startsWith(shownText.replace(/…$/, "")), `"${shownText}" starts verse 5`).toBe(true);
    expect(shownText.length).toBeGreaterThan(20);
    // One line, cut with an ellipsis, never wider than the row.
    const line = await top.locator('[data-role="bookmark-text"]').evaluate((el) => {
      const style = getComputedStyle(el);
      return { nowrap: style.whiteSpace, overflow: style.textOverflow, height: el.getBoundingClientRect().height };
    });
    expect(line).toMatchObject({ nowrap: "nowrap", overflow: "ellipsis" });
    expect(line.height).toBeLessThan(24);
    // Nothing else per row: no rename, no move-here, no delete button.
    await expect(top.locator("button")).toHaveCount(2);
    await expect(sheet(page).locator('[data-role="bookmark-rename"], [data-role="bookmark-move"], [data-role="bookmark-delete"]')).toHaveCount(0);
    if (touch) {
      await assertMinTapTarget(top.locator('[data-role="bookmark-open"]'), 44);
      await assertMinTapTarget(top.locator('[data-role="bookmark-drag"]'), 44);
      await assertNoHorizontalOverflow(page);
    }
    for (const item of await rows(page).all()) {
      const box = await item.boundingBox();
      const viewport = page.viewportSize();
      expect(box && viewport && box.x + box.width <= viewport.width + 0.5, "row inside the screen").toBe(true);
    }

    // The whole row opens the reader at its verse, highlighted.
    await press(top.locator(".study-bookmark__text"), touch);
    await expectChapter(page, "slk-seb:HEB.11");
    await expect(page).toHaveURL(/p=HEB\.11&v=5$/);
    await expect(verse(page, 5)).toHaveAttribute("data-selected", "true");
    await expect(verse(page, 5)).toBeInViewport();
    await finish(opened);
  });

  test(`${label}: rows reorder by a drag on the handle — above the first, below the last, in the middle`, async ({
    browser,
  }) => {
    const opened = await open(browser, profile);
    const { page } = opened;

    // An empty list: the empty note, no "Vybrať", nothing to drag.
    await openList(page, touch);
    await expect(sheet(page).locator('[data-role="bookmarks-empty"]')).toBeVisible();
    await expect(sheet(page).locator('[data-role="bookmarks-choose"]')).toHaveCount(0);
    await expect(sheet(page).locator('[data-role="bookmark-drag"]')).toHaveCount(0);
    await press(sheet(page).locator('[data-role="study-sheet-close"]'), touch);
    await expect(page).not.toHaveURL(/s=zalozky/);

    const [one, two, three, four] = await seed(page, [1, 2, 3, 4]);
    await openList(page, touch);
    await expectOrder(page, [four, three, two, one]);

    // The bottom row above the first.
    await dragTo(page, one, (await rowBox(page, four)).top + 4, touch);
    await expectOrder(page, [one, four, three, two]);

    // The top row below the last.
    await dragTo(page, one, (await rowBox(page, two)).bottom - 4, touch);
    await expectOrder(page, [four, three, two, one]);

    // The top row into the middle, between `three` and `two`.
    const gap = ((await rowBox(page, three)).bottom + (await rowBox(page, two)).top) / 2;
    await dragTo(page, four, gap, touch);
    await expectOrder(page, [three, four, two, one]);

    // A drop on its own place sends nothing and changes nothing.
    const puts: string[] = [];
    page.on("request", (request) => {
      if (request.method() === "PUT" && request.url().includes("/bible/study/bookmarks/order")) puts.push(request.url());
    });
    await dragTo(page, two, (await rowBox(page, two)).top + 6, touch);
    await expect(row(page, two)).not.toHaveAttribute("data-dragging", "true");
    await expectOrder(page, [three, four, two, one]);
    expect(puts).toEqual([]);

    // The order survives a new visit.
    await page.reload();
    await page.waitForSelector('body[data-wasm-ready="true"]', { state: "attached", timeout: 30_000 });
    await expect(sheet(page)).toHaveAttribute("data-state", "ready");
    await expectOrder(page, [three, four, two, one]);
    await finish(opened);
  });

  test(`${label}: Vybrať chooses rows for ONE Zmazať (N); Späť puts them back at their places`, async ({
    browser,
  }) => {
    const opened = await open(browser, profile);
    const { page } = opened;
    const [one, two, three, four, five] = await seed(page, [1, 2, 3, 4, 5]);
    await openList(page, touch);
    const before = [five, four, three, two, one];
    await expectOrder(page, before);

    const choose = sheet(page).locator('[data-role="bookmarks-choose"]');
    const remove = sheet(page).locator('[data-role="bookmarks-delete"]');
    await expect(choose).toHaveText("Vybrať");
    await expect(remove).toHaveCount(0);
    await press(choose, touch);
    await expect(choose).toHaveText("Hotovo");
    await expect(remove).toHaveText("Zmazať (0)");
    await expect(remove).toBeDisabled();
    // Choosing hides the handles and a tap chooses instead of opening.
    await expect(sheet(page).locator('[data-role="bookmark-drag"]')).toHaveCount(0);
    for (const id of [five, three, one]) await press(row(page, id).locator('[data-role="bookmark-pick"]'), touch);
    await press(row(page, three).locator('[data-role="bookmark-pick"]'), touch);
    await press(row(page, three).locator('[data-role="bookmark-pick"]'), touch);
    await expect(page).toHaveURL(/s=zalozky/);
    await expect(remove).toHaveText("Zmazať (3)");
    await expect(row(page, three)).toHaveAttribute("data-chosen", "true");
    await expect(row(page, two)).not.toHaveAttribute("data-chosen", "true");
    if (touch) {
      await assertMinTapTarget(remove, 44);
      await assertMinTapTarget(choose, 44);
      await assertNoHorizontalOverflow(page);
    }
    // The delete button stays put under the list, inside the sheet.
    const removeBox = await remove.boundingBox();
    const sheetBox = await sheet(page).boundingBox();
    expect(removeBox && sheetBox && removeBox.y + removeBox.height <= sheetBox.y + sheetBox.height + 0.5).toBe(true);

    await press(remove, touch);
    await expect(notice(page)).toContainText("Zmazané 3 záložky");
    await expectOrder(page, [four, two]);
    await expect(choose).toHaveText("Vybrať");
    await expect(remove).toHaveCount(0);

    // Späť: each at its old place, the whole list as it was.
    await press(page.locator('[data-role="study-notice-undo"]'), touch);
    await expect(notice(page)).toContainText("Záložky sú späť");
    await expect.poll(async () => (await stored(page)).map((item) => item.verse)).toEqual([5, 4, 3, 2, 1]);
    await expect.poll(async () => (await shownIds(page)).length).toBe(5);

    // "Hotovo" leaves choosing with nothing deleted.
    await press(choose, touch);
    await press(rows(page).first().locator('[data-role="bookmark-pick"]'), touch);
    await press(choose, touch);
    await expect(remove).toHaveCount(0);
    await expect(sheet(page).locator('[data-role="bookmark-drag"]')).toHaveCount(5);
    expect((await stored(page)).length).toBe(5);
    await finish(opened);
  });
}

test("Späť that fails half-way keeps Späť for the rest, and a second Späť brings every row back to its place", async ({
  browser,
}) => {
  const opened = await open(browser, PHONE_390);
  const { page } = opened;
  const [one, two, three] = await seed(page, [1, 2, 3]);
  await openList(page, true);
  await expectOrder(page, [three, two, one]);
  await page.locator('[data-role="bookmarks-choose"]').tap();
  for (const id of [three, one]) await row(page, id).locator('[data-role="bookmark-pick"]').tap();
  await page.locator('[data-role="bookmarks-delete"]').tap();
  await expect(notice(page)).toContainText("Zmazané 2 záložky");
  await expectOrder(page, [two]);

  // The second re-create fails once (the server is away for a moment).
  let creates = 0;
  await page.route("**/bible/study/bookmarks", async (route) => {
    if (route.request().method() !== "POST") return route.continue();
    creates += 1;
    if (creates === 2) {
      return route.fulfill({ status: 503, contentType: "application/json", body: JSON.stringify({ message: "Server je dočasne nedostupný" }) });
    }
    return route.continue();
  });
  await page.locator('[data-role="study-notice-undo"]').tap();
  await expect(notice(page)).toContainText("Server je dočasne nedostupný");
  // The first is back at the top; the one that failed still has its Späť.
  await expect.poll(async () => (await stored(page)).map((item) => item.verse)).toEqual([3, 2]);
  await page.locator('[data-role="study-notice-undo"]').tap();
  await expect(notice(page)).toContainText("Záložka je späť");
  await expect.poll(async () => (await stored(page)).map((item) => item.verse)).toEqual([3, 2, 1]);
  await expect.poll(async () => (await shownIds(page)).length).toBe(3);
  expect(creates).toBe(3);

  const deliberate = /Failed to load resource: the server responded with a status of 503\b/;
  expect(opened.errors.filter((text) => deliberate.test(text))).toHaveLength(1);
  opened.errors.splice(0, opened.errors.length, ...opened.errors.filter((text) => !deliberate.test(text)));
  await finish(opened);
});

test("the old rename and move routes are gone; an empty delete is refused in Slovak", async ({ browser }) => {
  const opened = await open(browser, PHONE_390);
  const { page } = opened;
  const [id] = await seed(page, [1]);
  const answers = await page.evaluate(async (id) => {
    const patch = await fetch(`/bible/study/bookmarks/${id}`, {
      method: "PATCH",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ name: "Nedeľa" }),
    });
    const empty = await fetch("/bible/study/bookmarks/delete", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ ids: [] }),
    });
    return { patch: patch.status, empty: { status: empty.status, body: await empty.json() } };
  }, id);
  expect([404, 405]).toContain(answers.patch);
  expect(answers.empty).toEqual({ status: 422, body: { message: "Nie je čo zmazať" } });
  const deliberate = /Failed to load resource: the server responded with a status of (404|405|422)\b/;
  expect(opened.errors.filter((text) => deliberate.test(text))).toHaveLength(2);
  opened.errors.splice(0, opened.errors.length, ...opened.errors.filter((text) => !deliberate.test(text)));
  expect((await stored(page)).map((item) => item.name)).toEqual(["Hebrejom 11:1"]);
  await finish(opened);
});
