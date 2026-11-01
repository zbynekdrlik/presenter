/**
 * #851 E3 — the sermon's order, its stored slides, HOTOVO, another device's
 * write reaching an open editor, and the last tap going out with a closing tab.
 * Nothing reaches a wall (T11).
 */
import { test, expect, type Page } from "@playwright/test";
import {
  assertNoHorizontalOverflow,
  deriveTestConfig,
  PHONE_390,
  refreshDevData,
  startTestServer,
  stopServer,
  touchDrag,
  type ServerHandle,
} from "./support";
import { finish, openStudy } from "./study-support";

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

const role = (page: Page, name: string) => page.locator(`[data-role="${name}"]`);

type Detail = {
  revision: number;
  items: { id: string; label: string; verseStart: number; edits: Record<string, { deleted: number[] }> }[];
  slides: { mainReference: string; mainText: string }[];
};

async function newSermon(title: string): Promise<string> {
  const response = await fetch(`${baseURL}/bible/sermons`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ title, serviceDate: "2030-07-07", defaultTranslation: "slk-seb" }),
  });
  expect(response.status).toBe(200);
  return ((await response.json()) as { id: string }).id;
}

const sermonOf = async (id: string) =>
  (await (await fetch(`${baseURL}/bible/sermons/${id}`)).json()) as Detail;

async function addVerse(id: string, verse: number) {
  const { revision } = await sermonOf(id);
  const response = await fetch(`${baseURL}/bible/sermons/${id}/items`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      expectedRevision: revision,
      translation: "slk-seb",
      bookCode: "HEB",
      chapter: 11,
      verseStart: verse,
      verseEnd: verse,
    }),
  });
  expect(response.status).toBe(200);
}

const versesOf = async (id: string) => (await sermonOf(id)).items.map((item) => item.verseStart);

async function openReorder(id: string, viewport = PHONE_390) {
  const opened = await openStudy(browser0, baseURL, viewport, `/ui/study?view=sermon&id=${id}`, null);
  const { page } = opened;
  await expect(role(page, "sermon-editor")).toHaveAttribute("data-state", "ready");
  await role(page, "sermon-reorder-open").click();
  await expect(role(page, "sermon-reorder")).toBeVisible();
  return opened;
}

// `openStudy` takes the browser; each test sets it before calling `openReorder`.
let browser0: Parameters<typeof openStudy>[0];
test.beforeEach(({ browser }) => {
  browser0 = browser;
});

async function drag(page: Page, from: number, y: (rows: { top: number; bottom: number }[]) => number) {
  const rows = role(page, "sermon-order-row");
  const boxes = [];
  for (let i = 0; i < (await rows.count()); i += 1) {
    const box = (await rows.nth(i).boundingBox())!;
    boxes.push({ top: box.y, bottom: box.y + box.height });
  }
  const handle = (await role(page, "sermon-drag-handle").nth(from).boundingBox())!;
  await page.mouse.move(handle.x + handle.width / 2, handle.y + handle.height / 2);
  await page.mouse.down();
  await page.mouse.move(handle.x + handle.width / 2, y(boxes), { steps: 6 });
  await page.mouse.up();
}

/// The POSTs from here on: a reorder is the only one these tests can cause.
const postsOf = (page: Page) => {
  const posts: string[] = [];
  page.on("request", (request) => {
    if (request.method() === "POST") posts.push(new URL(request.url()).pathname);
  });
  return posts;
};

/// A write that must NOT happen has no settle marker: only the wait can show it.
const nothingFollows = (page: Page) => page.waitForTimeout(800);

async function reorderElsewhere(id: string, itemIds: string[]) {
  const { revision } = await sermonOf(id);
  const response = await fetch(`${baseURL}/bible/sermons/${id}/items/reorder`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ expectedRevision: revision, itemIds }),
  });
  expect(response.status).toBe(200);
}

const closeSheet = (page: Page) =>
  role(page, "sermon-reorder").locator('[data-role="study-sheet-close"]').click();

const seeded = async (title: string) => {
  const id = await newSermon(title);
  for (const verse of [1, 2, 3, 4]) await addVerse(id, verse);
  return id;
};

test("an empty sermon has no handles and leaving sends nothing", async () => {
  const id = await newSermon("Prázdna");
  const opened = await openReorder(id);
  const { page } = opened;
  await expect(role(page, "sermon-order-empty")).toHaveText("Zatiaľ žiadne položky");
  await expect(role(page, "sermon-drag-handle")).toHaveCount(0);
  const posts = postsOf(page);
  await closeSheet(page);
  await expect(role(page, "sermon-reorder")).toHaveCount(0);
  await nothingFollows(page);
  expect(posts).toEqual([]);
  await finish(opened);
});

