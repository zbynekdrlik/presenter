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
  await expect(role(page, "study-sermon-pane").locator('[data-role="sermon-item"]')).toHaveAttribute(
    "data-label",
    "Hebrejom 11:1 (SEB)",
  );
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
