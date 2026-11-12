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
/** By id: the list runs by service date, so older tests' sermons come first. */
const openRow = (page: Page, id: string) =>
  page.locator(`[data-role="sermon-row"][data-sermon-id="${id}"] [data-role="sermon-open"]`).tap();
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

/** #936: the add sheet's row of the sermon `id`. */
const addRow = (page: Page, id: string) =>
  page.locator(`[data-role="add-to-sermon-row"][data-sermon-id="${id}"]`);

test("a selected range goes into a new sermon as one item, and the next add asks again (#936)", async ({ browser }) => {
  const older = await newSermon("Stará kázeň", "2026-01-04");
  await addVerse(older, 3, 3);
  const opened = await openStudy(browser, baseURL, PHONE_390, undefined, undefined, async (page) => {
    // The prod case: a remembered old sermon must not take the passage.
    await page.addInitScript((sermon) => {
      localStorage.setItem("presenter:study.activeSermon", JSON.stringify(sermon));
    }, older);
  });
  const { page } = opened;
  await verseNumber(page, 1).tap();
  const range = role(page, "verse-range");
  await expect(range).toBeVisible();
  await expect(role(page, "verse-range-count")).toHaveText("1 verš");
  await expect(role(page, "verse-range-shrink")).toBeDisabled();
  for (let step = 0; step < 10; step += 1) {
    await role(page, "verse-range-grow").tap();
  }
  await expect(role(page, "verse-range-count")).toHaveText("11 veršov");
  await expect(page).toHaveURL(/v=1-11(&|$)/);
  await expect(role(page, "verse-actions")).toContainText("Hebrejom 11:1-11 (SEB)");
  await expect(role(page, "verse-add-to-sermon")).toHaveText("Pridať do kázne (11 veršov)");
  await role(page, "verse-range-shrink").tap();
  await expect(role(page, "verse-range-count")).toHaveText("10 veršov");
  await role(page, "verse-range-grow").tap();

  await role(page, "verse-add-to-sermon").tap();
  const sheet = role(page, "add-to-sermon");
  await expect(sheet).toBeVisible();
  await expect(role(page, "add-to-sermon-passage")).toHaveText("Hebrejom 11:1-11 (SEB)");
  // "Nová kázeň" comes first, then the newest service date.
  await expect(sheet.locator(".study-list > *").first()).toHaveAttribute("data-role", "add-to-new-sermon");
  await expect(addRow(page, older)).toContainText("naposledy");
  expect((await sermonOf(older)).items).toHaveLength(1);
  await assertNoHorizontalOverflow(page);
  const created = page.waitForResponse(
    (response) => response.request().method() === "POST" && new URL(response.url()).pathname === "/bible/sermons",
  );
  await role(page, "add-to-new-sermon").tap();
  const id = ((await (await created).json()) as { id: string }).id;
  await expect(role(page, "study-notice")).toContainText("Pridané do kázne");
  await expect(sheet).toHaveCount(0);
  const fresh = (await (await fetch(`${baseURL}/bible/sermons/${id}`)).json()) as {
    secondaryTranslation: string | null;
    items: { label: string }[];
    slides: { secondaryText: string }[];
  };
  expect(fresh.items.map((item) => item.label)).toEqual(["Hebrejom 11:1-11 (SEB)"]);
  // #937: nobody picked a second language, so no slide carries one.
  expect(fresh.secondaryTranslation).toBeNull();
  expect(fresh.slides.length).toBeGreaterThan(0);
  expect(fresh.slides.every((slide) => slide.secondaryText === "")).toBe(true);
  expect((await sermonOf(older)).items).toHaveLength(1);

  // The next add asks again; the new sermon is now the one marked.
  await verseNumber(page, 12).tap();
  await role(page, "verse-add-to-sermon").tap();
  await expect(sheet).toBeVisible();
  await expect(addRow(page, id)).toContainText("naposledy");
  await addRow(page, id).tap();
  await expect(role(page, "study-notice")).toContainText("Pridané do kázne");
  expect((await sermonOf(id)).items).toHaveLength(2);
  expect((await sermonOf(older)).items).toHaveLength(1);
  await finish(opened);
});