test("a drag above the first row goes first", async () => {
  const id = await seeded("Nad prvú");
  const opened = await openReorder(id);
  await drag(opened.page, 2, (rows) => rows[0].top + 2);
  await closeSheet(opened.page);
  await expect.poll(() => versesOf(id)).toEqual([3, 1, 2, 4]);
  await expect(role(opened.page, "sermon-item").first()).toHaveAttribute("data-label", "Hebrejom 11:3 (SEB)");
  await finish(opened);
});

test("a drag below the last row goes last", async () => {
  const id = await seeded("Pod poslednú");
  const opened = await openReorder(id);
  await drag(opened.page, 0, (rows) => rows[3].bottom - 2);
  await closeSheet(opened.page);
  await expect.poll(() => versesOf(id)).toEqual([2, 3, 4, 1]);
  await finish(opened);
});

test("a drag into the middle lands there, and a finger does the same", async () => {
  const id = await seeded("Do stredu");
  const opened = await openReorder(id);
  const { page } = opened;
  await drag(page, 3, (rows) => rows[1].top + 2);
  await expect(role(page, "sermon-order-row").nth(1).locator(".sermon-order__label")).toContainText("11:4");

  const handle = (await role(page, "sermon-drag-handle").nth(0).boundingBox())!;
  const target = (await role(page, "sermon-order-row").nth(2).boundingBox())!;
  const x = handle.x + handle.width / 2;
  await touchDrag(page, { x, y: handle.y + handle.height / 2 }, { x, y: target.y + target.height - 2 });
  await closeSheet(page);
  await expect.poll(() => versesOf(id)).toEqual([4, 2, 1, 3]);
  await assertNoHorizontalOverflow(page);
  await finish(opened);
});

test("a drop on its own place sends nothing", async () => {
  const id = await seeded("Na mieste");
  const opened = await openReorder(id);
  const { page } = opened;
  const posts = postsOf(page);
  await drag(page, 1, (rows) => rows[1].top + 4);
  await closeSheet(page);
  await nothingFollows(page);
  expect(posts).toEqual([]);
  await finish(opened);
});

test("an order changed elsewhere is not undone by an open sheet", async () => {
  const id = await seeded("Dve poradia");
  const opened = await openReorder(id);
  const { page } = opened;
  await expect(role(page, "sermon-editor")).toHaveAttribute("data-live", "true");
  const posts = postsOf(page);
  const items = (await sermonOf(id)).items.map((item) => item.id);
  const moved = [items[1], items[0], items[2], items[3]];
  await reorderElsewhere(id, moved);
  await expect(role(page, "sermon-item").first()).toHaveAttribute("data-label", "Hebrejom 11:2 (SEB)");

  // Nothing touched here: the sheet just closes.
  await closeSheet(page);
  await nothingFollows(page);
  expect(posts).toEqual([]);
  await expect.poll(() => versesOf(id)).toEqual([2, 1, 3, 4]);

  // Moved here on the old order: named, never sent.
  await role(page, "sermon-reorder-open").click();
  const third = role(page, "sermon-order-row").nth(2);
  await third.locator('[data-role="sermon-order-up"]').click();
  await reorderElsewhere(id, items);
  await expect(role(page, "sermon-item").first()).toHaveAttribute("data-label", "Hebrejom 11:1 (SEB)");
  await closeSheet(page);
  await expect(role(page, "study-notice")).toContainText("Poradie sa medzitým zmenilo");
  await nothingFollows(page);
  expect(posts).toEqual([]);
  expect(await versesOf(id)).toEqual([1, 2, 3, 4]);
  await finish(opened);
});

test("an order that failed to go out waits for its retry, and HOTOVO says so", async () => {
  const id = await seeded("Výpadok poradia");
  const opened = await openReorder(id);
  const { page } = opened;
  await role(page, "sermon-order-row").nth(1).locator('[data-role="sermon-order-up"]').click();
  const reorder = /\/items\/reorder$/;
  await page.route(reorder, (route) => route.abort());
  await closeSheet(page);
  await expect(role(page, "sermon-order-retry")).toBeVisible();

  await role(page, "sermon-done").click();
  await expect(role(page, "study-notice")).toContainText("Poradie sa neuložilo");
  await expect(role(page, "sermon-editor")).toBeVisible();

  await page.unroute(reorder);
  await role(page, "sermon-order-retry").click();
  await expect.poll(() => versesOf(id)).toEqual([2, 1, 3, 4]);
  await expect(role(page, "sermon-order-retry")).toHaveCount(0);
  opened.errors.splice(0, opened.errors.length, ...opened.errors.filter((line) => !line.includes("ERR_FAILED")));
  await finish(opened);
});

