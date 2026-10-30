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

function collectPosts(page: Page, path: string): unknown[] {
  const bodies: unknown[] = [];
  page.on("request", (req: Request) => {
    if (req.method() === "POST" && new URL(req.url()).pathname === path) {
      bodies.push(req.postDataJSON());
    }
  });
  return bodies;
}

test("phone: 'zid 11 1' resolves to Hebrejom 11:1, loads while typing, and its card goes to the wall", async ({
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
    // The old inputs are gone on a phone; the console is the Live tab.
    await expect(page.locator('[data-role="book-filter"]')).toBeHidden();

    const card = page.locator('[data-role="slide-card"]').first();
    await expect(card).toContainText("Hebrejom 11:1 (SEB)");
    await card.locator('[data-role="slide-trigger-zone"]').tap();
    await expect.poll(() => shows.length).toBe(1);
    expect(shows[0]).toMatchObject({ mainReference: "Hebrejom 11:1 (SEB)" });
    const active = await (await request.get(new URL("/bible/active-slide", baseURL).toString())).json();
    expect(active.mainReference).toBe("Hebrejom 11:1 (SEB)");
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
