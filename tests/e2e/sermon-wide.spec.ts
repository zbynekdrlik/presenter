/**
 * #851 E3b — on a wide screen the preacher reads on the left and builds on the
 * right; a narrow one shows the sermon alone, and a rotation loses neither.
 * Nothing reaches a wall (T11).
 */
import { test, expect, type Page } from "@playwright/test";
import {
  assertNoHorizontalOverflow,
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";
import { expectChapter, field, finish, openStudy, verseNumber } from "./study-support";

test.describe.configure({ timeout: 180_000 });

const WIDE = { viewport: { width: 1280, height: 800 } };

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

async function newSermon(title: string): Promise<string> {
  const response = await fetch(`${baseURL}/bible/sermons`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ title, serviceDate: "2030-06-02", defaultTranslation: "slk-seb" }),
  });
  expect(response.status).toBe(200);
  return ((await response.json()) as { id: string }).id;
}

async function addVerse(id: string, verse: number) {
  const { revision } = (await (await fetch(`${baseURL}/bible/sermons/${id}`)).json()) as { revision: number };
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

async function openBeside(page: Page, id: string) {
  await page.goto(`/ui/study?view=sermon&id=${id}`);
  await expect(role(page, "study-sermon-pane").locator('[data-role="sermon-editor"]')).toHaveAttribute(
    "data-state",
    "ready",
  );
  await expectChapter(page, "slk-seb:HEB.11");
}

test("the sermon sits beside the last place, and a verse goes across", async ({ browser }) => {
  const id = await newSermon("Vedľa čítačky");
  const opened = await openStudy(browser, baseURL, WIDE);
  const { page } = opened;
  await openBeside(page, id);
  expect(new URL(page.url()).searchParams.get("pane")).toBe(id);
  await assertNoHorizontalOverflow(page);

  const reader = await page.locator('[data-role="study-chapter"]').boundingBox();
  const pane = await role(page, "study-sermon-pane").boundingBox();
  expect(reader && pane && reader.x + reader.width <= pane.x).toBeTruthy();

  await verseNumber(page, 1).click();
  await role(page, "verse-add-to-sermon").click();
  await expect(role(page, "study-notice")).toContainText("Pridané do kázne");
  const item = role(page, "study-sermon-pane").locator('[data-role="sermon-item"]');
  await expect(item).toHaveAttribute("data-label", "Hebrejom 11:1 (SEB)");
  await expect(item).toBeVisible();

  await verseNumber(page, 2).click();
  await expect(role(page, "study-sermon-pane")).toBeVisible();
  const sheet = await role(page, "verse-actions").boundingBox();
  const side = await role(page, "study-sermon-pane").boundingBox();
  expect(sheet && side && sheet.x + sheet.width <= side.x + 1).toBeTruthy();
  await finish(opened);
});

test("a move in the text keeps the sermon, Back restores the place and a reload the pane", async ({ browser }) => {
  const id = await newSermon("Pri listovaní");
  const opened = await openStudy(browser, baseURL, WIDE);
  const { page } = opened;
  await openBeside(page, id);
  const title = role(page, "sermon-title");
  await expect(title).toHaveText("Pri listovaní");

  await field(page).fill("heb 12");
  await field(page).press("Enter");
  await expectChapter(page, "slk-seb:HEB.12");
  await expect(title).toHaveText("Pri listovaní");
  expect(new URL(page.url()).searchParams.get("pane")).toBe(id);

  await page.goBack();
  await expectChapter(page, "slk-seb:HEB.11");
  await expect(title).toHaveText("Pri listovaní");

  await page.reload();
  await expectChapter(page, "slk-seb:HEB.11");
  await expect(title).toHaveText("Pri listovaní");
  await finish(opened);
});

test("a narrow screen shows the sermon alone and turning back restores both", async ({ browser }) => {
  const id = await newSermon("Otočenie");
  const opened = await openStudy(browser, baseURL, WIDE);
  const { page } = opened;
  await openBeside(page, id);

  await page.setViewportSize({ width: 800, height: 1100 });
  await expect(role(page, "study-sermon-pane")).toHaveCount(0);
  await expect(role(page, "sermon-editor")).toHaveAttribute("data-state", "ready");
  await expect(page.locator('[data-role="study-chapter"]')).toHaveCount(0);
  expect(new URL(page.url()).searchParams.get("view")).toBe("sermon");

  await page.setViewportSize({ width: 1280, height: 800 });
  await expect(role(page, "study-sermon-pane")).toBeVisible();
  await expectChapter(page, "slk-seb:HEB.11");
  await finish(opened);
});

test("HOTOVO leaves the pane for the list", async ({ browser }) => {
  const id = await newSermon("Dokončenie");
  const opened = await openStudy(browser, baseURL, WIDE);
  const { page } = opened;
  await openBeside(page, id);
  await role(page, "sermon-done").click();
  await expect(role(page, "sermon-list")).toHaveAttribute("data-state", "ready");
  await expect(role(page, "study-sermon-pane")).toHaveCount(0);
  await finish(opened);
});

test("a refused order survives a rotation until HOTOVO has named it", async ({ browser }) => {
  const id = await newSermon("Odmietnuté");
  for (const verse of [1, 2]) await addVerse(id, verse);
  const opened = await openStudy(browser, baseURL, WIDE);
  const { page } = opened;
  await openBeside(page, id);
  await role(page, "sermon-reorder-open").click();
  await role(page, "sermon-order-row").nth(1).locator('[data-role="sermon-order-up"]').click();
  await page.route(/\/items\/reorder$/, async (route) => {
    await addVerse(id, 3);
    await route.continue();
  });
  await role(page, "sermon-reorder").locator('[data-role="study-sheet-close"]').click();
  await expect(role(page, "study-notice")).toContainText("Poradie sa medzitým zmenilo");

  await page.setViewportSize({ width: 800, height: 1100 });
  await expect(role(page, "study-sermon-pane")).toHaveCount(0);
  await expect(role(page, "sermon-item")).toHaveCount(3);
  await role(page, "sermon-done").click();
  await expect(role(page, "study-notice")).toContainText("Poradie sa medzitým zmenilo");
  await expect(role(page, "sermon-list")).toHaveCount(0);
  opened.errors.splice(0, opened.errors.length, ...opened.errors.filter((line) => !line.includes("409")));
  await finish(opened);
});

test("a sermon deleted elsewhere leaves the pane, and the reader stays", async ({ browser }) => {
  const id = await newSermon("Zmazaná");
  const opened = await openStudy(browser, baseURL, WIDE);
  const { page } = opened;
  await openBeside(page, id);
  await expect(role(page, "sermon-editor")).toHaveAttribute("data-live", "true");
  expect((await fetch(`${baseURL}/bible/sermons/${id}`, { method: "DELETE" })).status).toBe(204);
  await expect(role(page, "study-sermon-pane")).toHaveCount(0);
  expect(new URL(page.url()).searchParams.get("pane")).toBeNull();
  await expectChapter(page, "slk-seb:HEB.11");
  await expect(role(page, "study-notice")).toContainText("Kázeň už neexistuje");
  opened.errors.splice(0, opened.errors.length, ...opened.errors.filter((line) => !line.includes("404")));
  await finish(opened);
});
