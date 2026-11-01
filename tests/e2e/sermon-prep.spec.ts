/**
 * #851 E1 + E2 — the preacher's path with no middleman: a verse in the reader
 * goes to a sermon, is cut with taps, is saved, and the stored slides carry the
 * truthful reference. Nothing reaches a wall (T11).
 */
import { test, expect, type Page } from "@playwright/test";
import {
  assertNoHorizontalOverflow,
  deriveTestConfig,
  PHONE_390,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";
import { field, finish, openStudy, verseNumber } from "./study-support";

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
const token = (page: Page, verse: number, index: number) =>
  page.locator(`[data-role="sermon-token"][data-verse="${verse}"][data-index="${index}"]`);
const saved = (page: Page) =>
  expect(role(page, "sermon-save")).toHaveAttribute("data-save-state", "saved");

async function newSermon(title: string, serviceDate = "2030-06-02"): Promise<string> {
  const response = await fetch(`${baseURL}/bible/sermons`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ title, serviceDate, defaultTranslation: "slk-seb" }),
  });
  expect(response.status).toBe(200);
  return ((await response.json()) as { id: string }).id;
}

const sermonOf = async (id: string) =>
  (await (await fetch(`${baseURL}/bible/sermons/${id}`)).json()) as {
    revision: number;
    items: { id: string; label: string; edits: Record<string, { deleted: number[]; emphasized: number[] }> }[];
    slides: { mainReference: string; mainText: string }[];
  };

const addVerse = async (id: string, verseStart: number, verseEnd: number) => {
  const { revision } = await sermonOf(id);
  const response = await fetch(`${baseURL}/bible/sermons/${id}/items`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      expectedRevision: revision,
      translation: "slk-seb",
      bookCode: "HEB",
      chapter: 11,
      verseStart,
      verseEnd,
    }),
  });
  expect(response.status).toBe(200);
};

test("a verse goes from the reader to a new sermon and is cut into a truthful slide", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390);
  const { page } = opened;
  await verseNumber(page, 1).tap();
  await role(page, "verse-add-to-sermon").tap();

  await expect(role(page, "add-to-sermon")).toBeVisible();
  await role(page, "add-to-new-sermon").tap();
  await expect(role(page, "study-notice")).toContainText("Pridané do kázne");

  await role(page, "study-sermons").tap();
  await expect(role(page, "sermon-list")).toHaveAttribute("data-state", "ready");
  const row = role(page, "sermon-row").first();
  await expect(row).toHaveAttribute("data-active", "true");
  await expect(row).toContainText("1 položka");
  await row.locator('[data-role="sermon-open"]').tap();

  await expect(role(page, "sermon-editor")).toHaveAttribute("data-state", "ready");
  const item = role(page, "sermon-item");
  await expect(item).toHaveAttribute("data-label", "Hebrejom 11:1 (SEB)");

  // "a zdôvodnením toho, čo nevidíme," goes with one tap and Do konca verša.
  await token(page, 1, 7).tap();
  await role(page, "sermon-cut-to-end").tap();
  await expect(token(page, 1, 11)).toHaveAttribute("data-state", "struck");
  await expect(item).toHaveAttribute("data-label", "Hebrejom 11:1a (SEB)");

  await role(page, "sermon-mode").locator('[data-mode="emphasize"]').tap();
  await token(page, 1, 0).tap();
  await expect(token(page, 1, 0)).toHaveAttribute("data-state", "emphasized");
  await expect(token(page, 1, 0)).toHaveText("VIERA");
  await saved(page);
  await assertNoHorizontalOverflow(page);

  // The server's own slide: the text the wall would get.
  const id = new URL(page.url()).searchParams.get("id") as string;
  const stored = await sermonOf(id);
  expect(stored.slides).toHaveLength(1);
  expect(stored.slides[0].mainReference).toBe("Hebrejom 11:1a (SEB)");
  expect(stored.slides[0].mainText).toContain("VIERA je zárukou toho, v čo dúfame");

  // A reload shows what was saved.
  await page.reload();
  await expect(role(page, "sermon-item")).toHaveAttribute("data-label", "Hebrejom 11:1a (SEB)");
  await expect(token(page, 1, 7)).toHaveAttribute("data-state", "struck");
  await finish(opened);
});

test("the active sermon takes the next verse at once and Späť takes it back", async ({ browser }) => {
  const id = await newSermon("Aktívna");
  await addVerse(id, 3, 3);
  const opened = await openStudy(browser, baseURL, PHONE_390, undefined, undefined, async (page) => {
    await page.addInitScript((sermon) => {
      localStorage.setItem("presenter:study.activeSermon", JSON.stringify(sermon));
    }, id);
  });
  const { page } = opened;
  await verseNumber(page, 2).tap();
  await role(page, "verse-add-to-sermon").tap();
  await expect(role(page, "study-notice")).toContainText("Pridané do kázne „Aktívna“");
  expect((await sermonOf(id)).items).toHaveLength(2);

  await role(page, "study-notice-undo").tap();
  await expect(role(page, "study-notice")).toContainText("Odstránené z kázne");
  expect((await sermonOf(id)).items).toHaveLength(1);
  await finish(opened);
});

