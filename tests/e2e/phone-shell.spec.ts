/**
 * #850 B1a — the operator on a phone (spec §6.1–§6.4, S4): the bottom tab bar
 * replaces the burger (whose overlay never closed on 2026-10-12), the header
 * fits one row, no section scrolls sideways, every field is ≥ 16 px (iOS
 * zooms below that) and the navigation controls are ≥ 44 px. A landscape
 * phone is a phone (left rail); a desktop window, even a short one, is not.
 */
import {
  test,
  expect,
  type Browser,
  type BrowserContext,
  type BrowserContextOptions,
  type Page,
} from "@playwright/test";
import {
  assertMinFontSize,
  assertMinTapTarget,
  assertNoHorizontalOverflow,
  attachConsoleErrorCollector,
  deriveTestConfig,
  PHONE_360,
  PHONE_390,
  PHONE_LANDSCAPE,
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

const SECTIONS = [
  { view: "worship", label: "Piesne", path: "/ui/operator" },
  { view: "bible", label: "Biblia", path: "/ui/operator/bible" },
  { view: "timers", label: "Časovače", path: "/ui/operator/timers" },
  { view: "ai", label: "AI", path: "/ui/operator/ai" },
  { view: "menovky", label: "Menovky", path: "/ui/operator/menovky" },
  { view: "settings", label: "Nastavenia", path: "/ui/operator/settings" },
];

type Opened = { context: BrowserContext; page: Page; errors: string[] };

async function open(browser: Browser, profile: BrowserContextOptions, path: string): Promise<Opened> {
  const context = await browser.newContext({ ...profile, baseURL });
  const page = await context.newPage();
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  await page.goto(path);
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  return { context, page, errors };
}

test("the tab bar switches all six sections, the URL follows and Back returns", async ({
  browser,
}) => {
  const { context, page, errors } = await open(browser, PHONE_390, "/ui/operator");
  try {
    await expect(page.locator("body")).toHaveAttribute("data-shell", "phone");
    await expect(page.locator('[data-role="mobile-menu-toggle"]')).toHaveCount(0);
    await expect(page.locator('[data-role="view-toggle"]').first()).toBeHidden();
    const tabs = page.locator('[data-role="phone-tab"]');
    await expect(tabs).toHaveText(SECTIONS.map((s) => s.label));

    for (const section of SECTIONS.slice(1).concat(SECTIONS[0])) {
      await page.locator(`[data-role="phone-tab"][data-view="${section.view}"]`).tap();
      await expect(page.locator("body")).toHaveAttribute("data-view", section.view);
      expect(new URL(page.url()).pathname).toBe(section.path);
      await expect(page.locator('[data-role="phone-tab"][aria-current="page"]')).toHaveAttribute(
        "data-view",
        section.view,
      );
      await expect(page.locator(`[data-view-panel="${section.view}"]`)).toBeVisible();
    }

    // Worship was last; Back returns to Settings, then to Menovky.
    await page.goBack();
    await expect(page.locator("body")).toHaveAttribute("data-view", "settings");
    expect(new URL(page.url()).pathname).toBe("/ui/operator/settings");
    await page.goBack();
    await expect(page.locator("body")).toHaveAttribute("data-view", "menovky");

    // A tap on the section already shown adds no history entry.
    const before = await page.evaluate(() => history.length);
    await page.locator('[data-role="phone-tab"][data-view="menovky"]').tap();
    expect(await page.evaluate(() => history.length)).toBe(before);
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

for (const [name, profile] of [
  ["360x740", PHONE_360],
  ["390x844", PHONE_390],
  ["844x390", PHONE_LANDSCAPE],
] as const) {
  test(`${name}: no sideways scroll, 16 px fields, 44 px navigation in every section`, async ({
    browser,
  }) => {
    const { context, page, errors } = await open(browser, profile, "/ui/operator");
    try {
      for (const section of SECTIONS) {
        await page.goto(section.path);
        await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
        await expect(page.locator("body")).toHaveAttribute("data-shell", "phone");
        await expect(page.locator(`[data-view-panel="${section.view}"]`)).toBeVisible();
        await page.evaluate(() => document.fonts.ready);
        await assertNoHorizontalOverflow(page);
        await assertMinFontSize(page.locator("input:visible, select:visible, textarea:visible"), 16);
        await assertMinTapTarget(page.locator('[data-role="phone-tab"]'), 44);
        await assertMinTapTarget(page.locator('[data-role="mode-toggle"]'), 44);
        await assertMinTapTarget(page.locator('[data-role="stage-layout-select"]'), 44);
        if (section.view === "worship") {
          await assertMinTapTarget(page.locator('[data-role="clear-slide"]'), 44);
          await assertMinTapTarget(page.locator('[data-role="ableset-follow"]'), 44);
        }
        // Worship and, until the console replaces it, Bible keep a search row.
        if (section.view !== "worship" && section.view !== "bible") {
          const header = await page.locator(".operator__header").boundingBox();
          expect(header?.height ?? 999, `${section.view} header height`).toBeLessThanOrEqual(56);
        }
      }
      expect(errors).toEqual([]);
    } finally {
      await context.close();
    }
  });
}

test("a phone keeps Clear on Piesne and the verse search on Biblia", async ({ browser }) => {
  const { context, page, errors } = await open(browser, PHONE_390, "/ui/operator");
  try {
    const cleared = page.waitForRequest(
      (req) => req.method() === "POST" && new URL(req.url()).pathname === "/stage/clear",
    );
    await page.locator('[data-role="clear-slide"]').tap();
    await cleared;

    await page.goto("/ui/operator/bible");
    await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
    await expect(page.locator('[data-role="global-search-query"]')).toBeVisible();
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("the page height follows the visible area, not the tallest viewport", async ({ browser }) => {
  const { context, page, errors } = await open(browser, PHONE_390, "/ui/operator");
  try {
    // A 100vh floor would keep the tab bar under a showing browser toolbar.
    expect(await page.evaluate(() => getComputedStyle(document.body).minHeight)).toBe("0px");
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("an open keyboard on a small portrait phone leaves the tab bar at the bottom", async ({
  browser,
}) => {
  const { context, page, errors } = await open(browser, PHONE_360, "/ui/operator/bible");
  try {
    // resizes-content: 360x640 with a 300 px keyboard is wider than tall.
    await page.setViewportSize({ width: 360, height: 340 });
    await expect(page.locator("body")).toHaveAttribute("data-shell", "phone");
    await expect
      .poll(async () => (await page.locator('[data-role="phone-tab-bar"]').boundingBox())?.width)
      .toBe(360);
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("a landscape phone gets the left rail; a desktop window keeps the top nav", async ({
  browser,
}) => {
  const phone = await open(browser, PHONE_LANDSCAPE, "/ui/operator/bible");
  try {
    const rail = await phone.page.locator('[data-role="phone-tab-bar"]').boundingBox();
    expect(rail).not.toBeNull();
    expect(rail!.x).toBe(0);
    expect(rail!.width).toBeGreaterThanOrEqual(64);
    expect(rail!.height).toBeGreaterThanOrEqual(380);
    expect(phone.errors).toEqual([]);
  } finally {
    await phone.context.close();
  }

  // A short desktop window has a fine pointer: still the desktop shell.
  for (const viewport of [
    { width: 1280, height: 800 },
    { width: 844, height: 390 },
  ]) {
    const desktop = await open(browser, { viewport }, "/ui/operator");
    try {
      await expect(desktop.page.locator("body")).toHaveAttribute("data-shell", "desktop");
      await expect(desktop.page.locator('[data-role="phone-tab-bar"]')).toHaveCount(0);
      await expect(desktop.page.locator('[data-role="view-toggle"]')).toHaveText(
        SECTIONS.map((s) => s.label),
      );
      if (viewport.width === 1280) {
        await expect(desktop.page.locator('[data-role="stage-preview-frame"]')).toHaveCount(1);
      }
      expect(desktop.errors).toEqual([]);
    } finally {
      await desktop.context.close();
    }
  }
});

test("a phone on Piesne never mounts the stage-preview iframe (no second app)", async ({
  browser,
}) => {
  const { context, page, errors } = await open(browser, PHONE_390, "/ui/operator");
  try {
    await expect(page.locator("body")).toHaveAttribute("data-view", "worship");
    await expect(page.locator('[data-role="stage-preview-frame"]')).toHaveCount(0);
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("the Bible tab works on a phone today: 'zid' finds Hebrejom (resolver fallback)", async ({
  browser,
}) => {
  const { context, page, errors } = await open(browser, PHONE_390, "/ui/operator/bible");
  try {
    const main = page.locator('[data-role="main-translation"]');
    await main.selectOption("slk-seb");
    const bookList = page.locator('[data-role="book-list"]');
    await expect(bookList).toHaveAttribute("data-books-translation", "slk-seb");
    const filter = page.locator('[data-role="book-filter"]');
    await filter.fill("zid");
    const items = bookList.locator('[data-role="book-item"]');
    await expect(items).toHaveCount(1);
    await expect(items).toHaveAttribute("data-book-code", "HEB");
    await filter.press("Enter");
    await expect(bookList.locator('[data-role="book-item"][data-active="true"]')).toHaveAttribute(
      "data-book-code",
      "HEB",
    );
    await assertNoHorizontalOverflow(page);
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});
