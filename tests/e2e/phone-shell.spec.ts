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
        await assertMinTapTarget(page.locator('[data-role="more-toggle"]'), 44);
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

test("a phone keeps Clear on Piesne; on Biblia the console field is the verse search", async ({ browser }) => {
  const { context, page, errors } = await open(browser, PHONE_390, "/ui/operator");
  try {
    const cleared = page.waitForRequest(
      (req) => req.method() === "POST" && new URL(req.url()).pathname === "/stage/clear",
    );
    await page.locator('[data-role="clear-slide"]').tap();
    await cleared;

    await page.goto("/ui/operator/bible");
    await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
    // #850 B2c: no header search on Biblia; words typed into the console
    // search the main translation's text.
    await expect(page.locator('[data-role="global-search-query"]')).toHaveCount(0);
    const field = page.locator('[data-role="console-field"]');
    await expect(field).toBeVisible();
    await page
      .locator('[data-role="console-translation"][data-kind="main"][data-translation="slk-seb"]')
      .tap();
    await expect(page.locator('[data-role="bible-console"]')).toHaveAttribute(
      "data-books-translation",
      "slk-seb",
    );
    await field.fill("viera je zárukou");
    const hit = page.locator('[data-role="console-text-search"]').first();
    await expect(hit).toBeVisible();
    await expect(hit).toHaveAttribute("data-book-code", "HEB");
    await assertNoHorizontalOverflow(page);
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

test("Viac opens and closes via ✕, the backdrop, Escape and Back", async ({ browser }) => {
  const { context, page, errors } = await open(browser, PHONE_390, "/ui/operator/bible");
  try {
    const sheet = page.locator('[data-role="more-sheet"]');
    const toggle = page.locator('[data-role="more-toggle"]');
    await expect(sheet).toHaveAttribute("data-open", "false");
    await expect(page.locator('[data-role="stage-layout-select"]')).toBeHidden();

    await toggle.tap();
    await expect(sheet).toHaveAttribute("data-open", "true");
    await expect(sheet.locator('[data-role="surface-nav-link"]')).toHaveCount(5);
    await expect(sheet.locator('[data-role="ai-status-chip"]')).toHaveCount(1);
    await expect(sheet.locator('[data-role="stage-layout-select"]')).toBeVisible();
    await page.locator('[data-role="more-sheet-close"]').tap();
    await expect(sheet).toHaveAttribute("data-open", "false");

    await toggle.tap();
    await page.locator('[data-role="more-sheet-backdrop"]').click({ position: { x: 10, y: 10 } });
    await expect(sheet).toHaveAttribute("data-open", "false");

    await toggle.tap();
    await page.keyboard.press("Escape");
    await expect(sheet).toHaveAttribute("data-open", "false");

    await toggle.tap();
    await page.goBack();
    await expect(sheet).toHaveAttribute("data-open", "false");
    await expect(page.locator("body")).toHaveAttribute("data-view", "bible");
    expect(new URL(page.url()).pathname).toBe("/ui/operator/bible");
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("crossing the phone width moves the chips without doubling their polls", async ({ browser }) => {
  // An iPad at 768 px is a phone, turned to 1024 px it is a desktop.
  const context = await browser.newContext({ baseURL, viewport: { width: 1280, height: 800 } });
  const page = await context.newPage();
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  try {
    await page.goto("/ui/operator");
    await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
    const body = page.locator("body");
    for (const [width, shell] of [
      [390, "phone"],
      [1280, "desktop"],
      [390, "phone"],
      [1280, "desktop"],
    ] as const) {
      await page.setViewportSize({ width, height: 800 });
      await expect(body).toHaveAttribute("data-shell", shell);
    }
    // On a desktop the chips are the list of the (closed) Pripojenia menu
    // (#875 H1): exactly one copy mounted, and it is the menu's.
    await expect(page.locator('[data-role="ai-status-chip"]')).toHaveCount(1);
    await expect(
      page.locator('[data-role="header-menu-panel"][data-menu="connections"] [data-role="ai-status-chip"]'),
    ).toHaveCount(1);
    await expect(page.locator('[data-role="stage-layout-select"]:visible')).toHaveCount(1);

    let polls = 0;
    page.on("request", (req) => {
      if (new URL(req.url()).pathname === "/integrations/resolume/status") polls += 1;
    });
    // One copy polls every 5 s (three in 12 s, one more if a tick runs late);
    // the three mounts of a phone-desktop-phone-desktop trip would make six.
    await page.waitForTimeout(12_000);
    expect(polls).toBeGreaterThanOrEqual(2);
    expect(polls).toBeLessThanOrEqual(4);
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("an open sheet leaves with the phone width; a closed one keeps no half-answered confirm", async ({
  browser,
}) => {
  const { context, page, errors } = await open(browser, PHONE_390, "/ui/operator");
  try {
    const sheet = page.locator('[data-role="more-sheet"]');
    const select = sheet.locator('[data-role="stage-layout-select"]');
    const toggle = page.locator('[data-role="more-toggle"]');

    await toggle.tap();
    const other = await select.evaluate((el) => {
      const s = el as HTMLSelectElement;
      return [...s.options].map((o) => o.value).find((v) => v !== s.value) ?? "";
    });
    await select.selectOption(other);
    await expect(page.locator('[data-role="stage-layout-confirm"]')).toBeVisible();
    await page.locator('[data-role="more-sheet-close"]').dblclick();
    await expect(sheet).toHaveAttribute("data-open", "false");
    // Still on the page: the double tap went back once.
    expect(new URL(page.url()).pathname).toBe("/ui/operator");
    await expect(page.locator("body")).toHaveAttribute("data-wasm-ready", "true");

    // The second tap of a double tap meets the shield, never the page under it;
    // a moment later the page takes taps again.
    await toggle.tap();
    const close = page.locator('[data-role="more-sheet-close"]');
    const underSecondTap = await close.evaluate(
      (el) =>
        new Promise<string | null>((resolve) => {
          const box = el.getBoundingClientRect();
          (el as HTMLElement).click();
          requestAnimationFrame(() => {
            const hit = document.elementFromPoint(box.x + box.width / 2, box.y + box.height / 2);
            resolve(hit?.getAttribute("data-role") ?? null);
          });
        }),
    );
    expect(underSecondTap).toBe("more-sheet-shield");
    await expect(page.locator('[data-role="more-sheet-shield"]')).toBeHidden();

    // A reload with the sheet open leaves no entry that makes Back seem dead.
    await toggle.tap();
    await page.reload();
    await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
    await expect(sheet).toHaveAttribute("data-open", "false");
    await expect
      .poll(() => page.evaluate(() => Boolean((history.state as { sheet?: boolean } | null)?.sheet)))
      .toBe(false);

    await toggle.tap();
    await expect(select).toBeVisible();
    await expect(page.locator('[data-role="stage-layout-confirm"]')).toHaveCount(0);

    await page.setViewportSize({ width: 1280, height: 800 });
    await expect(page.locator("body")).toHaveAttribute("data-shell", "desktop");
    await expect(sheet).toHaveCount(0);
    await page.setViewportSize({ width: 390, height: 844 });
    await expect(sheet).toHaveAttribute("data-open", "false");
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("a stage layout change from a phone needs a confirm (S5)", async ({ browser, request }) => {
  const { context, page, errors } = await open(browser, PHONE_390, "/ui/operator");
  const layoutPosts: string[] = [];
  page.on("request", (req) => {
    if (req.method() === "POST" && new URL(req.url()).pathname === "/stage/layout") {
      layoutPosts.push((req.postDataJSON() as { code: string }).code);
    }
  });
  try {
    const before = (await (await request.get(new URL("/stage/layout", baseURL).toString())).json()).code;
    const target = before === "preach" ? "worship-snv" : "preach";
    await page.locator('[data-role="more-toggle"]').tap();
    const select = page.locator('[data-role="more-sheet"] [data-role="stage-layout-select"]');
    const confirm = page.locator('[data-role="stage-layout-confirm"]');

    await select.selectOption(target);
    await expect(confirm).toBeVisible();
    await expect(confirm).toContainText("Prepnúť stage na");
    await expect(select).toHaveValue(before);
    await page.locator('[data-role="stage-layout-confirm-no"]').tap();
    await expect(confirm).toHaveCount(0);
    expect(layoutPosts).toEqual([]);

    await select.selectOption(target);
    await page.locator('[data-role="stage-layout-confirm-yes"]').tap();
    await expect.poll(() => layoutPosts).toEqual([target]);
    await expect(select).toHaveValue(target);
    const after = (await (await request.get(new URL("/stage/layout", baseURL).toString())).json()).code;
    expect(after).toBe(target);
    await request.post(new URL("/stage/layout", baseURL).toString(), { data: { code: before } });
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});