test("Poradie opened from its URL lists the sermon", async ({ browser }) => {
  const id = await seeded("Z odkazu");
  const opened = await openStudy(browser, baseURL, PHONE_390, `/ui/study?view=sermon&id=${id}&s=poradie`, null);
  const { page } = opened;
  await expect(role(page, "sermon-order-row")).toHaveCount(4);
  const posts = postsOf(page);
  await closeSheet(page);
  await nothingFollows(page);
  expect(posts).toEqual([]);
  await finish(opened);
});

test("a sermon deleted elsewhere offers no HOTOVO", async ({ browser }) => {
  const id = await newSermon("Zmazaná");
  await addVerse(id, 1);
  const opened = await openStudy(browser, baseURL, PHONE_390, `/ui/study?view=sermon&id=${id}`, null);
  const { page } = opened;
  await expect(role(page, "sermon-done")).toBeVisible();
  await expect(role(page, "sermon-editor")).toHaveAttribute("data-live", "true");
  expect((await fetch(`${baseURL}/bible/sermons/${id}`, { method: "DELETE" })).status).toBe(204);
  await expect(role(page, "sermon-missing")).toBeVisible();
  await expect(role(page, "sermon-done")).toHaveCount(0);
  opened.errors.splice(0, opened.errors.length, ...opened.errors.filter((line) => !line.includes("404")));
  await finish(opened);
});

test("▲ ▼ move a step and Back sends the order", async () => {
  const id = await seeded("Šípky");
  const opened = await openReorder(id);
  const { page } = opened;
  await expect(role(page, "sermon-order-row").first().locator('[data-role="sermon-order-up"]')).toBeDisabled();
  await expect(role(page, "sermon-order-row").last().locator('[data-role="sermon-order-down"]')).toBeDisabled();

  const posts = postsOf(page);
  await role(page, "sermon-order-row").nth(1).locator('[data-role="sermon-order-up"]').click();
  await role(page, "sermon-order-row").nth(1).locator('[data-role="sermon-order-down"]').click();
  await page.goBack();
  await expect(role(page, "sermon-reorder")).toHaveCount(0);
  await expect.poll(() => versesOf(id)).toEqual([2, 3, 1, 4]);
  expect(posts).toHaveLength(1);
  await finish(opened);
});

test("a reorder that lost to another device says so and keeps the server's order", async () => {
  const id = await seeded("Súbeh");
  const opened = await openReorder(id);
  const { page } = opened;
  await role(page, "sermon-order-row").nth(1).locator('[data-role="sermon-order-up"]').click();
  await page.route(/\/items\/reorder$/, async (route) => {
    await addVerse(id, 5);
    await route.continue();
  });
  await closeSheet(page);
  await expect(role(page, "study-notice")).toContainText("Poradie sa medzitým zmenilo");
  expect(await versesOf(id)).toEqual([1, 2, 3, 4, 5]);
  await expect(role(page, "sermon-item")).toHaveCount(5);
  // The refused write's own console line.
  opened.errors.splice(0, opened.errors.length, ...opened.errors.filter((line) => !line.includes("409")));
  await finish(opened);
});