test("a sermon deleted while the sheet is open: the tap says so and its row goes (#936)", async ({ browser }) => {
  const gone = await newSermon("Zmazaná medzitým");
  const opened = await openStudy(browser, baseURL, PHONE_390);
  const { page } = opened;
  await verseNumber(page, 2).tap();
  await role(page, "verse-add-to-sermon").tap();
  await expect(addRow(page, gone)).toBeVisible();
  const deleted = await fetch(`${baseURL}/bible/sermons/${gone}`, { method: "DELETE" });
  expect(deleted.status).toBe(204);

  await addRow(page, gone).tap();
  await expect(role(page, "study-notice")).toContainText("Kázeň už neexistuje");
  await expect(addRow(page, gone)).toHaveCount(0);
  await expect(role(page, "add-to-sermon")).toBeVisible();
  // The deliberate 404 of the deleted sermon's own read.
  opened.errors.splice(0, opened.errors.length, ...opened.errors.filter((line) => !line.includes("404")));
  await finish(opened);
});

test("the second language is the preacher's visible choice, none by default (#937)", async ({ browser }) => {
  const id = await newSermon("Druhý preklad");
  await addVerse(id, 1, 1);
  const opened = await openStudy(browser, baseURL, PHONE_390, `/ui/study?view=sermon&id=${id}`, null);
  const { page } = opened;
  await expect(role(page, "sermon-editor")).toHaveAttribute("data-state", "ready");
  const choice = role(page, "sermon-secondary");
  await expect(choice).toHaveValue("");
  await expect(choice.locator("option:checked")).toHaveText("žiadny");
  const detail = async () =>
    (await (await fetch(`${baseURL}/bible/sermons/${id}`)).json()) as {
      secondaryTranslation: string | null;
      slides: { secondaryText: string; secondaryReference: string }[];
    };
  expect((await detail()).slides[0].secondaryText).toBe("");
  const secondaryLine = page.locator('[data-role="sermon-slide"] .sermon-slide__text--secondary');

  // A write that fails (a malformed answer) puts the choice back.
  await page.route(`**/bible/sermons/${id}`, async (route) => {
    if (route.request().method() === "PATCH") {
      await route.fulfill({ status: 200, contentType: "application/json", body: "not-json" });
    } else {
      await route.fallback();
    }
  });
  await choice.selectOption("eng-kjv");
  await expect(choice).toHaveValue("");
  expect((await detail()).secondaryTranslation).toBeNull();
  await page.unroute(`**/bible/sermons/${id}`);

  await choice.selectOption("eng-kjv");
  await saved(page);
  await expect.poll(async () => (await detail()).secondaryTranslation).toBe("eng-kjv");
  const bilingual = await detail();
  expect(bilingual.slides[0].secondaryText).toContain("faith");
  expect(bilingual.slides[0].secondaryReference).toContain("(KJV)");
  // What the preacher sees in Náhľad: the second line under the verse.
  await role(page, "sermon-preview-open").tap();
  await expect(secondaryLine.first()).toContainText("faith");
  await page.goBack();
  await expect(role(page, "sermon-preview-open")).toBeVisible();

  await page.reload();
  await expect(role(page, "sermon-secondary")).toHaveValue("eng-kjv");
  await role(page, "sermon-secondary").selectOption("");
  await saved(page);
  await expect.poll(async () => (await detail()).secondaryTranslation).toBeNull();
  expect((await detail()).slides[0].secondaryText).toBe("");
  await role(page, "sermon-preview-open").tap();
  await expect(page.locator('[data-role="sermon-slide"]').first()).toBeVisible();
  await expect(secondaryLine).toHaveCount(0);
  await finish(opened);
});

test("the last sermon is only marked in the sheet, never chosen, and Späť takes the verse back", async ({ browser }) => {
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
  // #936 (owner 2026-11-12): "Musí mi vždy dať na výber" — nothing is added
  // before the preacher picks.
  await expect(role(page, "add-to-sermon")).toBeVisible();
  await expect(addRow(page, id)).toContainText("naposledy");
  expect((await sermonOf(id)).items).toHaveLength(1);
  await addRow(page, id).tap();
  await expect(role(page, "study-notice")).toContainText("Pridané do kázne „Aktívna“");
  expect((await sermonOf(id)).items).toHaveLength(2);

  await role(page, "study-notice-undo").tap();
  await expect(role(page, "study-notice")).toContainText("Odstránené z kázne");
  expect((await sermonOf(id)).items).toHaveLength(1);
  await finish(opened);
});

