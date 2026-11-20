/**
 * #983 — Study search, owner 2026-11-20: "výsledky boli random miesta v
 * biblii vôbec nie v poradí … nevedel som prepínať v ktorom preklade chcem
 * hľadať … pri každom výsledku si ľahko prepnúť ako to je v inom preklade".
 * Every hit in the Bible's order with the total named, a translation row that
 * runs the search again, and per-hit translation chips that show the verse in
 * another translation inside the row and back. Phone (touch) and desktop
 * (mouse).
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

const DESKTOP: BrowserContextOptions = { viewport: { width: 1280, height: 900 } };

const results = (page: Page) => page.locator('[data-role="search-results"]');
const hits = (page: Page) => page.locator('[data-role="search-hit"]');
const translationChip = (page: Page, code: string) =>
  page.locator(`[data-role="search-translation"][data-translation="${code}"]`);

/** A phone taps, a desktop clicks. */
async function press(target: Locator, touch: boolean): Promise<void> {
  if (touch) await target.tap();
  else await target.click();
}

/** Every hit's (book number, chapter, verse), as the list shows them. */
async function places(page: Page): Promise<number[][]> {
  return hits(page).evaluateAll((rows) =>
    rows.map((row) => [
      Number(row.getAttribute("data-book-number")),
      Number(row.getAttribute("data-chapter")),
      Number(row.getAttribute("data-verse")),
    ]),
  );
}

function inBibleOrder(found: number[][]): boolean {
  return found.every((place, i) => {
    if (i === 0) return true;
    const before = found[i - 1];
    for (let k = 0; k < 3; k += 1) {
      if (place[k] !== before[k]) return place[k] > before[k];
    }
    return true;
  });
}