test("Náhľad shows the stored slide with its count; a missing secondary line can be asked for again", async ({ browser }) => {
  const id = await newSermon("Náhľad");
  await addVerse(id, 1);
  const opened = await openStudy(browser, baseURL, PHONE_390, `/ui/study?view=sermon&id=${id}`, null);
  const { page } = opened;
  const answers = new RegExp(`/bible/sermons/${id}(/items/[^/]+/edits)?$`);
  await page.route(answers, async (route) => {
    const answer = await route.fetch();
    await route.fulfill({ response: answer, json: { ...(await answer.json()), secondaryMissing: true } });
  });
  await page.reload();
  await page.locator('[data-role="sermon-token"][data-verse="1"][data-index="0"]').tap();
  await expect(role(page, "sermon-save")).toHaveAttribute("data-save-state", "saved");
  await role(page, "sermon-preview-open").tap();
  await expect(role(page, "sermon-preview-unsaved")).toHaveCount(0);

  const stored = await sermonOf(id);
  const slide = role(page, "sermon-slide");
  await expect(slide).toHaveCount(stored.slides.length);
  await expect(role(page, "sermon-slide-label")).toHaveText("Hebrejom 11:1b (SEB)");
  await expect(role(page, "sermon-slide-main")).toHaveText(stored.slides[0].mainText.replace(/\n/g, " "));
  await expect(slide.locator('[data-role="slide-char-count"][data-field="main"]')).toHaveText(
    String([...stored.slides[0].mainText].length),
  );
  await assertNoHorizontalOverflow(page);

  await expect(role(page, "sermon-preview-warning")).toBeVisible();
  await page.unroute(answers);
  await role(page, "sermon-regenerate").tap();
  await expect(role(page, "sermon-preview-warning")).toHaveCount(0);
  expect((await sermonOf(id)).revision).toBe(stored.revision + 1);

  await page.goBack();
  await expect(role(page, "sermon-preview")).toHaveCount(0);

  // A tap whose save fails: the sheet says these slides lack it.
  const edits = /\/edits$/;
  await page.route(edits, (route) => route.abort());
  await page.locator('[data-role="sermon-token"][data-verse="1"][data-index="1"]').tap();
  await expect(role(page, "sermon-save")).toHaveAttribute("data-save-state", "error");
  await role(page, "sermon-preview-open").tap();
  await expect(role(page, "sermon-preview-unsaved")).toContainText("neuložila");
  await page.unroute(edits);
  opened.errors.splice(0, opened.errors.length, ...opened.errors.filter((line) => !line.includes("ERR_FAILED")));
  await finish(opened);
});

test("HOTOVO returns to the list and says where the sermon went", async ({ browser }) => {
  const id = await newSermon("Hotovo");
  await addVerse(id, 1);
  const opened = await openStudy(browser, baseURL, PHONE_390, `/ui/study?view=sermon&id=${id}`, null);
  const { page } = opened;
  await page.locator('[data-role="sermon-token"][data-verse="1"][data-index="0"]').tap();
  await role(page, "sermon-done").tap();
  await expect(role(page, "sermon-list")).toHaveAttribute("data-state", "ready");
  await expect(role(page, "study-notice")).toContainText("Pripravené kazateľom");
  expect((await sermonOf(id)).items[0].edits["1"].deleted, "saved before it said so").toEqual([0]);
  await finish(opened);
});

test("another device's write appears in an open editor by itself", async ({ browser }) => {
  const id = await newSermon("Dve obrazovky");
  await addVerse(id, 1);
  const opened = await openStudy(browser, baseURL, PHONE_390, `/ui/study?view=sermon&id=${id}`, null);
  const { page } = opened;
  await expect(role(page, "sermon-item")).toHaveCount(1);
  await expect(role(page, "sermon-counts")).toContainText("1 položka");
  // The hub never replays: the write comes once the socket has delivered a frame.
  await expect(role(page, "sermon-editor")).toHaveAttribute("data-live", "true");
  await addVerse(id, 2);
  await expect(role(page, "sermon-item")).toHaveCount(2);
  await finish(opened);
});

test("the last tap goes out with a closing tab", async ({ browser }) => {
  const id = await newSermon("Zatvorená karta");
  await addVerse(id, 1);
  const opened = await openStudy(browser, baseURL, PHONE_390, `/ui/study?view=sermon&id=${id}`, null);
  const { page } = opened;
  await expect(role(page, "sermon-item")).toBeVisible();
  // A plain fetch dies with the tab: this one must ask to outlive it.
  await page.evaluate(() => {
    const original = window.fetch.bind(window);
    window.fetch = (input, init) => {
      if ((input as Request).method === "PUT") {
        (window as unknown as { keepalive?: boolean }).keepalive = (input as Request).keepalive;
      }
      return original(input, init);
    };
  });
  await page.locator('[data-role="sermon-token"][data-index="1"]').first().tap();
  await expect(role(page, "sermon-save")).toHaveAttribute("data-save-state", "saved");
  expect(await page.evaluate(() => (window as unknown as { keepalive?: boolean }).keepalive)).toBe(true);
  await page.evaluate(() => {
    (document.querySelector('[data-role="sermon-token"]') as HTMLElement).click();
  });
  await page.close();
  await expect.poll(async () => (await sermonOf(id)).items[0].edits["1"]?.deleted).toEqual([0, 1]);
  expect(opened.errors).toEqual([]);
  await opened.context.close();
});