test("a search hit adds to the sermon picked in the sheet, words from the editor lead there", async ({ browser }) => {
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
  await expect(addRow(page, id)).toContainText("naposledy");
  await addRow(page, id).tap();
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
  // Only the aborted request's own console line: a panic here must still fail.
  opened.errors.splice(0, opened.errors.length, ...opened.errors.filter((line) => !line.includes("Failed to fetch") && !line.includes("ERR_FAILED")));
  await finish(opened);
});

test("a tap is stored even when the preacher leaves the sermon at once", async ({ browser }) => {
  const id = await newSermon("Odchod");
  await addVerse(id, 1, 1);
  const opened = await openStudy(browser, baseURL, PHONE_390, `/ui/study?view=sermon&id=${id}`, null);
  const { page } = opened;
  await expect(role(page, "sermon-item")).toBeVisible();

  // Inside the 300 ms the save waits for; the answer lands on a closed editor.
  await page.evaluate(() => {
    const word = document.querySelector('[data-role="sermon-token"]') as HTMLElement;
    word.click();
    (document.querySelector('[data-role="study-sermons"]') as HTMLElement).click();
  });
  await expect(role(page, "sermon-list")).toHaveAttribute("data-state", "ready");
  await expect.poll(async () => (await sermonOf(id)).items[0].edits["1"]?.deleted).toEqual([0]);
  await finish(opened);
});

test("a save that failed waits for the preacher's return, on screen", async ({ browser }) => {
  const id = await newSermon("Návrat");
  await addVerse(id, 1, 1);
  const opened = await openStudy(browser, baseURL, PHONE_390, `/ui/study?view=sermon&id=${id}`, null);
  const { page } = opened;
  await expect(role(page, "sermon-item")).toBeVisible();

  const edits = /\/bible\/sermons\/.*\/edits$/;
  await page.route(edits, (route) => route.abort());
  await token(page, 1, 0).tap();
  await expect(role(page, "sermon-save")).toHaveAttribute("data-save-state", "error");
  await role(page, "study-sermons").tap();
  await expect(role(page, "sermon-list")).toHaveAttribute("data-state", "ready");
  await page.unroute(edits);

  await openRow(page, id);
  await expect(token(page, 1, 0)).toHaveAttribute("data-state", "struck");
  await role(page, "sermon-save-retry").tap();
  await saved(page);
  expect((await sermonOf(id)).items[0].edits["1"].deleted).toEqual([0]);
  opened.errors.splice(0, opened.errors.length, ...opened.errors.filter((line) => !line.includes("Failed to fetch") && !line.includes("ERR_FAILED")));
  await finish(opened);
});

test("a tap during a slow structural write is saved after it", async ({ browser }) => {
  const id = await newSermon("Pomaly");
  await addVerse(id, 1, 1);
  const opened = await openStudy(browser, baseURL, PHONE_390, `/ui/study?view=sermon&id=${id}`, null);
  const { page } = opened;
  await expect(role(page, "sermon-item")).toHaveCount(1);

  let release = () => {};
  const held = new Promise<void>((resolve) => (release = resolve));
  await page.route(/\/bible\/sermons\/[^/]+\/items$/, async (route) => {
    await held;
    await route.continue();
  });
  await role(page, "sermon-add-field").fill("heb 11 2");
  await role(page, "sermon-add-field").press("Enter");
  await token(page, 1, 0).tap();
  // Past the 300 ms, so the save's timer finds the add holding the queue.
  await page.waitForTimeout(500);
  await expect(role(page, "sermon-save")).toHaveAttribute("data-save-state", "saving");
  release();
  await expect(role(page, "sermon-item")).toHaveCount(2);
  await saved(page);
  expect((await sermonOf(id)).items[0].edits["1"].deleted).toEqual([0]);
  await finish(opened);
});