for (const [name, profile, touch] of [
  ["phone", PHONE_390, true],
  ["desktop", DESKTOP, false],
] as const) {
  test(`${name}: every hit of "moc" in the Bible's order, the count names them all`, async ({ browser }) => {
    const searches: URL[] = [];
    const opened = await openStudy(browser, baseURL, profile, "/ui/study?view=search&t=slk-seb&q=moc", null, async (page) => {
      page.on("request", (request) => {
        if (request.url().includes("/bible/search")) searches.push(new URL(request.url()));
      });
    });
    const { page } = opened;
    await expect(results(page)).toHaveAttribute("data-loaded-for", "slk-seb:moc");
    await expect(results(page)).toHaveAttribute("data-state", "ready");
    expect(searches).toHaveLength(1);
    expect(searches[0].searchParams.get("order")).toBe("bible");
    expect(searches[0].searchParams.get("limit")).toBe("1000");

    // SNV: 653 hits, relevance cut them at 100 (the owner's 19.11 search).
    await expect.poll(() => hits(page).count()).toBeGreaterThan(100);
    const found = await places(page);
    expect(found.length).toBeLessThan(1000);
    await expect(page.locator('[data-role="search-count"]')).toHaveText(`Výsledkov: ${found.length}`);
    expect(found[0][0], "the first hit is in the Pentateuch").toBeLessThanOrEqual(5);
    expect(inBibleOrder(found), `hits in Bible order: ${JSON.stringify(found.slice(0, 12))}`).toBe(true);
    expect(found.some((place) => place[0] >= 40), "the New Testament follows").toBe(true);
    await assertNoHorizontalOverflow(page);
    await finish(opened);
  });

  test(`${name}: the translation row runs the search again in that translation`, async ({ browser }) => {
    const opened = await openStudy(
      browser,
      baseURL,
      profile,
      "/ui/study?view=search&t=slk-seb&q=viera+z%C3%A1rukou",
      null,
    );
    const { page } = opened;
    await expect(results(page)).toHaveAttribute("data-loaded-for", "slk-seb:viera zárukou");
    const row = page.locator('[data-role="search-translations"]');
    await expect(translationChip(page, "slk-seb")).toHaveAttribute("aria-pressed", "true");
    // The NLT has no text on this box to search: it is no chip here.
    await expect(translationChip(page, "eng-nlt")).toHaveCount(0);
    // The header's chips are the reader's: one row of translations on screen.
    await expect(page.locator('[data-role="study-favs"]')).toHaveCount(0);
    if (touch) await assertMinTapTarget(row.locator("button"), 44);

    const asked = page.waitForRequest(/\/bible\/search\?.*translation=eng-kjv/);
    await press(translationChip(page, "eng-kjv"), touch);
    const request = new URL((await asked).url());
    expect(request.searchParams.get("query")).toBe("viera zárukou");
    expect(request.searchParams.get("order")).toBe("bible");
    await expect(results(page)).toHaveAttribute("data-loaded-for", "eng-kjv:viera zárukou");
    await expect(page).toHaveURL(/view=search&t=eng-kjv&q=viera/);
    await expect(translationChip(page, "eng-kjv")).toHaveAttribute("aria-pressed", "true");
    await expect(translationChip(page, "slk-seb")).toHaveAttribute("aria-pressed", "false");

    await press(translationChip(page, "slk-seb"), touch);
    await expect(results(page)).toHaveAttribute("data-loaded-for", "slk-seb:viera zárukou");
    await expect(hits(page).filter({ hasText: "Hebrejom 11:1" }).first()).toBeVisible();
    await finish(opened);
  });

  test(`${name}: a hit shows its verse in another translation and goes back`, async ({ browser }) => {
    const opened = await openStudy(
      browser,
      baseURL,
      profile,
      "/ui/study?view=search&t=slk-seb&q=viera+z%C3%A1rukou",
      null,
    );
    const { page } = opened;
    await expect(results(page)).toHaveAttribute("data-loaded-for", "slk-seb:viera zárukou");
    const row = page.locator('[data-role="search-hit-row"]').filter({ hasText: "Hebrejom 11:1" }).first();
    const hit = row.locator('[data-role="search-hit"]');
    const text = row.locator('[data-role="search-hit-text"]');
    const chip = (code: string) => row.locator(`[data-role="hit-translation"][data-translation="${code}"]`);
    const original = (await text.innerText()).trim();
    await expect(text.locator("mark").first()).toBeVisible();
    await expect(chip("slk-seb")).toHaveAttribute("aria-pressed", "true");
    await expect(row.locator('[data-role="search-hit-compare"]')).toHaveText("Porovnať");
    if (touch) {
      await assertMinTapTarget(row.locator('[data-role="hit-translation"]'), 44);
      await assertMinTapTarget(row.locator('[data-role="search-hit-compare"]'), 44);
    }

    // The KJV's Hebrews 11:1 in the row, no search asked again.
    const searches: string[] = [];
    page.on("request", (request) => {
      if (request.url().includes("/bible/search")) searches.push(request.url());
    });
    await press(chip("eng-kjv"), touch);
    await expect(text).toContainText("Now faith is the substance of things hoped for");
    await expect(hit).toContainText("Hebrejom 11:1 · KJV");
    await expect(hit).toHaveAttribute("data-translation", "eng-kjv");
    await expect(chip("eng-kjv")).toHaveAttribute("aria-pressed", "true");
    await expect(chip("slk-seb")).toHaveAttribute("aria-pressed", "false");
    // The other rows keep their own text.
    await expect(hits(page).filter({ hasText: "· KJV" })).toHaveCount(1);

    // Back to the hit: the SEB text with its marks.
    await press(chip("slk-seb"), touch);
    await expect(text).toHaveText(original);
    await expect(text.locator("mark").first()).toBeVisible();
    await expect(hit).toContainText("Hebrejom 11:1 · SEB");
    expect(searches).toEqual([]);

    // A tap on the row opens the verse in the translation it shows.
    await press(chip("eng-kjv"), touch);
    await expect(text).toContainText("Now faith");
    await press(hit, touch);
    await expect(page.locator('[data-role="study-chapter"]')).toHaveAttribute(
      "data-loaded-for",
      "eng-kjv:HEB.11",
    );
    await finish(opened);
  });

  test(`${name}: a translation lacking the verse says so, acts in the hit's own, and is asked again later`, async ({
    browser,
  }) => {
    const opened = await openStudy(
      browser,
      baseURL,
      profile,
      "/ui/study?view=search&t=slk-seb&q=viera+z%C3%A1rukou",
      null,
    );
    const { page } = opened;
    await expect(results(page)).toHaveAttribute("data-loaded-for", "slk-seb:viera zárukou");
    // The server's answer for a chapter a translation lacks: 200, no verses.
    const kjvChapter = /\/bible\/study\/chapter\?translation=eng-kjv&book_code=HEB&chapter=11$/;
    await page.route(kjvChapter, (route) =>
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ book: "Hebrews", verses: [] }),
      }),
    );
    const row = page.locator('[data-role="search-hit-row"]').filter({ hasText: "Hebrejom 11:1" }).first();
    const hit = row.locator('[data-role="search-hit"]');
    const text = row.locator('[data-role="search-hit-text"]');
    const chip = (code: string) => row.locator(`[data-role="hit-translation"][data-translation="${code}"]`);

    await press(chip("eng-kjv"), touch);
    await expect(text).toHaveText("Tento preklad tento verš nemá.");
    await expect(chip("eng-kjv")).toHaveAttribute("aria-pressed", "true");
    // Nothing to read there: the row still opens the hit's own translation.
    await expect(hit).toHaveAttribute("data-translation", "slk-seb");

    // The empty chapter was never cached: once it answers, a tap reads it.
    await page.unroute(kjvChapter);
    await press(chip("slk-seb"), touch);
    await press(chip("eng-kjv"), touch);
    await expect(text).toContainText("Now faith is the substance");
    await expect(hit).toHaveAttribute("data-translation", "eng-kjv");
    await finish(opened);
  });
}
