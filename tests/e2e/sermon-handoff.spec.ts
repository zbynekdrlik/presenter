/**
 * #851 H1 — the sermon reaches the service without a middleman: the operator's
 * console lists today's sermon and plays its stored slides in the prepared
 * order, the Prepared tab shows it read-only, the tablet opens on it.
 */
import { test, expect, type Browser, type Page } from "@playwright/test";
import {
  assertMinTapTarget,
  assertNoHorizontalOverflow,
  attachConsoleErrorCollector,
  deriveTestConfig,
  PHONE_390,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

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

type Item = { id: string; label: string; slideIds: string[]; verseStart: number };
type Slide = { id: string; mainText: string; mainReference: string };
type Detail = { id: string; revision: number; items: Item[]; slides: Slide[] };

const json = async <T>(path: string, method = "GET", body?: unknown): Promise<T> => {
  const response = await fetch(`${baseURL}${path}`, {
    method,
    headers: body === undefined ? {} : { "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  expect(response.ok, `${method} ${path}: ${response.status}`).toBeTruthy();
  return (await response.json()) as T;
};

const sermonOf = (id: string) => json<Detail>(`/bible/sermons/${id}`);

async function addVerse(id: string, verse: number) {
  const { revision } = await sermonOf(id);
  await json(`/bible/sermons/${id}/items`, "POST", {
    expectedRevision: revision,
    translation: "slk-seb",
    bookCode: "HEB",
    chapter: 11,
    verseStart: verse,
    verseEnd: verse,
  });
}

/** A sermon of `serviceDate` (default: the server's today) with these verses. */
async function sermon(title: string, verses: number[], serviceDate?: string): Promise<string> {
  const date = serviceDate ?? (await json<{ today: string }>("/bible/sermons")).today;
  const { id } = await json<{ id: string }>("/bible/sermons", "POST", {
    title,
    serviceDate: date,
    defaultTranslation: "slk-seb",
  });
  for (const verse of verses) await addVerse(id, verse);
  return id;
}

/** Strike the first word of an item's verse. */
async function cutFirstWord(id: string, item: Item) {
  const { revision } = await sermonOf(id);
  await json(`/bible/sermons/${id}/items/${item.id}/edits`, "PUT", {
    expectedRevision: revision,
    verses: { [String(item.verseStart)]: { deleted: [0], emphasized: [] } },
  });
}

const removeSermons = async () => {
  const today = await json<Detail[]>("/bible/sermons/today");
  for (const { id } of today) {
    expect((await fetch(`${baseURL}/bible/sermons/${id}`, { method: "DELETE" })).status).toBe(204);
  }
};

test.beforeEach(async () => {
  await removeSermons();
  const prefs = await fetch(`${baseURL}/bible/preferences`, {
    method: "PUT",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ mainTranslation: "slk-seb", characterLimit: 320 }),
  });
  expect(prefs.ok).toBeTruthy();
});

async function openOperator(browser: Browser, viewport = PHONE_390) {
  const context = await browser.newContext({ ...viewport, baseURL });
  const page = await context.newPage();
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  await page.goto("/ui/operator/bible");
  await page.waitForSelector('body[data-wasm-ready="true"]', { state: "attached", timeout: 30_000 });
  await expect(role(page, "book-list")).toHaveAttribute("data-books-translation", "slk-seb");
  await expect(role(page, "console-field")).toBeVisible();
  return { context, page, errors };
}

const onWall = async () =>
  (await json<{ output?: { mainText?: string } | null; mainText?: string } | null>("/bible/active-slide"));

test("a tap previews the stored text, ZOBRAZIŤ and ĎALŠÍ walk the prepared order", async ({ browser }) => {
  const id = await sermon("Viera", [1, 2]);
  const before = await sermonOf(id);
  await cutFirstWord(id, before.items[0]);
  const stored = await sermonOf(id);
  const { context, page, errors } = await openOperator(browser);
  const triggers: string[] = [];
  const resolved: string[] = [];
  page.on("request", (request) => {
    const path = new URL(request.url()).pathname;
    if (path.endsWith("/trigger")) triggers.push(path);
    if (path === "/bible/trigger-slide") triggers.push(path);
    if (path === "/bible/resolve") resolved.push(path);
  });

  await expect(role(page, "console-prepared")).toBeVisible();
  await expect(role(page, "console-prepared-title")).toHaveText("Viera");
  const rows = role(page, "console-prepared-row");
  await expect(rows).toHaveCount(2);
  await expect(rows.first()).toContainText(stored.items[0].label);
  await assertMinTapTarget(rows, 44);
  await assertNoHorizontalOverflow(page);

  await rows.first().click();
  await expect(role(page, "bible-console")).toHaveAttribute("data-preview-ready", "true");
  await expect(role(page, "console-preview-main")).toHaveText(stored.slides[0].mainText);
  await expect(rows.first()).toHaveAttribute("aria-pressed", "true");
  expect(triggers, "a tap never shows").toEqual([]);
  expect(resolved, "the stored slide, never the whole verse").toEqual([]);

  await role(page, "console-show").click();
  await expect(role(page, "console-show")).toContainText("ĎALŠÍ:");
  await expect(role(page, "console-preview-main")).toHaveText(stored.slides[1].mainText);
  await expect(rows.nth(1)).toHaveAttribute("aria-pressed", "true");
  expect(triggers).toEqual([`/bible/presentations/${id}/slides/${stored.slides[0].id}/trigger`]);
  expect(JSON.stringify(await onWall())).toContain(stored.slides[0].mainText.slice(0, 30));

  await role(page, "console-show").click();
  await expect(role(page, "console-show")).toHaveText("KONIEC KÁZNE");
  expect(triggers).toHaveLength(2);
  expect(JSON.stringify(await onWall())).toContain(stored.slides[1].mainText.slice(0, 30));

  await role(page, "console-prev").click();
  await expect(role(page, "console-preview-main")).toHaveText(stored.slides[0].mainText);
  expect(errors).toEqual([]);
  await context.close();
});

test("typing hides the section and leaves the sermon, an emptied field brings it back", async ({ browser }) => {
  await sermon("Písanie", [1]);
  const { context, page, errors } = await openOperator(browser);
  await role(page, "console-prepared-row").click();
  await expect(role(page, "bible-console")).toHaveAttribute("data-preview-ready", "true");
  await role(page, "console-field").fill("zid 11 3");
  await expect(role(page, "console-prepared")).toHaveCount(0);
  await expect(role(page, "bible-console")).toHaveAttribute("data-preview-reference", "Hebrejom 11:3 (SEB)");
  await role(page, "console-field").fill("");
  await expect(role(page, "console-prepared")).toBeVisible();
  await expect(role(page, "console-prepared-row")).toHaveAttribute("aria-pressed", "false");
  expect(errors).toEqual([]);
  await context.close();
});

test("a load from the old inputs lets go of the sermon: ZOBRAZIŤ can never play it", async ({ browser }) => {
  await sermon("Staré polia", [1]);
  const { context, page, errors } = await openOperator(browser, { viewport: { width: 1280, height: 800 } } as never);
  await role(page, "console-prepared-row").click();
  await expect(role(page, "bible-console")).toHaveAttribute("data-preview-ready", "true");
  await role(page, "book-filter").fill("Rimanom");
  await role(page, "book-item").first().click();
  await role(page, "load-button").click();
  await expect(role(page, "slide-card").first()).toContainText("Rimanom 1:1");
  await expect(role(page, "console-prepared-row")).toHaveAttribute("aria-pressed", "false");
  await expect(role(page, "bible-console")).toHaveAttribute("data-preview-state", "empty");
  await expect(role(page, "console-show")).toBeDisabled();
  expect(errors).toEqual([]);
  await context.close();
});

test("a verse the preacher adds or cuts reaches an open console, a sermon of another day does not", async ({ browser }) => {
  const id = await sermon("Živá", [1]);
  await sermon("Inokedy", [2], "2031-03-02");
  const { context, page, errors } = await openOperator(browser);
  await expect(role(page, "console-prepared-title")).toHaveCount(1);
  await role(page, "console-prepared-row").click();
  await expect(role(page, "bible-console")).toHaveAttribute("data-preview-ready", "true");

  await addVerse(id, 2);
  await expect(role(page, "console-prepared-row")).toHaveCount(2);
  const item = (await sermonOf(id)).items[0];
  await cutFirstWord(id, item);
  const stored = await sermonOf(id);
  await expect(role(page, "console-preview-main")).toHaveText(stored.slides[0].mainText);
  expect(errors).toEqual([]);
  await context.close();
});

test("a sermon deleted while it is picked leaves nothing to show", async ({ browser }) => {
  const id = await sermon("Zmazaná", [1]);
  const { context, page, errors } = await openOperator(browser);
  await role(page, "console-prepared-row").click();
  await expect(role(page, "bible-console")).toHaveAttribute("data-preview-ready", "true");
  expect((await fetch(`${baseURL}/bible/sermons/${id}`, { method: "DELETE" })).status).toBe(204);
  await expect(role(page, "console-prepared")).toHaveCount(0);
  await expect(role(page, "console-show")).toBeDisabled();
  expect(errors).toEqual([]);
  await context.close();
});

test("the Prepared tab shows a sermon read-only and a card still plays it", async ({ browser }) => {
  const id = await sermon("Na čítanie", [1], "2031-03-09");
  const stored = await sermonOf(id);
  const { context, page, errors } = await openOperator(browser, { viewport: { width: 1280, height: 800 } } as never);
  await page.locator('[data-role="bible-tab"][data-tab="prepared"]').click();
  await page.locator(`[data-role="presentation-card"][data-presentation-id="${id}"]`).click();
  await expect(role(page, "presentation-sermon-badge")).toHaveText(/^Kázeň · ne 9\. 3\.$/);
  await expect(role(page, "sermon-badge")).toHaveText(/^Kázeň · ne 9\. 3\.$/);
  await expect(role(page, "sermon-edit-link")).toHaveAttribute("href", `/ui/study?view=sermon&id=${id}`);
  await expect(role(page, "add-empty-slide")).toHaveCount(0);

  await page.locator('[data-role="mode-toggle"][data-mode="edit"]').first().click();
  const card = role(page, "sermon-slide-card");
  await expect(card).toHaveCount(1);
  await expect(card.locator("textarea, input")).toHaveCount(0);
  await expect(role(page, "slide-delete")).toHaveCount(0);
  await expect(role(page, "slide-drag-handle")).toHaveCount(0);

  await card.click();
  await expect.poll(async () => JSON.stringify(await onWall())).toContain(stored.slides[0].mainText.slice(0, 30));
  expect(errors).toEqual([]);
  await context.close();
  expect((await fetch(`${baseURL}/bible/sermons/${id}`, { method: "DELETE" })).status).toBe(204);
});

test("the tablet opens on today's sermon and redraws a later cut", async ({ browser }) => {
  await json("/bible/presentations", "POST", { name: "0 prvá podľa mena" });
  const id = await sermon("Dnešná", [1]);
  const context = await browser.newContext({ viewport: { width: 1024, height: 768 }, baseURL });
  const page = await context.newPage();
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  await page.goto("/ui/tablet");
  const row = page.locator(`[data-role="presentation-button"][data-presentation-id="${id}"]`);
  await expect(row).toHaveAttribute("data-active", "true");
  await expect(row.locator('[data-role="presentation-sermon"]')).toContainText("Kázeň ·");
  await cutFirstWord(id, (await sermonOf(id)).items[0]);
  const stored = await sermonOf(id);
  await expect(page.getByText(stored.slides[0].mainText, { exact: false }).first()).toBeVisible({ timeout: 15_000 });
  expect(errors).toEqual([]);
  await context.close();
});

test("a hung sermon read leaves the tablet's list usable", async ({ browser }) => {
  const { id } = await json<{ id: string }>("/bible/presentations", "POST", { name: "Bez kázne" });
  const context = await browser.newContext({ viewport: { width: 1024, height: 768 }, baseURL });
  const page = await context.newPage();
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  await page.route("**/bible/sermons/today**", () => undefined);
  await page.goto("/ui/tablet");
  const row = page.locator(`[data-role="presentation-button"][data-presentation-id="${id}"]`);
  await row.click();
  await expect(row).toHaveAttribute("data-active", "true");
  expect(errors).toEqual([]);
  await context.close();
});