test("an old answer never takes back a saved tap", async ({ browser }) => {
  const id = await newSermon("Stará odpoveď");
  await addVerse(id, 1, 1);
  const opened = await openStudy(browser, baseURL, PHONE_390, `/ui/study?view=sermon&id=${id}`, null);
  const { page } = opened;
  await expect(role(page, "sermon-item")).toBeVisible();

  // The reload of the return trip reads before the tap and answers after it.
  let release = () => {};
  const held = new Promise<void>((resolve) => (release = resolve));
  let stalled = 0;
  let landed = false;
  await page.route(new RegExp(`/bible/sermons/${id}$`), async (route) => {
    const answer = await route.fetch();
    stalled += 1;
    await held;
    await route.fulfill({ response: answer });
    landed = true;
  });
  await role(page, "study-sermons").tap();
  await openRow(page, id);
  await expect.poll(() => stalled).toBeGreaterThanOrEqual(1);
  await token(page, 1, 0).tap();
  await saved(page);
  release();
  await expect.poll(() => landed).toBe(true);
  await token(page, 1, 1).tap();
  await saved(page);
  await expect(token(page, 1, 0)).toHaveAttribute("data-state", "struck");
  expect((await sermonOf(id)).items[0].edits["1"].deleted).toEqual([0, 1]);
  await finish(opened);
});

test("Späť after a delete shares the queue with a tap on another card", async ({ browser }) => {
  const id = await newSermon("Späť a ťuknutie");
  await addVerse(id, 1, 1);
  await addVerse(id, 2, 2);
  const opened = await openStudy(browser, baseURL, PHONE_390, `/ui/study?view=sermon&id=${id}`, null);
  const { page } = opened;
  await expect(role(page, "sermon-item")).toHaveCount(2);

  await token(page, 1, 0).tap();
  const first = role(page, "sermon-item").first();
  await first.locator('[data-role="sermon-item-menu"]').tap();
  await first.locator('[data-role="sermon-item-delete"]').tap();
  await expect(role(page, "sermon-item")).toHaveCount(1);
  await token(page, 2, 0).tap();
  await role(page, "study-notice-undo").tap();
  await expect(role(page, "sermon-item")).toHaveCount(2);
  await saved(page);
  const back = await sermonOf(id);
  expect(back.items[0].edits["1"].deleted, "the first card, its tap restored").toEqual([0]);
  expect(back.items[1].edits["2"].deleted, "the tap in between").toEqual([0]);
  await finish(opened);
});

test("a double tap creates one sermon and adds one verse", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390, "/ui/study?view=sermons", null);
  const { page } = opened;
  const writes: string[] = [];
  page.on("request", (request) => {
    if (request.method() === "POST") writes.push(new URL(request.url()).pathname);
  });
  // A second write would land after the first: only the wait can show it.
  const written = async (ending: RegExp, count: number) => {
    await page.waitForTimeout(1_000);
    expect(writes.filter((path) => ending.test(path))).toHaveLength(count);
  };
  await expect(role(page, "sermon-list")).toHaveAttribute("data-state", "ready");
  await role(page, "sermon-new-open").tap();
  await role(page, "sermon-new-date").fill("2032-05-02");
  await role(page, "sermon-new-title").fill("Dvojité");
  await role(page, "sermon-new-create").dblclick();
  await expect(role(page, "sermon-title")).toHaveText("Dvojité");
  const id = new URL(page.url()).searchParams.get("id") as string;
  await written(/\/bible\/sermons$/, 1);

  // Enter twice in the field: it keeps its text until the verse is in.
  await role(page, "sermon-add-field").fill("heb 11 2");
  await page.evaluate(() => {
    const form = document.querySelector(".sermon-add") as HTMLFormElement;
    form.requestSubmit();
    form.requestSubmit();
  });
  await expect(role(page, "sermon-item")).toHaveCount(1);
  await written(/\/items$/, 1);
  await expect(role(page, "sermon-add-field")).toHaveValue("");

  await page.goto(`${baseURL}/ui/study?view=search&q=viera&t=slk-seb`);
  await expect(role(page, "search-results")).toHaveAttribute("data-state", "ready");
  await role(page, "search-hit-add").first().tap();
  await addRow(page, id).dblclick();
  await expect(role(page, "study-notice")).toContainText("Pridané do kázne „Dvojité“");
  await written(/\/items$/, 2);
  expect((await sermonOf(id)).items).toHaveLength(2);
  await finish(opened);
});