test("a search hit adds to the active sermon, words from the editor lead there", async ({ browser }) => {
  const id = await newSermon("Hľadanie");
  const opened = await openStudy(browser, baseURL, PHONE_390, `/ui/study?view=sermon&id=${id}`, null);
  const { page } = opened;
  await expect(role(page, "sermon-empty")).toBeVisible();

  await role(page, "sermon-add-field").fill("heb 11 2-3");
  await role(page, "sermon-add-field").press("Enter");
  await expect(role(page, "sermon-item")).toHaveAttribute("data-label", "Hebrejom 11:2-3 (SEB)");

  await role(page, "sermon-add-field").fill("viera zárukou");
  await role(page, "sermon-add-field").press("Enter");
  await expect(role(page, "search-results")).toHaveAttribute("data-state", "ready");
  await role(page, "search-hit-add").first().tap();
  await expect(role(page, "study-notice")).toContainText("Pridané do kázne „Hľadanie“");
  expect((await sermonOf(id)).items).toHaveLength(2);
  await expect(field(page)).toBeVisible();
  await finish(opened);
});

test("a tap on a save that fails stays on screen and the next try stores it", async ({ browser }) => {
  const id = await newSermon("Výpadok");
  await addVerse(id, 1, 1);
  const opened = await openStudy(browser, baseURL, PHONE_390, `/ui/study?view=sermon&id=${id}`, null);
  const { page } = opened;
  await expect(role(page, "sermon-item")).toBeVisible();

  const edits = /\/bible\/sermons\/.*\/edits$/;
  await page.route(edits, (route) => route.abort());
  await token(page, 1, 0).tap();
  await expect(role(page, "sermon-save")).toHaveAttribute("data-save-state", "error");
  await expect(token(page, 1, 0)).toHaveAttribute("data-state", "struck");
  expect(Object.keys((await sermonOf(id)).items[0].edits)).toEqual([]);

  await page.unroute(edits);
  await role(page, "sermon-save-retry").tap();
  await saved(page);
  expect((await sermonOf(id)).items[0].edits["1"].deleted).toEqual([0]);
  opened.errors.length = 0; // the aborted request's own console line
  await finish(opened);
});

test("an edit survives another device's write; a deleted item comes back with Späť", async ({ browser }) => {
  const id = await newSermon("Dve zariadenia");
  await addVerse(id, 1, 1);
  await addVerse(id, 2, 2);
  const opened = await openStudy(browser, baseURL, PHONE_390, `/ui/study?view=sermon&id=${id}`, null);
  const { page } = opened;
  await expect(role(page, "sermon-item")).toHaveCount(2);

  await addVerse(id, 3, 3); // another device
  await token(page, 1, 0).tap();
  await saved(page);
  await expect(role(page, "study-notice")).toContainText("tvoja úprava je uložená");
  await expect(role(page, "sermon-item")).toHaveCount(3);
  expect((await sermonOf(id)).items[0].edits["1"].deleted).toEqual([0]);

  const first = role(page, "sermon-item").first();
  await first.locator('[data-role="sermon-item-menu"]').tap();
  await first.locator('[data-role="sermon-item-delete"]').tap();
  await expect(role(page, "sermon-item")).toHaveCount(2);
  await role(page, "study-notice-undo").tap();
  await expect(role(page, "sermon-item")).toHaveCount(3);
  const back = await sermonOf(id);
  expect(back.items[0].edits["1"].deleted, "same place, same edits").toEqual([0]);
  // The deliberate conflict's own console line.
  opened.errors.splice(0, opened.errors.length, ...opened.errors.filter((line) => !line.includes("409")));
  await finish(opened);
});

test("a new sermon gets its date; a deleted one leaves the list and the device", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390, "/ui/study?view=sermons", null);
  const { page } = opened;
  await expect(role(page, "sermon-list")).toHaveAttribute("data-state", "ready");
  await role(page, "sermon-new-open").tap();
  await role(page, "sermon-new-date").fill("2031-03-02");
  await role(page, "sermon-new-title").fill("Veľká noc");
  await role(page, "sermon-new-create").tap();
  await expect(role(page, "sermon-title")).toHaveText("Veľká noc");
  await expect(role(page, "sermon-date")).toHaveText("Kázeň · ne 2. 3.");

  await role(page, "study-sermons").tap();
  const row = page.locator('[data-role="sermon-row"][data-date="2031-03-02"]');
  await expect(row).toHaveAttribute("data-active", "true");
  page.once("dialog", (dialog) => dialog.accept());
  await row.locator('[data-role="sermon-delete"]').tap();
  await expect(row).toHaveCount(0);
  const active = await page.evaluate(() => localStorage.getItem("presenter:study.activeSermon"));
  expect(active).toBeNull();
  await finish(opened);
});
