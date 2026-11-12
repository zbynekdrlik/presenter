/**
 * #851 S4 — cross-references on `/ui/study`: the most voted first, a tap opens
 * the place and Back returns, the rest behind "Ďalšie". Nothing here reaches a
 * wall (T11).
 */
import { test, expect, type Locator, type Page } from "@playwright/test";
import {
  assertNoHorizontalOverflow,
  deriveTestConfig,
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

const JOHN_3 = "/ui/study?t=slk-seb&p=JHN.3";
const items = (page: Page): Locator => page.locator('[data-role="crossref-item"]');

async function openReferences(page: Page, verse: number): Promise<void> {
  await verseNumber(page, verse).tap();
  await expect(page.locator('[data-role="verse-crossrefs"]')).toHaveText(/Odkazy \(\d+\)/);
  await page.locator('[data-role="verse-crossrefs"]').tap();
  await expect(items(page).first()).toBeVisible();
}

test("Odkazy lists the ten most voted, Ďalšie the rest, and names its source", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390, JOHN_3, "slk-seb:JHN.3");
  const { page } = opened;
  await openReferences(page, 16);
  await expect(page).toHaveURL(/v=16&s=odkazy/);

  await expect(items(page)).toHaveCount(10);
  const votes = await items(page).evaluateAll((rows) =>
    rows.map((row) => Number(row.getAttribute("data-votes"))),
  );
  expect(votes).toEqual([...votes].sort((a, b) => b - a));
  await expect(page.locator('[data-role="crossref-source"]')).toHaveText(
    "Krížové odkazy: OpenBible.info (CC BY)",
  );
  await assertNoHorizontalOverflow(page);

  const more = page.locator('[data-role="crossref-more"]');
  const hidden = Number(/\((\d+)\)/.exec((await more.textContent()) ?? "")?.[1]);
  await more.tap();
  await expect(items(page)).toHaveCount(10 + hidden);
  await expect(more).toHaveCount(0);
  await finish(opened);
});

test("a reference opens its place, and Back returns to the verse", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390, JOHN_3, "slk-seb:JHN.3");
  const { page } = opened;
  await openReferences(page, 16);
  const first = items(page).first();
  await expect(first.locator('[data-role="crossref-open"]')).toHaveText("Rimanom 5:8");
  await first.locator('[data-role="crossref-open"]').tap();

  await expectChapter(page, "slk-seb:ROM.5");
  await expect(page.locator('[data-role="study-verse"][data-selected="true"]')).toHaveAttribute(
    "data-verse",
    "8",
  );
  await page.goBack();
  await expectChapter(page, "slk-seb:JHN.3");
  await expect(page).toHaveURL(/p=JHN\.3/);
  await expect(page.locator('[data-role="study-sheet-close"]')).toHaveCount(0);
  // Nothing behind it is a sheet either.
  await page.goBack();
  await expect(page).not.toHaveURL(/s=/);
  await expect(page.locator('[data-role="study-sheet-close"]')).toHaveCount(0);
  await finish(opened);
});

test("a verse this translation lacks shows the KJV and says whose text it is", async ({ browser }) => {
  // Acts 2:17 points at Joel 2:28, which is 3:1 in the SEB.
  const opened = await openStudy(
    browser,
    baseURL,
    PHONE_390,
    "/ui/study?t=slk-seb&p=ACT.2&v=17&s=odkazy",
    "slk-seb:ACT.2",
  );
  const { page } = opened;
  await page.locator('[data-role="crossref-more"]').tap();
  const joel = items(page).filter({ hasText: "Joel 2:28" });
  await expect(joel).toContainText("(KJV)");
  await expect(joel).toContainText("pour out my spirit");
  await joel.locator('[data-role="crossref-open"]').tap();
  await expectChapter(page, "eng-kjv:JOL.2");
  await finish(opened);
});

test("a failed list says why and retries inline", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390, JOHN_3, "slk-seb:JHN.3");
  const { page } = opened;
  let failed = false;
  await page.route(/\/bible\/study\/crossrefs\?/, async (route) => {
    if (failed) return route.fallback();
    failed = true;
    await route.fulfill({ status: 200, contentType: "application/json", body: "not-json" });
  });
  await verseNumber(page, 16).tap();
  await page.locator('[data-role="verse-crossrefs"]').tap();
  const card = page.locator('[data-role="crossref-error"]');
  await expect(card).toContainText("Nepodarilo sa načítať");
  await card.locator('[data-role="study-retry"]').tap();
  await expect(items(page)).toHaveCount(10);
  await finish(opened);
});

test("Back from a reference in the same chapter scrolls to the verse", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_390, JOHN_3, "slk-seb:JHN.3");
  const { page } = opened;
  await openReferences(page, 2);
  const more = page.locator('[data-role="crossref-more"]');
  if ((await more.count()) > 0) await more.tap();
  await items(page).filter({ hasText: "Ján 3:26" }).locator('[data-role="crossref-open"]').tap();
  await expect(page).toHaveURL(/v=26/);
  await expect(verse(page, 26)).toBeInViewport();
  await page.goBack();
  await expect(page).toHaveURL(/v=2(&|$)/);
  await expect(verse(page, 2)).toBeInViewport();
  await finish(opened);
});

test("adding a reference leaves the list open for the next one", async ({ browser }) => {
  const created = await fetch(`${baseURL}/bible/sermons`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ title: "Odkazy", serviceDate: "2030-06-02", defaultTranslation: "slk-seb" }),
  });
  expect(created.status).toBe(200);
  const id = ((await created.json()) as { id: string }).id;
  const opened = await openStudy(browser, baseURL, PHONE_390, JOHN_3, "slk-seb:JHN.3", async (page) => {
    await page.addInitScript((sermon) => {
      localStorage.setItem("presenter:study.activeSermon", JSON.stringify(sermon));
    }, id);
  });
  const { page } = opened;
  await openReferences(page, 16);
  await items(page).first().locator('[data-role="crossref-add"]').tap();
  // #936: the sheet asks which sermon, then the list comes back.
  await page.locator(`[data-role="add-to-sermon-row"][data-sermon-id="${id}"]`).tap();
  await expect(page.locator('[data-role="study-notice"]')).toContainText("Pridané do kázne „Odkazy“");
  await expect(page.locator('[data-role="add-to-sermon"]')).toHaveCount(0);
  await expect(items(page)).toHaveCount(10);
  await expect(page).toHaveURL(/s=odkazy/);
  const sermon = (await (await fetch(`${baseURL}/bible/sermons/${id}`)).json()) as { items: unknown[] };
  expect(sermon.items).toHaveLength(1);
  await finish(opened);
});
