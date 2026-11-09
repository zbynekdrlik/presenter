/**
 * #921 — every installed translation in Study (owner 2026-11-09: "chcem mať
 * tieto novo pridané preklady Biblie aj v štúdiu"). The favourites row stays
 * one row: the favourites that fit + ▾, which opens all of them grouped by
 * language (`s=preklady`, Back closes it). A pick reads that translation at
 * the same place and the row shows it in the last chip's place; the
 * favourites stay as they are. Nothing here reaches a wall (T11).
 */
import { test, expect } from "@playwright/test";
import {
  assertMinTapTarget,
  assertNoHorizontalOverflow,
  deriveTestConfig,
  PHONE_360,
  PHONE_390,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";
import { expectChapter, favourite, finish, openStudy } from "./study-support";
import { EXTRA_TRANSLATIONS, routeExtraTranslations } from "./extra-translations";

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

/** CI's six translations + the four SNV has beyond them. */
const INSTALLED = 6 + EXTRA_TRANSLATIONS.length;

for (const [name, profile] of [
  ["360", PHONE_360],
  ["390", PHONE_390],
] as const) {
  test(`${name}: ▾ lists every translation, a pick reads it and the row shows it`, async ({
    browser,
  }) => {
    const opened = await openStudy(
      browser,
      baseURL,
      profile,
      "/ui/study?t=slk-seb&p=HEB.11",
      "slk-seb:HEB.11",
      routeExtraTranslations,
    );
    const { page } = opened;
    const chips = page.locator('[data-role="study-fav"]');
    const open = page.locator('[data-role="study-translations-open"]');
    const sheet = page.locator('[data-role="study-translations-sheet"]');

    // One row: four favourites on a phone under 420 px, then ▾.
    await expect(chips).toHaveCount(4);
    await expect(open).toBeVisible();
    const rowTops = await page
      .locator('[data-role="study-favs"] button')
      .evaluateAll((els) => [...new Set(els.map((el) => Math.round(el.getBoundingClientRect().top)))]);
    expect(rowTops).toHaveLength(1);
    await assertMinTapTarget(page.locator('[data-role="study-favs"] button'), 44);
    await assertNoHorizontalOverflow(page);

    await open.tap();
    await expect(sheet).toBeVisible();
    await expect(page).toHaveURL(/[?&]s=preklady(&|$)/);
    const options = sheet.locator('[data-role="translation-option"]');
    await expect(options).toHaveCount(INSTALLED);
    await expect(sheet.locator(".translation-list__heading")).toHaveText([
      "Slovenské",
      "České",
      "Anglické",
    ]);
    await expect(sheet.locator('[data-translation="slk-seb"]')).toHaveAttribute("aria-pressed", "true");
    await expect(sheet.locator('[data-translation="eng-ampc"]')).toContainText("Amplified Bible");
    await assertMinTapTarget(options, 44);
    await assertNoHorizontalOverflow(page);

    // Back closes it, nothing changed.
    await page.goBack();
    await expect(sheet).toHaveCount(0);
    await expectChapter(page, "slk-seb:HEB.11");

    // AMPC: read at the same place, the last chip shows it, the favourites stay.
    await open.tap();
    await sheet.locator('[data-translation="eng-ampc"]').tap();
    await expect(sheet).toHaveCount(0);
    await expectChapter(page, "eng-ampc:HEB.11");
    await expect(chips).toHaveCount(4);
    await expect(chips.last()).toHaveAttribute("data-translation", "eng-ampc");
    await expect(favourite(page, "eng-ampc")).toHaveAttribute("aria-pressed", "true");
    await expect(page.locator('[data-role="study-fav"][aria-pressed="true"]')).toHaveCount(1);

    // CSP the same way; a favourite chip goes back, and the row is the favourites again.
    await open.tap();
    await sheet.locator('[data-translation="ces-csp"]').tap();
    await expectChapter(page, "ces-csp:HEB.11");
    await expect(chips.last()).toHaveAttribute("data-translation", "ces-csp");
    await favourite(page, "slk-roh").tap();
    await expectChapter(page, "slk-roh:HEB.11");
    await expect(chips).toHaveText(["SEB", "ROH", "SEVP", "KJV"]);

    // The fifth favourite (NLT) lives in ▾ on a narrow phone.
    await open.tap();
    await expect(sheet.locator('[data-translation="eng-nlt"]')).toBeVisible();
    await sheet.locator('[data-translation="slk-roh"]').tap();
    await expect(sheet).toHaveCount(0);
    await expectChapter(page, "slk-roh:HEB.11");
    await assertNoHorizontalOverflow(page);
    await finish(opened);
  });
}

test("a wider screen keeps the five favourites beside ▾", async ({ browser }) => {
  const opened = await openStudy(browser, baseURL, { viewport: { width: 1024, height: 768 } });
  const { page } = opened;
  await expect(page.locator('[data-role="study-fav"]')).toHaveText(["SEB", "ROH", "SEVP", "KJV", "NLT"]);
  await expect(page.locator('[data-role="study-translations-open"]')).toBeVisible();
  await assertNoHorizontalOverflow(page);
  await finish(opened);
});