test("an edit survives another device's write; a deleted item comes back with Späť", async ({ browser }) => {
  const id = await newSermon("Dve zariadenia");
  await addVerse(id, 1, 1);
  await addVerse(id, 2, 2);
  const opened = await openStudy(browser, baseURL, PHONE_390, `/ui/study?view=sermon&id=${id}`, null);
  const { page } = opened;
  await expect(role(page, "sermon-item")).toHaveCount(2);

  // Another device, in the moment before this save: an earlier write would
  // reach the open editor through the socket and no conflict would happen.
  let raced = false;
  await page.route(/\/items\/[^/]+\/edits$/, async (route) => {
    if (!raced) {
      raced = true;
      await addVerse(id, 3, 3);
    }
    await route.continue();
  });
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

test("Späť on an added verse reaches the open editor", async ({ browser }) => {
  const id = await newSermon("Späť v editore");
  const opened = await openStudy(browser, baseURL, PHONE_390, undefined, undefined, async (page) => {
    await page.addInitScript((sermon) => {
      localStorage.setItem("presenter:study.activeSermon", JSON.stringify(sermon));
    }, id);
  });
  const { page } = opened;
  await verseNumber(page, 2).tap();
  await role(page, "verse-add-to-sermon").tap();
  await addRow(page, id).tap();
  await expect(role(page, "study-notice-undo")).toBeVisible();
  await role(page, "study-sermons").tap();
  await openRow(page, id);
  await expect(role(page, "sermon-item")).toHaveCount(1);

  await role(page, "study-notice-undo").tap();
  await expect(role(page, "sermon-item")).toHaveCount(0);
  await expect(role(page, "sermon-empty")).toBeVisible();
  expect((await sermonOf(id)).items).toHaveLength(0);
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

test("a sermon opened on its own leads back to the verses (owner, 2026-11-06)", async ({ browser }) => {
  // The reader was here before: Biblia must land on that place, not a default.
  const id = await newSermon("Späť k veršom");
  const opened = await openStudy(browser, baseURL, PHONE_390);
  const { page } = opened;
  // A reopened tab: the history starts on the sermon, so ◀ has nowhere to go.
  await page.goto(`${baseURL}/ui/study?view=sermon&id=${id}`);
  await expect(role(page, "sermon-title")).toHaveText("Späť k veršom");
  await expect(role(page, "history-back")).toHaveAttribute("aria-disabled", "true");
  await role(page, "study-bible").tap();
  await expect(page.locator('[data-role="study-chapter"]')).toHaveAttribute("data-loaded-for", "slk-seb:HEB.11");
  await expect(field(page)).toBeVisible();
  // Biblia belongs to the sermon views only; the list offers it too.
  await expect(role(page, "study-bible")).toHaveCount(0);
  await role(page, "study-sermons").tap();
  await expect(role(page, "sermon-list")).toHaveAttribute("data-state", "ready");
  await role(page, "study-bible").tap();
  await expect(page.locator('[data-role="study-chapter"]')).toHaveAttribute("data-loaded-for", "slk-seb:HEB.11");
  await assertNoHorizontalOverflow(page);
  await finish(opened);
});

test("Biblia opens a chapter in a browser that never read here", async ({ browser }) => {
  // No last place stored: the start route names no translation of its own.
  const opened = await openStudy(browser, baseURL, PHONE_390, "/ui/study?view=sermons", null);
  const { page } = opened;
  await expect(role(page, "sermon-list")).toHaveAttribute("data-state", "ready");
  await role(page, "study-bible").tap();
  const shown = page.locator('[data-role="study-chapter"]');
  await expect(shown).toHaveAttribute("data-state", "ready");
  await expect(shown).toHaveAttribute("data-loaded-for", /^[a-z]+-[a-z]+:[0-9A-Z]+\.\d+$/);
  await finish(opened);
});
