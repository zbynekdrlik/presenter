/**
 * #850 B2a — the verse console (spec §7, §10). The owner's 2026-10-12 input
 * `zid 11 1` resolves to Hebrejom 11:1 in the browser, its slides load while
 * typing, and the passage reaches the wall. One server for the file; every
 * test starts from main = slk-seb and an empty wall.
 */
import {
  test,
  expect,
  type Browser,
  type BrowserContext,
  type BrowserContextOptions,
  type Page,
  type Request,
  type WebSocketRoute,
} from "@playwright/test";
import {
  assertMinFontSize,
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

test.beforeEach(async ({ request }) => {
  const prefs = await request.put(new URL("/bible/preferences", baseURL).toString(), {
    data: { mainTranslation: "slk-seb", characterLimit: 320 },
  });
  expect(prefs.ok()).toBeTruthy();
  const clear = await request.post(new URL("/bible/clear", baseURL).toString(), { data: {} });
  expect(clear.ok()).toBeTruthy();
});

type Opened = { context: BrowserContext; page: Page; errors: string[] };

async function openConsole(browser: Browser, profile: BrowserContextOptions): Promise<Opened> {
  const context = await browser.newContext({ ...profile, baseURL });
  const page = await context.newPage();
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  await page.goto("/ui/operator/bible");
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  await expect(page.locator('[data-role="book-list"]')).toHaveAttribute(
    "data-books-translation",
    "slk-seb",
  );
  await expect(page.locator('[data-role="console-field"]')).toBeVisible();
  return { context, page, errors };
}

const consoleRoot = (page: Page) => page.locator('[data-role="bible-console"]');

/** Type into the console and wait until its slides are loaded for exactly this input. */
async function lookUp(page: Page, text: string, reference: string): Promise<void> {
  await page.locator('[data-role="console-field"]').fill(text);
  await expect(consoleRoot(page)).toHaveAttribute("data-preview-ready", "true");
  await expect(consoleRoot(page)).toHaveAttribute("data-preview-reference", reference);
}

const showButton = (page: Page) => page.locator('[data-role="console-show"]');

function collectPosts(page: Page, path: string): unknown[] {
  const bodies: unknown[] = [];
  page.on("request", (req: Request) => {
    if (req.method() === "POST" && new URL(req.url()).pathname === path) {
      bodies.push(req.postDataJSON());
    }
  });
  return bodies;
}

test("phone: 'zid 11 1' → one tap on ZOBRAZIŤ puts Hebrejom 11:1 on the wall and the stage (S6)", async ({
  browser,
  request,
}) => {
  const { context, page, errors } = await openConsole(browser, PHONE_390);
  try {
    const shows = collectPosts(page, "/bible/trigger-slide");
    await lookUp(page, "zid 11 1", "Hebrejom 11:1 (SEB)");
    const row = page.locator('[data-role="console-row"]');
    await expect(row).toHaveCount(1);
    await expect(row).toHaveAttribute("data-kind", "passage");
    await expect(row).toHaveAttribute("data-book-code", "HEB");
    await expect(row).toHaveAttribute("data-chapter", "11");
    await expect(row).toHaveAttribute("data-verse-start", "1");
    await expect(row).toHaveAttribute("data-verse-end", "1");
    await expect(row).toContainText("Hebrejom 11:1");
    await expect(row).toContainText("SEB");
    const preview = page.locator('[data-role="console-preview"]');
    await expect(preview).toHaveAttribute("data-ready", "true");
    await expect(preview).toHaveAttribute("data-reference", "Hebrejom 11:1 (SEB)");
    await expect(page.locator('[data-role="console-preview-main"]')).toContainText("Viera");
    // On a phone the console IS the Live tab: no old inputs, no slide grid.
    await expect(page.locator('[data-role="book-filter"]')).toBeHidden();
    await expect(page.locator('[data-role="slide-card"]').first()).toBeHidden();
    await expect(page.locator('[data-role="console-on-air"]')).toHaveAttribute("data-live", "false");

    await page.locator('[data-role="console-show"]').tap(); // the one tap (1 field + 1 tap)
    await expect.poll(() => shows.length).toBe(1);
    expect(shows[0]).toMatchObject({ mainReference: "Hebrejom 11:1 (SEB)", bookCode: "HEB" });
    const onAir = page.locator('[data-role="console-on-air"]');
    await expect(onAir).toHaveAttribute("data-live", "true");
    await expect(page.locator('[data-role="console-on-air-reference"]')).toHaveText("Hebrejom 11:1");
    await expect(page.locator('[data-role="console-on-air-codes"]')).toContainText("SEB");
    const active = await (await request.get(new URL("/bible/active-slide", baseURL).toString())).json();
    expect(active.mainReference).toBe("Hebrejom 11:1 (SEB)");

    // Every output reads the same slide: the stage on its Bible layout shows it.
    await request.post(new URL("/stage/layout", baseURL).toString(), { data: { code: "bible" } });
    const stage = await context.newPage();
    try {
      await stage.goto("/stage");
      await stage.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
      await expect(stage.locator(".stage__bible-reference")).toHaveText("Hebrejom 11:1 (SEB)");
      await expect(stage.locator(".stage__bible-text")).toContainText("Viera");
    } finally {
      await request.post(new URL("/stage/layout", baseURL).toString(), {
        data: { code: "worship-snv" },
      });
      await stage.close();
    }
    expect(shows).toHaveLength(1);
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("switching the main translation re-resolves at once: Hebrejom in SEB, Židom in ROH", async ({
  browser,
}) => {
  const { context, page, errors } = await openConsole(browser, PHONE_390);
  try {
    await lookUp(page, "zid 11 1", "Hebrejom 11:1 (SEB)");
    await page.locator('[data-role="main-translation"]').selectOption("slk-roh");
    await expect(page.locator('[data-role="book-list"]')).toHaveAttribute(
      "data-books-translation",
      "slk-roh",
    );
    await expect(page.locator('[data-role="console-row"]')).toContainText("Židom 11:1");
    await expect(page.locator('[data-role="console-row"]')).toContainText("ROH");
    await expect(consoleRoot(page)).toHaveAttribute("data-preview-reference", "Židom 11:1 (ROH)");
    await expect(consoleRoot(page)).toHaveAttribute("data-preview-ready", "true");
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("a reference that does not exist, numbers without a book and a book alone each say what to do", async ({
  browser,
}) => {
  const { context, page, errors } = await openConsole(browser, PHONE_360);
  try {
    const field = page.locator('[data-role="console-field"]');
    const hint = page.locator('[data-role="console-hint"]');
    await field.fill("zid 14");
    await expect(hint).toHaveText("Kniha má len 13 kapitol");
    await expect(page.locator('[data-role="console-row"]')).toHaveCount(0);
    await expect(consoleRoot(page)).toHaveAttribute("data-preview-state", "empty");

    await field.fill("11:1");
    await expect(hint).toHaveText("Napíš aj knihu");
    await expect(page.locator('[data-role="console-row"]')).toHaveCount(0);

    await field.fill("jan");
    const bookOnly = page.locator('[data-role="console-row"][data-kind="book_only"]');
    await expect(bookOnly).toContainText("Ján");
    await bookOnly.tap();
    await expect(field).toHaveValue("Ján ");
    await expect(field).toBeFocused();
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("a verse list is one row per verse and a tap on a row loads that verse", async ({ browser }) => {
  const { context, page, errors } = await openConsole(browser, PHONE_390);
  try {
    await lookUp(page, "mt 5:3.5.7", "Matúš 5:3 (SEB)");
    const rows = page.locator('[data-role="console-row"]');
    await expect(rows).toHaveCount(3);
    await expect(rows.nth(0)).toHaveAttribute("data-default", "true");
    await rows.nth(1).tap();
    await expect(rows.nth(1)).toHaveAttribute("aria-pressed", "true");
    await expect(consoleRoot(page)).toHaveAttribute("data-preview-reference", "Matúš 5:5 (SEB)");
    await expect(consoleRoot(page)).toHaveAttribute("data-preview-ready", "true");
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("a slow answer for an older input never replaces the newer one", async ({ browser }) => {
  const { context, page, errors } = await openConsole(browser, PHONE_390);
  try {
    let releaseFirst: () => void = () => {};
    const firstHeld = new Promise<void>((resolve) => {
      releaseFirst = resolve;
    });
    let held = 0;
    await page.route("**/bible/resolve", async (route) => {
      const body = route.request().postDataJSON() as { bookCode?: string };
      if (body.bookCode === "HEB" && held === 0) {
        held += 1;
        await firstHeld;
      }
      await route.continue();
    });
    const field = page.locator('[data-role="console-field"]');
    await field.fill("zid 11 1");
    await expect.poll(() => held).toBe(1);
    await field.fill("rim 8 28");
    await expect(consoleRoot(page)).toHaveAttribute("data-preview-reference", "Rimanom 8:28 (SEB)");
    const late = page.waitForResponse(
      (res) => new URL(res.url()).pathname === "/bible/resolve" &&
        (res.request().postDataJSON() as { bookCode?: string }).bookCode === "HEB",
    );
    releaseFirst();
    await (await late).finished();
    await expect(consoleRoot(page)).toHaveAttribute("data-preview-reference", "Rimanom 8:28 (SEB)");
    await expect(consoleRoot(page)).toHaveAttribute("data-preview-ready", "true");
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("a failed load says so inline and Enter tries again", async ({ browser }) => {
  const { context, page, errors } = await openConsole(browser, PHONE_390);
  try {
    // A malformed 200, never a mocked 5xx: Chrome logs every non-2xx (ui skill #598).
    await page.route("**/bible/resolve", (route) =>
      route.fulfill({ status: 200, contentType: "application/json", body: "not-json" }),
    );
    const field = page.locator('[data-role="console-field"]');
    await field.fill("zid 11 1");
    await expect(consoleRoot(page)).toHaveAttribute("data-preview-state", "error");
    await expect(page.locator('[data-role="console-preview-error"]')).toContainText(
      "Náhľad sa nenačítal",
    );
    await expect(page.locator('[data-role="toast"]')).toContainText("Náhľad sa nenačítal");
    await page.unroute("**/bible/resolve");
    await field.press("Enter");
    await expect(consoleRoot(page)).toHaveAttribute("data-preview-ready", "true");
    await expect(consoleRoot(page)).toHaveAttribute("data-preview-reference", "Hebrejom 11:1 (SEB)");
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("phone fit: the field is 16 px with autocorrect off, controls are 44 px, nothing scrolls sideways", async ({
  browser,
}) => {
  const { context, page, errors } = await openConsole(browser, PHONE_360);
  try {
    const field = page.locator('[data-role="console-field"]');
    await expect(field).toHaveAttribute("enterkeyhint", "go");
    await expect(field).toHaveAttribute("autocorrect", "off");
    await expect(field).toHaveAttribute("autocapitalize", "off");
    await expect(field).toHaveAttribute("spellcheck", "false");
    await lookUp(page, "zid 11 1", "Hebrejom 11:1 (SEB)");
    await assertMinFontSize(field, 16);
    await assertMinTapTarget(page.locator('[data-role="console-row"]'), 44);
    await assertMinTapTarget(page.locator('[data-role="console-clear-field"]'), 44);
    await assertNoHorizontalOverflow(page);
    // Enter ("Go") closes the keyboard: the field loses focus.
    await field.focus();
    await field.press("Enter");
    await expect(field).not.toBeFocused();
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("desktop: the console sits above the old inputs and feeds the slide grid", async ({
  browser,
}) => {
  const { context, page, errors } = await openConsole(browser, { viewport: { width: 1280, height: 800 } });
  try {
    await expect(page.locator("body")).toHaveAttribute("data-shell", "desktop");
    await expect(page.locator('[data-role="book-filter"]')).toBeVisible();
    await lookUp(page, "zid 11 1", "Hebrejom 11:1 (SEB)");
    await expect(page.locator('[data-role="slide-card"]').first()).toContainText(
      "Hebrejom 11:1 (SEB)",
    );
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("desktop: the grid never keeps an older verse a card tap could send", async ({ browser }) => {
  const { context, page, errors } = await openConsole(browser, { viewport: { width: 1280, height: 800 } });
  try {
    const cards = page.locator('[data-role="slide-card"]');
    await lookUp(page, "zid 11 1", "Hebrejom 11:1 (SEB)");
    await expect(cards.first()).toContainText("Hebrejom 11:1 (SEB)");

    // A reference that does not exist: nothing of Hebrews stays on offer.
    const field = page.locator('[data-role="console-field"]');
    await field.fill("zid 14");
    await expect(cards).toHaveCount(0);

    // While the next passage loads, the grid is empty.
    let release: () => void = () => {};
    const held = new Promise<void>((resolve) => {
      release = resolve;
    });
    await page.route("**/bible/resolve", async (route) => {
      await held;
      await route.continue();
    });
    await field.fill("rim 8 28");
    await expect(consoleRoot(page)).toHaveAttribute("data-preview-state", "loading");
    await expect(cards).toHaveCount(0);
    release();
    await expect(cards.first()).toContainText("Rimanom 8:28 (SEB)");
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("desktop: a load from the old inputs takes the grid, the console lets go", async ({ browser }) => {
  const { context, page, errors } = await openConsole(browser, { viewport: { width: 1280, height: 800 } });
  try {
    const cards = page.locator('[data-role="slide-card"]');
    await lookUp(page, "zid 11 1", "Hebrejom 11:1 (SEB)");

    await page.locator('[data-role="book-filter"]').fill("Rimanom");
    await page.locator('[data-role="book-item"]').first().click();
    await page.locator('[data-role="load-button"]').click();
    await expect(cards.first()).toContainText("Rimanom 1:1-32 (SEB)");
    await expect(page.locator('[data-role="console-field"]')).toHaveValue("");

    // A later translation or limit change has no console target to reload.
    await page.locator('[data-role="secondary-translation"]').selectOption({ index: 2 });
    await expect(consoleRoot(page)).toHaveAttribute("data-preview-state", "empty");
    await expect(cards.first()).toContainText("Rimanom 1:1");
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("desktop: an old-inputs answer that lands after the console took over is dropped", async ({
  browser,
}) => {
  const { context, page, errors } = await openConsole(browser, { viewport: { width: 1280, height: 800 } });
  try {
    let release: () => void = () => {};
    const held = new Promise<void>((resolve) => {
      release = resolve;
    });
    let heldCount = 0;
    await page.route("**/bible/resolve", async (route) => {
      const body = route.request().postDataJSON() as { bookCode?: string };
      if (body.bookCode === "ROM") {
        heldCount += 1;
        await held;
      }
      await route.continue();
    });
    await page.locator('[data-role="book-filter"]').fill("Rimanom");
    await page.locator('[data-role="book-item"]').first().click();
    await page.locator('[data-role="load-button"]').click();
    await expect.poll(() => heldCount).toBe(1);

    await lookUp(page, "zid 11 1", "Hebrejom 11:1 (SEB)");
    const late = page.waitForResponse(
      (res) =>
        new URL(res.url()).pathname === "/bible/resolve" &&
        (res.request().postDataJSON() as { bookCode?: string }).bookCode === "ROM",
    );
    release();
    await (await late).finished();
    await expect(page.locator('[data-role="slide-card"]').first()).toContainText(
      "Hebrejom 11:1 (SEB)",
    );
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("a long passage shows part by part: ZOBRAZIŤ 1/n, then 2/n", async ({ browser }) => {
  const { context, page, errors } = await openConsole(browser, PHONE_390);
  try {
    const shows = collectPosts(page, "/bible/trigger-slide");
    await page.locator('[data-role="console-field"]').fill("jan 3 16-21");
    const preview = page.locator('[data-role="console-preview"]');
    await expect(preview).toHaveAttribute("data-ready", "true");
    const count = Number(await preview.getAttribute("data-slide-count"));
    expect(count).toBeGreaterThan(1);
    await expect(showButton(page)).toHaveText(`ZOBRAZIŤ 1/${count}`);
    await showButton(page).tap();
    // #850 B2b: after a show the primary button offers the next part.
    await expect(showButton(page)).toHaveText(`ĎALŠÍ: 3:16-21 2/${count}`);
    await expect(preview).toHaveAttribute("data-slide-index", "2");
    await showButton(page).tap();
    await expect.poll(() => shows.length).toBe(2);
    // Every part carries the passage's reference; the text is what moves on.
    const [first, second] = shows as Array<{ mainReference: string; mainText: string }>;
    expect(first.mainText).not.toBe(second.mainText);
    expect(first.mainReference).toMatch(/^Ján 3:16/);
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("Zhasnúť appears only while a verse is on the wall and clears it", async ({ browser, request }) => {
  const { context, page, errors } = await openConsole(browser, PHONE_390);
  try {
    const clears = collectPosts(page, "/bible/clear");
    const clear = page.locator('[data-role="console-clear-live"]');
    await expect(clear).toHaveCount(0);
    await lookUp(page, "zid 11 1", "Hebrejom 11:1 (SEB)");
    await showButton(page).tap();
    await expect(clear).toBeVisible();
    await assertMinTapTarget(clear, 44);
    await clear.tap();
    await expect.poll(() => clears.length).toBe(1);
    await expect(page.locator('[data-role="console-on-air"]')).toHaveText("Na stene: nič");
    await expect(clear).toHaveCount(0);
    const active = await (await request.get(new URL("/bible/active-slide", baseURL).toString())).json();
    expect(active).toBeNull();
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("a second tap while the show is on its way sends nothing more", async ({ browser }) => {
  const { context, page, errors } = await openConsole(browser, PHONE_390);
  try {
    const shows = collectPosts(page, "/bible/trigger-slide");
    let release: () => void = () => {};
    const held = new Promise<void>((resolve) => {
      release = resolve;
    });
    await page.route("**/bible/trigger-slide", async (route) => {
      await held;
      await route.continue();
    });
    await lookUp(page, "zid 11 1", "Hebrejom 11:1 (SEB)");
    await showButton(page).tap();
    // Busy, not `disabled`: a disabled button would lose the keyboard focus.
    await expect(showButton(page)).toHaveAttribute("aria-disabled", "true");
    await expect(showButton(page)).toHaveText("Zobrazujem…");
    await showButton(page).tap({ force: true });
    const answered = page.waitForResponse((res) => new URL(res.url()).pathname === "/bible/trigger-slide");
    release();
    await (await answered).finished();
    await expect(page.locator('[data-role="console-on-air-reference"]')).toHaveText("Hebrejom 11:1");
    expect(shows).toHaveLength(1);
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("alternatives and fixes: 'jan 3 16' offers 1 Ján, 'zid 14' offers Hebrejom 13", async ({ browser }) => {
  const { context, page, errors } = await openConsole(browser, PHONE_390);
  try {
    await lookUp(page, "jan 3 16", "Ján 3:16 (SEB)");
    const chip = page.locator('[data-role="console-alt-chip"][data-kind="alternative"]');
    await expect(chip).toHaveText("1 Ján");
    await assertMinTapTarget(chip, 44);
    await chip.tap();
    await expect(page.locator('[data-role="console-preview"]')).toHaveAttribute(
      "data-reference",
      "1 Ján 3:16 (SEB)",
    );
    await expect(chip).toHaveAttribute("aria-pressed", "true");

    const field = page.locator('[data-role="console-field"]');
    await field.fill("zid 14");
    await expect(page.locator('[data-role="console-hint"]')).toHaveText("Kniha má len 13 kapitol");
    await page.locator('[data-role="console-alt-chip"][data-kind="fix"]').tap();
    await expect(field).toHaveValue("Hebrejom 13");
    await expect(page.locator('[data-role="console-preview"]')).toHaveAttribute("data-ready", "true");
    await expect(page.locator('[data-role="console-preview"]')).toHaveAttribute(
      "data-reference",
      /^Hebrejom 13:/,
    );
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("two devices: a show or a clear from elsewhere reaches this phone's on-air strip", async ({
  browser,
  request,
}) => {
  const { context, page, errors } = await openConsole(browser, PHONE_390);
  try {
    await request.post(new URL("/bible/trigger-slide", baseURL).toString(), {
      data: { mainText: "Lebo Boh tak miloval svet", mainReference: "Ján 3:16 (SEB)" },
    });
    await expect(page.locator('[data-role="console-on-air-reference"]')).toHaveText("Ján 3:16");
    await request.post(new URL("/bible/clear", baseURL).toString(), { data: {} });
    await expect(page.locator('[data-role="console-on-air"]')).toHaveText("Na stene: nič");
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("a locked phone that missed the live event catches up when it is visible again", async ({
  browser,
  request,
}) => {
  const context = await browser.newContext({ ...PHONE_390, baseURL });
  const page = await context.newPage();
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  try {
    let deaf = false;
    await page.routeWebSocket(/\/live\/ws/, (ws) => {
      const server = ws.connectToServer();
      server.onMessage((message) => {
        if (deaf) return; // the suspended socket hears nothing
        ws.send(message);
      });
    });
    await page.goto("/ui/operator/bible");
    await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
    await expect(page.locator('[data-role="console-on-air"]')).toHaveText("Na stene: nič");
    await expect.poll(() => page.evaluate(() => (window as unknown as { __presenterLiveConnected: () => boolean }).__presenterLiveConnected())).toBe(true);

    deaf = true;
    await request.post(new URL("/bible/trigger-slide", baseURL).toString(), {
      data: { mainText: "Viera je podstatou", mainReference: "Hebrejom 11:1 (SEB)" },
    });
    await page.evaluate(() => {
      Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "hidden" });
      document.dispatchEvent(new Event("visibilitychange"));
    });
    await expect(page.locator('[data-role="console-on-air"]')).toHaveText("Na stene: nič");
    await page.evaluate(() => {
      Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "visible" });
      document.dispatchEvent(new Event("visibilitychange"));
    });
    await expect(page.locator('[data-role="console-on-air-reference"]')).toHaveText("Hebrejom 11:1");
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("a failed show says so inline and in a toast; nothing changes on the strip", async ({ browser }) => {
  const { context, page, errors } = await openConsole(browser, PHONE_390);
  try {
    await page.route("**/bible/trigger-slide", (route) =>
      route.fulfill({ status: 200, contentType: "application/json", body: "not-json" }),
    );
    await lookUp(page, "zid 11 1", "Hebrejom 11:1 (SEB)");
    await showButton(page).tap();
    await expect(page.locator('[data-role="console-show-error"]')).toContainText("Verš sa nezobrazil");
    await expect(page.locator('[data-role="toast"]')).toHaveAttribute("data-visible", "true");
    await expect(page.locator('[data-role="console-on-air"]')).toHaveAttribute("data-live", "false");
    await expect(showButton(page)).toBeEnabled();
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("while typing, the preview card carries its own ZOBRAZIŤ (the iPhone keyboard may hide the bar)", async ({
  browser,
}) => {
  const { context, page, errors } = await openConsole(browser, PHONE_390);
  try {
    const shows = collectPosts(page, "/bible/trigger-slide");
    const field = page.locator('[data-role="console-field"]');
    await field.tap();
    await field.pressSequentially("zid 11 1");
    await expect(page.locator('[data-role="console-preview"]')).toHaveAttribute("data-ready", "true");
    const inline = page.locator('[data-role="console-preview-show"]');
    await expect(inline).toBeVisible();
    await assertMinTapTarget(inline, 44);
    await inline.tap();
    await expect.poll(() => shows.length).toBe(1);
    expect(shows[0]).toMatchObject({ mainReference: "Hebrejom 11:1 (SEB)" });
    await expect(field).toBeFocused(); // the keyboard stays for the next reference
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("desktop: type → Enter → Enter shows the verse", async ({ browser }) => {
  const { context, page, errors } = await openConsole(browser, { viewport: { width: 1280, height: 800 } });
  try {
    const shows = collectPosts(page, "/bible/trigger-slide");
    const field = page.locator('[data-role="console-field"]');
    await field.click();
    await field.pressSequentially("zid 11 1");
    await field.press("Enter");
    await expect(showButton(page)).toBeFocused();
    await page.keyboard.press("Enter");
    await expect.poll(() => shows.length).toBe(1);
    expect(shows[0]).toMatchObject({ mainReference: "Hebrejom 11:1 (SEB)" });
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("desktop: Enter on an empty field leaves the typing after it alone", async ({ browser }) => {
  const { context, page, errors } = await openConsole(browser, { viewport: { width: 1280, height: 800 } });
  try {
    const field = page.locator('[data-role="console-field"]');
    await field.click();
    await field.press("Enter");
    // Slow enough for "zid 1" to load on the way: Hebrejom 1 must not take the focus.
    await field.pressSequentially("zid 11:1", { delay: 250 });
    await expect(field).toBeFocused();
    await expect(field).toHaveValue("zid 11:1");
    await expect(consoleRoot(page)).toHaveAttribute("data-preview-reference", "Hebrejom 11:1 (SEB)");
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("desktop: a translation switch keeps the console's verse over a book in the old inputs", async ({
  browser,
}) => {
  const { context, page, errors } = await openConsole(browser, { viewport: { width: 1280, height: 800 } });
  try {
    await page.locator('[data-role="book-filter"]').fill("Rimanom");
    await page.locator('[data-role="book-item"]').first().click();
    await page.locator('[data-role="load-button"]').click();
    await expect(page.locator('[data-role="slide-card"]').first()).toContainText("Rimanom 1:1-32 (SEB)");

    await lookUp(page, "zid 11 1", "Hebrejom 11:1 (SEB)");
    await page.locator('[data-role="main-translation"]').selectOption("slk-roh");
    await expect(page.locator('[data-role="book-list"]')).toHaveAttribute(
      "data-books-translation",
      "slk-roh",
    );
    await expect(consoleRoot(page)).toHaveAttribute("data-preview-reference", "Židom 11:1 (ROH)");
    // Longer than the old inputs' 300 ms auto-load, which must not fire.
    await page.waitForTimeout(600);
    await expect(page.locator('[data-role="console-field"]')).toHaveValue("zid 11 1");
    await expect(consoleRoot(page)).toHaveAttribute("data-preview-reference", "Židom 11:1 (ROH)");
    await expect(page.locator('[data-role="slide-card"]').first()).toContainText("Židom 11:1");
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("a tapped alternative ends with its input: the same text typed again is the first row", async ({
  browser,
}) => {
  const { context, page, errors } = await openConsole(browser, PHONE_390);
  try {
    await lookUp(page, "jan 3 16", "Ján 3:16 (SEB)");
    await page.locator('[data-role="console-alt-chip"][data-kind="alternative"]').tap();
    await expect(consoleRoot(page)).toHaveAttribute("data-preview-reference", "1 Ján 3:16 (SEB)");
    await lookUp(page, "zid 11 1", "Hebrejom 11:1 (SEB)");
    await lookUp(page, "jan 3 16", "Ján 3:16 (SEB)");
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("a split verse: Zhasnúť appears away from where ZOBRAZIŤ was", async ({
  browser,
}) => {
  const { context, page, errors } = await openConsole(browser, { ...PHONE_390, viewport: { width: 390, height: 844 } });
  try {
    await page.locator('[data-role="console-field"]').fill("zid 11 1-3");
    await expect(page.locator('[data-role="console-preview"]')).toHaveAttribute("data-ready", "true");
    await page.locator('[data-role="console-field"]').blur();
    const before = await showButton(page).boundingBox();
    await showButton(page).tap();
    const clear = page.locator('[data-role="console-clear-live"]');
    await expect(clear).toBeVisible();
    const clearBox = await clear.boundingBox();
    // A second tap where ZOBRAZIŤ was can never hit Zhasnúť.
    expect((clearBox?.y ?? 0) + (clearBox?.height ?? 0)).toBeLessThan((before?.y ?? 0) - 48);
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("a visible phone whose socket dropped reads the wall again on reconnect", async ({ browser, request }) => {
  const context = await browser.newContext({ ...PHONE_390, baseURL });
  const page = await context.newPage();
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  try {
    const sockets: WebSocketRoute[] = [];
    let dying: WebSocketRoute | null = null;
    await page.routeWebSocket(/\/live\/ws/, (ws: WebSocketRoute) => {
      sockets.push(ws);
      const server = ws.connectToServer();
      server.onMessage((message) => {
        if (ws === dying) return; // what the hub publishes now is lost
        ws.send(message);
      });
    });
    await page.goto("/ui/operator/bible");
    await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
    await expect(page.locator('[data-role="console-on-air"]')).toHaveText("Na stene: nič");
    await expect.poll(() => sockets.length).toBeGreaterThan(0);
    await expect.poll(() => page.evaluate(() => (window as unknown as { __presenterLiveConnected: () => boolean }).__presenterLiveConnected())).toBe(true);

    dying = sockets[sockets.length - 1];
    await request.post(new URL("/bible/trigger-slide", baseURL).toString(), {
      data: { mainText: "Viera je podstatou", mainReference: "Hebrejom 11:1 (SEB)" },
    });
    await dying.close({ code: 4000, reason: "reset" });
    await expect(page.locator('[data-role="console-on-air-reference"]')).toHaveText("Hebrejom 11:1", {
      timeout: 20_000,
    });
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("desktop: Space on a focused ZOBRAZIŤ shows the verse", async ({ browser }) => {
  const { context, page, errors } = await openConsole(browser, { viewport: { width: 1280, height: 800 } });
  try {
    const shows = collectPosts(page, "/bible/trigger-slide");
    const field = page.locator('[data-role="console-field"]');
    await field.click();
    await field.pressSequentially("zid 11 1");
    await field.press("Enter");
    await expect(showButton(page)).toBeFocused();
    await page.keyboard.press("Space");
    await expect.poll(() => shows.length).toBe(1);
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("a failed show's message leaves with the verse it was about", async ({ browser }) => {
  const { context, page, errors } = await openConsole(browser, PHONE_390);
  try {
    await page.route("**/bible/trigger-slide", (route) =>
      route.fulfill({ status: 200, contentType: "application/json", body: "not-json" }),
    );
    await lookUp(page, "zid 11 1", "Hebrejom 11:1 (SEB)");
    await showButton(page).tap();
    await expect(page.locator('[data-role="console-show-error"]')).toBeVisible();
    await page.locator('[data-role="console-clear-field"]').tap();
    await expect(page.locator('[data-role="console-show-error"]')).toHaveCount(0);
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("a slow re-read of the wall never replaces a newer live event", async ({ browser, request }) => {
  const { context, page, errors } = await openConsole(browser, PHONE_390);
  try {
    await expect(page.locator('[data-role="console-on-air"]')).toHaveText("Na stene: nič");
    let release: () => void = () => {};
    const held = new Promise<void>((resolve) => {
      release = resolve;
    });
    let asked = false;
    await page.route("**/bible/active-slide", async (route) => {
      asked = true;
      await held;
      await route.fulfill({ status: 200, contentType: "application/json", body: "null" });
    });
    await page.evaluate(() => document.dispatchEvent(new Event("visibilitychange")));
    await expect.poll(() => asked).toBe(true);
    await request.post(new URL("/bible/trigger-slide", baseURL).toString(), {
      data: { mainText: "Viera je podstatou", mainReference: "Hebrejom 11:1 (SEB)" },
    });
    await expect(page.locator('[data-role="console-on-air-reference"]')).toHaveText("Hebrejom 11:1");
    const answered = page.waitForResponse("**/bible/active-slide");
    release();
    await answered;
    await page.waitForTimeout(100);
    await expect(page.locator('[data-role="console-on-air-reference"]')).toHaveText("Hebrejom 11:1");
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("desktop: the keyboard steps a split passage, the focus stays on ZOBRAZIŤ", async ({ browser }) => {
  const { context, page, errors } = await openConsole(browser, { viewport: { width: 1280, height: 800 } });
  try {
    const shows = collectPosts(page, "/bible/trigger-slide");
    const field = page.locator('[data-role="console-field"]');
    await field.click();
    await field.pressSequentially("zid 11 1-20");
    await expect(page.locator('[data-role="console-preview"]')).toHaveAttribute("data-ready", "true");
    const count = Number(await page.locator('[data-role="console-preview"]').getAttribute("data-slide-count"));
    expect(count).toBeGreaterThan(3);
    await field.press("Enter");
    await expect(showButton(page)).toBeFocused();
    await page.keyboard.press("Space");
    await expect(showButton(page)).toHaveText(`ĎALŠÍ: 11:1-20 2/${count}`);
    await expect(showButton(page)).toBeFocused();
    await page.keyboard.press("Space");
    await expect(showButton(page)).toHaveText(`ĎALŠÍ: 11:1-20 3/${count}`);
    await expect(showButton(page)).toBeFocused();
    await page.keyboard.press("Enter");
    await expect(showButton(page)).toHaveText(`ĎALŠÍ: 11:1-20 4/${count}`);
    expect(shows).toHaveLength(3);
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("a show that fails after its verse left says so in a toast, not on the empty console", async ({ browser }) => {
  const { context, page, errors } = await openConsole(browser, PHONE_390);
  try {
    let release: () => void = () => {};
    const held = new Promise<void>((resolve) => {
      release = resolve;
    });
    await page.route("**/bible/trigger-slide", async (route) => {
      await held;
      await route.fulfill({ status: 200, contentType: "application/json", body: "not-json" });
    });
    await lookUp(page, "zid 11 1", "Hebrejom 11:1 (SEB)");
    await showButton(page).tap();
    await expect(showButton(page)).toHaveText("Zobrazujem…");
    await page.locator('[data-role="console-clear-field"]').tap();
    await expect(consoleRoot(page)).toHaveAttribute("data-preview-state", "empty");
    release();
    const toast = page.locator('[data-role="toast"]');
    await expect(toast).toHaveAttribute("data-visible", "true");
    // Above the bar, whose buttons it would hide (#875).
    const [toastBox, barBox] = await Promise.all([
      toast.boundingBox(),
      page.locator('[data-role="console-bar"]').boundingBox(),
    ]);
    expect(toastBox!.y + toastBox!.height).toBeLessThanOrEqual(barBox!.y);
    await expect(page.locator('[data-role="console-show-error"]')).toHaveCount(0);
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("phone: the first verse on the wall moves neither the field nor ZOBRAZIŤ", async ({ browser }) => {
  const { context, page, errors } = await openConsole(browser, PHONE_360);
  try {
    await page.locator('[data-role="secondary-translation"]').selectOption("eng-kjv");
    const field = page.locator('[data-role="console-field"]');
    await field.fill("1 tes 5 16-18");
    await expect(consoleRoot(page)).toHaveAttribute("data-preview-ready", "true");
    await field.blur();
    const fieldBefore = await field.boundingBox();
    const showBefore = await showButton(page).boundingBox();
    await showButton(page).tap();
    await expect(page.locator('[data-role="console-clear-live"]')).toBeVisible();
    await expect(page.locator('[data-role="console-on-air-codes"]')).toContainText("+");
    expect((await field.boundingBox())?.y).toBe(fieldBefore?.y);
    expect((await showButton(page).boundingBox())?.y).toBe(showBefore?.y);
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("a verse sent past the console, by Companion or the AI, is on the strip with Zhasnúť", async ({
  browser,
  request,
}) => {
  const { context, page, errors } = await openConsole(browser, PHONE_390);
  try {
    const trigger = new URL("/bible/trigger", baseURL).toString();
    const first = { translation: "slk-seb", book: "Hebrejom", chapter: 11, verseStart: 1 };
    expect((await request.post(trigger, { data: first })).ok()).toBeTruthy();
    await expect(page.locator('[data-role="console-on-air-reference"]')).toHaveText("Hebrejom 11:1");
    await expect(page.locator('[data-role="console-clear-live"]')).toBeVisible();

    // The next one replaces it, also over a verse the console itself showed.
    await lookUp(page, "zid 11 1", "Hebrejom 11:1 (SEB)");
    const shown = page.waitForResponse("**/bible/trigger-slide");
    await showButton(page).tap();
    await shown;
    const next = { translation: "slk-seb", book: "Hebrejom", chapter: 11, verseStart: 3 };
    expect((await request.post(trigger, { data: next })).ok()).toBeTruthy();
    await expect(page.locator('[data-role="console-on-air-reference"]')).toHaveText("Hebrejom 11:3");

    await page.reload();
    await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
    await expect(page.locator('[data-role="console-on-air-reference"]')).toHaveText("Hebrejom 11:3");
    await page.locator('[data-role="console-clear-live"]').tap();
    await expect(page.locator('[data-role="console-on-air"]')).toHaveText("Na stene: nič");
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("desktop: a held Enter stops at ZOBRAZIŤ, a held key never steps a split passage", async ({ browser }) => {
  const { context, page, errors } = await openConsole(browser, { viewport: { width: 1280, height: 800 } });
  try {
    const shows = collectPosts(page, "/bible/trigger-slide");
    const field = page.locator('[data-role="console-field"]');
    await field.click();
    await field.pressSequentially("zid 11 1-20");
    await expect(page.locator('[data-role="console-preview"]')).toHaveAttribute("data-ready", "true");
    await page.keyboard.down("Enter");
    await expect(showButton(page)).toBeFocused();
    await page.keyboard.down("Enter"); // a second down without an up is the auto-repeat
    await page.keyboard.down("Enter");
    await page.keyboard.up("Enter");
    await expect(showButton(page)).toBeFocused();
    expect(shows).toHaveLength(0);

    await page.keyboard.press("Enter");
    await expect.poll(() => shows.length).toBe(1);

    // Only the keys that send are held back: a held Tab walks past the button.
    await field.focus();
    for (let press = 0; press < 8; press += 1) {
      await page.keyboard.down("Tab");
    }
    await page.keyboard.up("Tab");
    await expect(showButton(page)).not.toBeFocused();
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("a show answered after a newer verse never puts the older one back on the strip", async ({
  browser,
  request,
}) => {
  const { context, page, errors } = await openConsole(browser, PHONE_390);
  try {
    let release: () => void = () => {};
    const held = new Promise<void>((resolve) => {
      release = resolve;
    });
    let reachedServer = false;
    await page.route("**/bible/trigger-slide", async (route) => {
      const response = await route.fetch();
      reachedServer = true;
      await held;
      await route.fulfill({ response });
    });
    await lookUp(page, "zid 11 1", "Hebrejom 11:1 (SEB)");
    await showButton(page).tap();
    await expect.poll(() => reachedServer).toBe(true);
    const next = { translation: "slk-seb", book: "Hebrejom", chapter: 11, verseStart: 3 };
    expect((await request.post(new URL("/bible/trigger", baseURL).toString(), { data: next })).ok()).toBeTruthy();
    const reference = page.locator('[data-role="console-on-air-reference"]');
    await expect(reference).toHaveText("Hebrejom 11:3");
    const answered = page.waitForResponse("**/bible/trigger-slide");
    release();
    await answered;
    await expect(showButton(page)).not.toHaveText("Zobrazujem…");
    await expect(reference).toHaveText("Hebrejom 11:3");
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("a clear that overtakes the show's answer leaves the strip empty", async ({ browser, request }) => {
  const { context, page, errors } = await openConsole(browser, PHONE_390);
  try {
    let release: () => void = () => {};
    const held = new Promise<void>((resolve) => {
      release = resolve;
    });
    let reachedServer = false;
    await page.route("**/bible/trigger-slide", async (route) => {
      const response = await route.fetch();
      reachedServer = true;
      await held;
      await route.fulfill({ response });
    });
    await lookUp(page, "zid 11 1", "Hebrejom 11:1 (SEB)");
    await showButton(page).tap();
    await expect.poll(() => reachedServer).toBe(true);
    expect((await request.post(new URL("/bible/clear", baseURL).toString(), { data: {} })).ok()).toBeTruthy();
    const onAir = page.locator('[data-role="console-on-air"]');
    await expect(onAir).toHaveText("Na stene: nič");
    const reread = page.waitForResponse("**/bible/active-slide");
    release();
    await reread;
    await expect(showButton(page)).not.toHaveText("Zobrazujem…");
    await expect(onAir).toHaveAttribute("data-live", "false");
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("a show with a deaf socket and a failing re-read still reaches the strip", async ({ browser }) => {
  const context = await browser.newContext({ ...PHONE_390, baseURL });
  const page = await context.newPage();
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  try {
    let deaf = false;
    await page.routeWebSocket(/\/live\/ws/, (ws) => {
      const server = ws.connectToServer();
      server.onMessage((message) => {
        if (deaf) return;
        ws.send(message);
      });
    });
    await page.goto("/ui/operator/bible");
    await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
    await expect(page.locator('[data-role="console-on-air"]')).toHaveText("Na stene: nič");
    await lookUp(page, "zid 11 1", "Hebrejom 11:1 (SEB)");

    deaf = true;
    await page.route("**/bible/active-slide", (route) =>
      route.fulfill({ status: 200, contentType: "application/json", body: "not-json" }),
    );
    await showButton(page).tap();
    await expect(page.locator('[data-role="console-on-air-reference"]')).toHaveText("Hebrejom 11:1");
    await expect(page.locator('[data-role="console-clear-live"]')).toBeVisible();
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("ĎALŠÍ: after a show the field clears and one tap shows the next verse (S6)", async ({ browser }) => {
  const { context, page, errors } = await openConsole(browser, PHONE_390);
  try {
    const shows = collectPosts(page, "/bible/trigger-slide");
    const preview = page.locator('[data-role="console-preview"]');
    await lookUp(page, "zid 11 1", "Hebrejom 11:1 (SEB)");
    await showButton(page).tap();
    await expect(page.locator('[data-role="console-field"]')).toHaveValue("");
    await expect(showButton(page)).toHaveText("ĎALŠÍ: 11:2");
    await expect(preview).toHaveAttribute("data-reference", "Hebrejom 11:2 (SEB)");
    await expect(preview).toHaveAttribute("data-ready", "true");
    await showButton(page).tap(); // next verse = 1 tap
    await expect.poll(() => shows.length).toBe(2);
    expect(shows[1]).toMatchObject({ mainReference: "Hebrejom 11:2 (SEB)" });
    await expect(showButton(page)).toHaveText("ĎALŠÍ: 11:3");
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("◀ / ▶ move only the preview, across a chapter end, never past the book", async ({ browser }) => {
  const { context, page, errors } = await openConsole(browser, PHONE_390);
  try {
    const shows = collectPosts(page, "/bible/trigger-slide");
    const preview = page.locator('[data-role="console-preview"]');
    const prev = page.locator('[data-role="console-prev"]');
    const next = page.locator('[data-role="console-next"]');
    await lookUp(page, "zid 11 1", "Hebrejom 11:1 (SEB)");
    await expect(prev).toHaveText("◀ 10:39");
    await expect(next).toHaveText("11:2 ▶");
    await assertMinTapTarget(page.locator('[data-role="console-bar"] button'), 44);
    await next.tap();
    await expect(preview).toHaveAttribute("data-reference", "Hebrejom 11:2 (SEB)");
    await prev.tap();
    await expect(preview).toHaveAttribute("data-reference", "Hebrejom 11:1 (SEB)");
    await prev.tap();
    await expect(preview).toHaveAttribute("data-reference", "Hebrejom 10:39 (SEB)");

    await lookUp(page, "zid 13 25", "Hebrejom 13:25 (SEB)");
    await expect(next).toBeDisabled();
    expect(shows).toHaveLength(0);
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("a verse list is walked verse by verse, then the verse after it", async ({ browser }) => {
  const { context, page, errors } = await openConsole(browser, PHONE_390);
  try {
    await lookUp(page, "mt 5:3.5.7", "Matúš 5:3 (SEB)");
    for (const nextLabel of ["ĎALŠÍ: 5:5", "ĎALŠÍ: 5:7", "ĎALŠÍ: 5:8"]) {
      await expect(page.locator('[data-role="console-preview"]')).toHaveAttribute("data-ready", "true");
      await showButton(page).tap();
      await expect(showButton(page)).toHaveText(nextLabel);
    }
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("while typing on a phone the tab bar steps aside for the keyboard", async ({ browser }) => {
  const { context, page, errors } = await openConsole(browser, PHONE_390);
  try {
    const tabs = page.locator('[data-role="phone-tab-bar"]');
    await expect(tabs).toBeVisible();
    await page.locator('[data-role="console-field"]').tap();
    await expect(tabs).toBeHidden();
    await page.locator('[data-role="console-field"]').press("Enter"); // "Go" closes the keyboard
    await expect(tabs).toBeVisible();
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("desktop: Enter, Enter, Space walks verse by verse, the focus stays on the button", async ({ browser }) => {
  const { context, page, errors } = await openConsole(browser, { viewport: { width: 1280, height: 800 } });
  try {
    const shows = collectPosts(page, "/bible/trigger-slide");
    const field = page.locator('[data-role="console-field"]');
    await field.click();
    await field.pressSequentially("zid 11 1");
    await expect(page.locator('[data-role="console-preview"]')).toHaveAttribute("data-ready", "true");
    await field.press("Enter");
    await expect(showButton(page)).toBeFocused();
    await page.keyboard.press("Enter");
    await expect(showButton(page)).toHaveText("ĎALŠÍ: 11:2");
    await expect(consoleRoot(page)).toHaveAttribute("data-preview-reference", "Hebrejom 11:2 (SEB)");
    await expect(showButton(page)).toBeFocused();
    await page.keyboard.press("Space");
    await expect(showButton(page)).toHaveText("ĎALŠÍ: 11:3");
    await expect(showButton(page)).toBeFocused();
    expect(shows).toMatchObject([
      { mainReference: "Hebrejom 11:1 (SEB)" },
      { mainReference: "Hebrejom 11:2 (SEB)" },
    ]);
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("desktop: a load from the old inputs after a show lets the next verse go", async ({ browser }) => {
  const { context, page, errors } = await openConsole(browser, { viewport: { width: 1280, height: 800 } });
  try {
    await lookUp(page, "zid 11 1", "Hebrejom 11:1 (SEB)");
    await showButton(page).click();
    await expect(showButton(page)).toHaveText("ĎALŠÍ: 11:2");
    await page.locator('[data-role="book-filter"]').fill("Rimanom");
    await page.locator('[data-role="book-item"]').first().click();
    await page.locator('[data-role="load-button"]').click();
    await expect(page.locator('[data-role="slide-card"]').first()).toContainText("Rimanom 1:1-32 (SEB)");
    await expect(consoleRoot(page)).toHaveAttribute("data-preview-state", "empty");
    await expect(showButton(page)).toHaveText("ZOBRAZIŤ");
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("▶ during a slow show moves nothing: the next part is never skipped", async ({ browser }) => {
  const { context, page, errors } = await openConsole(browser, PHONE_390);
  try {
    let release: () => void = () => {};
    const held = new Promise<void>((resolve) => {
      release = resolve;
    });
    await page.route("**/bible/trigger-slide", async (route) => {
      await held;
      await route.continue();
    });
    await page.locator('[data-role="console-field"]').fill("jan 3 16-21");
    const preview = page.locator('[data-role="console-preview"]');
    await expect(preview).toHaveAttribute("data-ready", "true");
    await showButton(page).tap();
    await expect(showButton(page)).toHaveText("Zobrazujem…");
    await page.locator('[data-role="console-next"]').tap({ force: true });
    await expect(preview).toHaveAttribute("data-slide-index", "1");
    const answered = page.waitForResponse((res) => new URL(res.url()).pathname === "/bible/trigger-slide");
    release();
    await (await answered).finished();
    await expect(preview).toHaveAttribute("data-slide-index", "2");
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("a translation switch keeps the stepped verse and the next verse after a show", async ({ browser }) => {
  const { context, page, errors } = await openConsole(browser, PHONE_390);
  try {
    const shows = collectPosts(page, "/bible/trigger-slide");
    await lookUp(page, "zid 11 1", "Hebrejom 11:1 (SEB)");
    await page.locator('[data-role="console-next"]').tap();
    await expect(consoleRoot(page)).toHaveAttribute("data-preview-reference", "Hebrejom 11:2 (SEB)");
    await page.locator('[data-role="main-translation"]').selectOption("slk-roh");
    await expect(consoleRoot(page)).toHaveAttribute("data-preview-reference", "Židom 11:2 (ROH)");

    await showButton(page).tap();
    await expect(showButton(page)).toHaveText("ĎALŠÍ: 11:3");
    await page.locator('[data-role="main-translation"]').selectOption("slk-seb");
    await expect(consoleRoot(page)).toHaveAttribute("data-preview-reference", "Hebrejom 11:3 (SEB)");
    await expect(showButton(page)).toHaveText("ĎALŠÍ: 11:3");
    await expect(consoleRoot(page)).toHaveAttribute("data-preview-ready", "true");
    await showButton(page).tap();
    await expect.poll(() => shows.length).toBe(2);
    expect(shows[1]).toMatchObject({ mainReference: "Hebrejom 11:3 (SEB)" });
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("a next verse that did not load: the primary button tries again, one more tap shows it", async ({
  browser,
}) => {
  const { context, page, errors } = await openConsole(browser, PHONE_390);
  try {
    const shows = collectPosts(page, "/bible/trigger-slide");
    await lookUp(page, "zid 11 1", "Hebrejom 11:1 (SEB)");
    await page.route("**/bible/resolve", (route) =>
      route.fulfill({ status: 200, contentType: "application/json", body: "not-json" }),
    );
    await showButton(page).tap();
    await expect(consoleRoot(page)).toHaveAttribute("data-preview-state", "error");
    await expect(showButton(page)).toHaveText("SKÚSIŤ ZNOVA");
    await page.unroute("**/bible/resolve");
    await showButton(page).tap();
    await expect(consoleRoot(page)).toHaveAttribute("data-preview-reference", "Hebrejom 11:2 (SEB)");
    expect(shows).toHaveLength(1);
    await expect(showButton(page)).toHaveText("ĎALŠÍ: 11:2");
    await showButton(page).tap();
    await expect.poll(() => shows.length).toBe(2);
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("the end of the book says so and sends the last verse only once", async ({ browser }) => {
  const { context, page, errors } = await openConsole(browser, PHONE_390);
  try {
    const shows = collectPosts(page, "/bible/trigger-slide");
    await lookUp(page, "zid 13 25", "Hebrejom 13:25 (SEB)");
    await showButton(page).tap();
    await expect(showButton(page)).toHaveText("KONIEC KNIHY");
    await expect(showButton(page)).toHaveAttribute("aria-disabled", "true");
    await showButton(page).tap({ force: true });
    await page.locator('[data-role="console-prev"]').tap();
    await expect(showButton(page)).toHaveText("ĎALŠÍ: 13:24");
    expect(shows).toHaveLength(1);
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("desktop: ▶ before a show keeps the focus on ZOBRAZIŤ for the next Space", async ({ browser }) => {
  const { context, page, errors } = await openConsole(browser, { viewport: { width: 1280, height: 800 } });
  try {
    const shows = collectPosts(page, "/bible/trigger-slide");
    const field = page.locator('[data-role="console-field"]');
    await field.click();
    await field.pressSequentially("zid 11 1");
    await expect(page.locator('[data-role="console-preview"]')).toHaveAttribute("data-ready", "true");
    await field.press("Enter");
    await expect(showButton(page)).toBeFocused();
    await page.locator('[data-role="console-next"]').click();
    await expect(consoleRoot(page)).toHaveAttribute("data-preview-reference", "Hebrejom 11:2 (SEB)");
    await expect(consoleRoot(page)).toHaveAttribute("data-preview-ready", "true");
    await expect(showButton(page)).toBeFocused();
    await page.keyboard.press("Space");
    await expect.poll(() => shows.length).toBe(1);
    expect(shows[0]).toMatchObject({ mainReference: "Hebrejom 11:2 (SEB)" });
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("a verse typed after the end of a book shows as usual", async ({ browser }) => {
  const { context, page, errors } = await openConsole(browser, PHONE_390);
  try {
    const shows = collectPosts(page, "/bible/trigger-slide");
    await lookUp(page, "zid 13 25", "Hebrejom 13:25 (SEB)");
    await showButton(page).tap();
    await expect(showButton(page)).toHaveText("KONIEC KNIHY");
    await lookUp(page, "zid 11 1", "Hebrejom 11:1 (SEB)");
    await expect(showButton(page)).toHaveText("ZOBRAZIŤ");
    await showButton(page).tap();
    await expect.poll(() => shows.length).toBe(2);
    expect(shows[1]).toMatchObject({ mainReference: "Hebrejom 11:1 (SEB)" });
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("desktop: a load from the old inputs after the end of a book frees the button", async ({ browser }) => {
  const { context, page, errors } = await openConsole(browser, { viewport: { width: 1280, height: 800 } });
  try {
    await lookUp(page, "zid 13 25", "Hebrejom 13:25 (SEB)");
    await showButton(page).click();
    await expect(showButton(page)).toHaveText("KONIEC KNIHY");
    await page.locator('[data-role="book-filter"]').fill("Rimanom");
    await page.locator('[data-role="book-item"]').first().click();
    await page.locator('[data-role="load-button"]').click();
    await expect(page.locator('[data-role="slide-card"]').first()).toContainText("Rimanom 1:1-32 (SEB)");
    await expect(showButton(page)).toHaveText("ZOBRAZIŤ");
    await lookUp(page, "zid 11 1", "Hebrejom 11:1 (SEB)");
    await expect(showButton(page)).toHaveAttribute("aria-disabled", "false");
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});
