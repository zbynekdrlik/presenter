/**
 * #966 — owner 2026-11-16: "je stale vela miest z ktorych sa neviem vratit do
 * biblie a musim ist najprv do kazne". From every study view that is not the
 * reader — opened cold, from a pasted link, so no history helps — the header's
 * Biblia leads back to the last reading place in ONE tap, also with a sheet
 * open over the view, and the 360 px header still fits.
 */
import { test, expect, type Locator, type Page } from "@playwright/test";
import {
  deriveTestConfig,
  PHONE_360,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";
import { expectChapter, finish, openStudy } from "./study-support";

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

const role = (page: Page, name: string): Locator => page.locator(`[data-role="${name}"]`);

/**
 * The 360 px header holds Biblia and Kázne inside the screen. The whole page is
 * not checked: Strong's book row scrolls sideways on purpose (36 books).
 */
async function assertHeaderFits(page: Page): Promise<void> {
  const overflow = await page.evaluate(() => {
    const width = window.innerWidth;
    return Array.from(document.querySelectorAll<HTMLElement>(".study-head *"))
      .filter((el) => el.getBoundingClientRect().right > width + 1)
      .map((el) => `${el.dataset.role ?? el.className} right=${Math.round(el.getBoundingClientRect().right)}`);
  });
  expect(overflow, "header elements past the right edge").toEqual([]);
  const pageOverflow = await page.evaluate(
    () => document.documentElement.scrollWidth - window.innerWidth,
  );
  expect(pageOverflow, "page wider than the viewport").toBeLessThanOrEqual(0);
}

/** Every study view that is not the reader, as a pasted link opens it. */
const AWAY_FROM_READING: Array<[string, string]> = [
  ["search", "/ui/study?view=search&t=slk-seb&q=viera"],
  ["strongs", "/ui/study?view=strongs&t=eng-kjv&n=G4102&scope=all"],
  ["compare", "/ui/study?view=compare&t=slk-seb&p=HEB.11.1&tt=slk-seb,eng-kjv"],
  ["sermons", "/ui/study?view=sermons"],
  ["search + text-size sheet", "/ui/study?view=search&t=slk-seb&q=viera&s=pismo"],
  ["compare + history sheet", "/ui/study?view=compare&t=slk-seb&p=HEB.11.1&tt=slk-seb,eng-kjv&s=historia"],
];

for (const [name, path] of AWAY_FROM_READING) {
  test(`Biblia leads back to reading in one tap from ${name}`, async ({ browser }) => {
    // Reading Hebrews 11 is the last place this browser read.
    const opened = await openStudy(browser, baseURL, PHONE_360);
    const { page } = opened;
    await page.goto(path);
    await page.waitForSelector('body[data-wasm-ready="true"]', { state: "attached", timeout: 30_000 });
    await expect(role(page, "study-bible")).toBeVisible();
    await expect(role(page, "study-sermons")).toBeVisible();
    await assertHeaderFits(page);
    await role(page, "study-bible").tap({ timeout: 10_000 });
    await expectChapter(page, "slk-seb:HEB.11");
    await expect(page).not.toHaveURL(/[?&]s=/);
    await expect(page).not.toHaveURL(/view=/);
    await finish(opened);
  });
}

test("the reader itself needs no Biblia and keeps its place title", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, PHONE_360);
  const { page } = opened;
  await expect(role(page, "study-bible")).toHaveCount(0);
  await expect(role(page, "study-place")).toContainText("11");
  await assertHeaderFits(page);
  await finish(opened);
});
