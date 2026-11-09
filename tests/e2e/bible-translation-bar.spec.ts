/**
 * #921 — the console's translation rows with many translations (owner
 * 2026-11-09: "keď budem mať 100 prekladov, celá plocha budú tlačidlá").
 * Each row is ONE line: the preferred translations that fit + ▾ with every
 * installed one. No label is clipped, nothing scrolls sideways, a pick from ▾
 * selects and the row shows it, and the preferred chips are a setting.
 */
import {
  test,
  expect,
  type Browser,
  type BrowserContextOptions,
  type Page,
} from "@playwright/test";
import {
  assertMinTapTarget,
  assertNoHorizontalOverflow,
  attachConsoleErrorCollector,
  deriveTestConfig,
  PHONE_360,
  PHONE_390,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";
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

const DEFAULT_PREFERENCES = {
  mainTranslation: "slk-seb",
  secondaryTranslation: "eng-nlt",
  characterLimit: 320,
  preferredMain: ["slk-seb", "slk-roh", "slk-sevp", "eng-kjv"],
  preferredSecondary: ["eng-nlt", "eng-kjv"],
};

test.beforeEach(async ({ request }) => {
  const prefs = await request.put(new URL("/bible/preferences", baseURL).toString(), {
    data: DEFAULT_PREFERENCES,
  });
  expect(prefs.ok()).toBeTruthy();
});

/** CI's six translations + the four SNV has beyond them. */
const INSTALLED = 6 + EXTRA_TRANSLATIONS.length;

const DESKTOP_NARROW = { viewport: { width: 1024, height: 768 } };
const DESKTOP_WIDE = { viewport: { width: 1920, height: 1080 } };

async function openConsole(browser: Browser, profile: BrowserContextOptions) {
  const context = await browser.newContext({ ...profile, baseURL });
  const page = await context.newPage();
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  await routeExtraTranslations(page);
  await page.goto("/ui/operator/bible");
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  await expect(page.locator('[data-role="bible-console"]')).toHaveAttribute(
    "data-books-translation",
    "slk-seb",
  );
  return { context, page, errors };
}

const row = (page: Page, kind: "main" | "secondary") =>
  page.locator(`.console-translations__row[data-row="${kind}"]`);
const chips = (page: Page, kind: "main" | "secondary") =>
  row(page, kind).locator('[data-role="console-translation"]');
const menu = (page: Page, kind: "main" | "secondary") =>
  page.locator(`[data-role="console-translation-menu"][data-kind="${kind}"]`);
const option = (page: Page, kind: "main" | "secondary", code: string) =>
  page.locator(
    `[data-role="translation-list"][data-kind="${kind}"] [data-role="translation-option"][data-translation="${code}"]`,
  );

/**
 * One line, no clipped label, nothing past the panel: every button of the row
 * shares one top, each label fits its chip, the left column does not scroll.
 */
async function assertRowsFit(page: Page): Promise<void> {
  await page.evaluate(() => document.fonts.ready);
  const report = await page.evaluate(() => {
    const rows = Array.from(document.querySelectorAll<HTMLElement>(".console-translations__row"));
    return rows.map((r) => {
      const buttons = Array.from(r.querySelectorAll<HTMLElement>("button"));
      const tops = buttons.map((b) => Math.round(b.getBoundingClientRect().top));
      const clipped = buttons
        .filter((b) => b.scrollWidth > b.clientWidth)
        .map((b) => `${b.textContent?.trim()} ${b.scrollWidth}>${b.clientWidth}`);
      const rowRect = r.getBoundingClientRect();
      const outside = buttons
        .filter((b) => b.getBoundingClientRect().right > rowRect.right + 0.5)
        .map((b) => b.textContent?.trim());
      return { row: r.dataset.row, tops: [...new Set(tops)], clipped, outside };
    });
  });
  for (const r of report) {
    expect(r.tops, `${r.row}: one line`).toHaveLength(1);
    expect(r.clipped, `${r.row}: clipped labels`).toEqual([]);
    expect(r.outside, `${r.row}: past the row`).toEqual([]);
  }
  const column = await page
    .locator(".operator__catalog--bible")
    .evaluate((el) => el.scrollWidth - el.clientWidth);
  expect(column, "the Biblia column scrolls sideways").toBeLessThanOrEqual(0);
  await assertNoHorizontalOverflow(page);
}

for (const [name, profile, cap] of [
  ["360", PHONE_360, 3],
  ["390", PHONE_390, 3],
  ["desktop 1024", DESKTOP_NARROW, 4],
  ["desktop 1920", DESKTOP_WIDE, 4],
] as const) {
  test(`${name}: each row is one line, ▾ lists every translation, a pick shows in the row`, async ({
    browser,
  }) => {
    const { context, page, errors } = await openConsole(browser, profile);
    try {
      // The preferred chips that fit, SEB on; the line below one fewer + žiadny.
      await expect(chips(page, "main")).toHaveText(
        ["SEB", "ROH", "SEVP", "KJV"].slice(0, cap),
      );
      await expect(chips(page, "main").first()).toHaveAttribute("aria-pressed", "true");
      await expect(row(page, "secondary").locator('[data-translation="eng-nlt"]')).toHaveAttribute(
        "aria-pressed",
        "true",
      );
      await expect(chips(page, "secondary")).toHaveCount(Math.min(2, cap - 1) + 1);
      // ▾ never looks selected.
      await expect(menu(page, "main")).not.toHaveAttribute("aria-pressed", /.*/);
      const tap = profile === DESKTOP_NARROW || profile === DESKTOP_WIDE ? 36 : 44;
      await assertMinTapTarget(row(page, "main").locator("button"), tap);
      await assertRowsFit(page);
      const heightBefore = await row(page, "main").evaluate((el) => el.getBoundingClientRect().height);

      // ▾ main: every installed translation but the NLT, grouped by language.
      await menu(page, "main").click();
      const mainList = page.locator('[data-role="translation-list"][data-kind="main"]');
      await expect(mainList.locator('[data-role="translation-option"]')).toHaveCount(INSTALLED - 1);
      await expect(mainList.locator(".translation-list__heading")).toHaveText([
        "Slovenské",
        "České",
        "Anglické",
      ]);
      await expect(option(page, "main", "slk-seb")).toHaveAttribute("aria-pressed", "true");
      await expect(option(page, "main", "eng-nlt")).toHaveCount(0);
      await expect(option(page, "main", "ces-csp")).toContainText("Český studijní překlad");
      await assertNoHorizontalOverflow(page);

      // A pick selects it, closes the list and takes the last chip's place.
      await option(page, "main", "ces-csp").click();
      await expect(mainList).toHaveCount(0);
      await expect(page.locator('[data-role="bible-console"]')).toHaveAttribute(
        "data-books-translation",
        "ces-csp",
      );
      await expect(chips(page, "main")).toHaveCount(cap);
      await expect(chips(page, "main").last()).toHaveText("CSP");
      await expect(chips(page, "main").last()).toHaveAttribute("aria-pressed", "true");
      await expect(row(page, "main").locator('[aria-pressed="true"]')).toHaveCount(1);
      expect(await row(page, "main").evaluate((el) => el.getBoundingClientRect().height)).toBe(
        heightBefore,
      );

      // ▾ second: every translation, the NLT too; AMPC goes under the main one.
      await menu(page, "secondary").click();
      const secondList = page.locator('[data-role="translation-list"][data-kind="secondary"]');
      await expect(secondList.locator('[data-role="translation-option"]')).toHaveCount(INSTALLED);
      await option(page, "secondary", "eng-ampc").click();
      await expect(secondList).toHaveCount(0);
      const ampc = row(page, "secondary").locator('[data-translation="eng-ampc"]');
      await expect(ampc).toHaveText("AMPC");
      await expect(ampc).toHaveAttribute("aria-pressed", "true");
      await assertRowsFit(page);

      // Escape and a press beside the list close it without a pick.
      await menu(page, "main").click();
      await expect(mainList).toBeVisible();
      await page.keyboard.press("Escape");
      await expect(mainList).toHaveCount(0);
      // Escape closes it with the focus anywhere (Safari never focuses a clicked button).
      await menu(page, "main").click();
      await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
      await page.keyboard.press("Escape");
      await expect(mainList).toHaveCount(0);
      const fieldBox = await page.locator('[data-role="console-field"]').boundingBox();
      if (!fieldBox) throw new Error("the console field has no box");
      await menu(page, "main").click();
      await page.mouse.click(fieldBox.x + fieldBox.width / 2, fieldBox.y + fieldBox.height / 2);
      await expect(mainList).toHaveCount(0);
      await expect(page.locator('[data-role="bible-console"]')).toHaveAttribute(
        "data-books-translation",
        "ces-csp",
      );

      // Every translation can be the main one and the second one.
      for (const code of ["slk-seb", "slk-roh", "slk-sevp", "slk-ssv", "slk-bot", "slk-mil", "eng-kjv", "eng-ampc"]) {
        await menu(page, "main").click();
        await option(page, "main", code).click();
        await expect(page.locator('[data-role="bible-console"]')).toHaveAttribute(
          "data-books-translation",
          code,
        );
        await expect(row(page, "main").locator(`[data-translation="${code}"]`)).toHaveAttribute(
          "aria-pressed",
          "true",
        );
      }
      for (const code of ["slk-seb", "slk-mil", "ces-csp", "eng-kjv", "eng-nlt"]) {
        await menu(page, "secondary").click();
        await option(page, "secondary", code).click();
        await expect(row(page, "secondary").locator(`[data-translation="${code}"]`)).toHaveAttribute(
          "aria-pressed",
          "true",
        );
      }
      await assertRowsFit(page);
      expect(errors).toEqual([]);
    } finally {
      await context.close();
    }
  });
}

test("the preferred translations are a setting: Nastavenie changes the chips, a reload keeps them", async ({
  browser,
  request,
}) => {
  const { context, page, errors } = await openConsole(browser, DESKTOP_WIDE);
  try {
    await page.locator('[data-role="bible-tab"][data-tab="settings"]').click();
    const box = (kind: string, code: string) =>
      page.locator(
        `[data-role="preferred-translation"][data-kind="${kind}"][data-translation="${code}"]`,
      );
    await expect(box("main", "slk-roh")).toBeChecked();
    await expect(box("main", "eng-nlt")).toHaveCount(0);
    await expect(box("secondary", "eng-nlt")).toBeChecked();

    const saved = () =>
      page.waitForResponse(
        (res) =>
          new URL(res.url()).pathname === "/bible/preferences" && res.request().method() === "PUT",
      );
    let put = saved();
    await box("main", "slk-roh").uncheck();
    await put;
    put = saved();
    await box("main", "ces-csp").check();
    await put;
    put = saved();
    await box("secondary", "eng-ampc").check();
    await put;

    const read = async () =>
      (await request.get(new URL("/bible/preferences", baseURL).toString())).json();
    const stored = await read();
    expect(stored.preferredMain).toEqual(["slk-seb", "slk-sevp", "eng-kjv", "ces-csp"]);
    expect(stored.preferredSecondary).toEqual(["eng-nlt", "eng-kjv", "eng-ampc"]);

    // Three quick ticks in both rows, no wait between them: none is lost.
    await box("main", "slk-sevp").uncheck();
    await box("secondary", "eng-ampc").uncheck();
    await box("main", "slk-mil").check();
    await expect
      .poll(async () => {
        const now = await read();
        return [now.preferredMain, now.preferredSecondary];
      })
      .toEqual([
        ["slk-seb", "eng-kjv", "ces-csp", "slk-mil"],
        ["eng-nlt", "eng-kjv"],
      ]);
    await box("main", "slk-mil").uncheck();
    await box("main", "slk-sevp").check();
    await box("secondary", "eng-ampc").check();
    await expect
      .poll(async () => (await read()).preferredMain)
      .toEqual(["slk-seb", "eng-kjv", "ces-csp", "slk-sevp"]);
    await expect.poll(async () => (await read()).preferredSecondary).toEqual([
      "eng-nlt",
      "eng-kjv",
      "eng-ampc",
    ]);

    await page.reload();
    await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
    await expect(chips(page, "main")).toHaveText(["SEB", "KJV", "CSP", "SEVP"]);
    await expect(row(page, "secondary").locator('[data-translation="eng-ampc"]')).toHaveText("AMPC");
    await assertRowsFit(page);
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});
